//! Adapter from one durable [`ExecutionAttempt`](nomifun_api_types::ExecutionAttempt)
//! to one real Agent conversation.
//!
//! This module deliberately knows nothing about planning, DAG scheduling or
//! execution lifecycle. It creates a conversation, requires the caller to
//! persist the attempt's `ConversationExecutionLink`, executes one turn, and returns the
//! observed output. The scheduler is therefore able to cancel an attempt as
//! soon as the conversation exists, without a correlation-id race.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use nomifun_ai_agent::AgentRuntimeRegistry;
use nomifun_api_types::{
    CreateConversationRequest, ExecutionModelPool, ExecutionModelRef, ExecutionParticipant,
    ListMessagesQuery, SendMessageRequest,
};
use nomifun_common::{
    AgentToolPolicy, AgentType, AppError, DecisionPolicy, DelegationPolicy, ProviderId,
    ProviderWithModel,
    MAX_AGENT_DELEGATION_DEPTH,
};
use nomifun_conversation::{AgentExecutionConversationPort, ConversationService};
use serde_json::{Value, json};

/// Async callback invoked immediately after the Agent conversation is created
/// and before its first message is sent. The scheduler uses it to persist the
/// attempt link and make cancellation/recovery race-free.
pub(crate) type AttemptStarted = Box<
    dyn FnOnce(String) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send>> + Send,
>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttemptOutcome {
    pub conversation_id: String,
    pub text: Option<String>,
    pub ok: bool,
    pub tokens: Option<i64>,
}

#[async_trait]
pub(crate) trait AttemptRunner: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    async fn execute(
        &self,
        owner_id: &str,
        participant: &ExecutionParticipant,
        execution_model_pool: &[ExecutionModelRef],
        workspace_dir: Option<&str>,
        step_title: &str,
        tool_policy: AgentToolPolicy,
        delegation_policy: DelegationPolicy,
        delegation_depth: i64,
        decision_policy: DecisionPolicy,
        attempt_creation_key: &str,
        brief: &str,
        step_spec: &str,
        timeout: Duration,
        on_started: AttemptStarted,
    ) -> Result<AttemptOutcome, AppError>;

    /// Continue a waiting attempt in its existing Agent conversation after a
    /// user decision. The same durable attempt and transcript remain attached.
    async fn continue_with_input(
        &self,
        _owner_id: &str,
        _conversation_id: &str,
        _operation_id: &str,
        _input: &str,
        _timeout: Duration,
    ) -> Result<AttemptOutcome, AppError> {
        Err(AppError::BadRequest(
            "this attempt runner cannot continue an existing attempt".to_owned(),
        ))
    }

    /// Best-effort is insufficient here: a queued attempt recovered after a
    /// process crash must remove any creation-keyed conversation that never
    /// acquired its durable Execution link.  Implementations may no-op only
    /// when they cannot create external conversation state.
    async fn discard_unlinked_creation(
        &self,
        _owner_id: &str,
        _attempt_creation_key: &str,
    ) -> Result<(), AppError> {
        Ok(())
    }

    async fn read_final_output(&self, _owner_id: &str, _conversation_id: &str) -> Option<String> {
        None
    }

    async fn last_error_retryable(&self, _owner_id: &str, _conversation_id: &str) -> bool {
        false
    }

    async fn last_error_present(&self, _owner_id: &str, _conversation_id: &str) -> bool {
        false
    }

    async fn last_error_summary(&self, _owner_id: &str, _conversation_id: &str) -> Option<String> {
        None
    }
}

/// Production adapter. `ConversationService` owns the real Agent runtime; this
/// type only performs the create/send/wait/read choreography for one attempt.
pub(crate) struct ConversationAttemptRunner {
    conv: ConversationService,
    execution_port: AgentExecutionConversationPort,
}

impl ConversationAttemptRunner {
    pub fn new(conv: ConversationService, runtime_registry: Arc<dyn AgentRuntimeRegistry>) -> Self {
        let execution_port = conv.agent_execution_port(runtime_registry);
        Self {
            conv,
            execution_port,
        }
    }

    async fn await_turn(&self, conversation_id: &str, timeout: Duration, poll: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if !self.conv.runtime_summary_for(conversation_id).await.is_processing {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(poll).await;
        }
    }

    async fn recent_messages(&self, owner_id: &str, conversation_id: &str) -> Option<Value> {
        let messages = self
            .conv
            .list_messages(
                owner_id,
                conversation_id,
                ListMessagesQuery {
                    page: Some(1),
                    page_size: Some(10),
                    order: Some("desc".to_owned()),
                    content_mode: None,
                    cursor: None,
                },
            )
            .await
            .ok()?;
        serde_json::to_value(messages).ok()
    }

    #[allow(clippy::too_many_arguments)]
    async fn deliver_turn(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        content: &str,
        origin: &str,
        timeout: Duration,
    ) -> Result<AttemptOutcome, AppError> {
        let delivery = self
            .execution_port
            .deliver_turn(
                owner_id,
                conversation_id,
                operation_id,
                SendMessageRequest {
                    content: content.to_owned(),
                    files: vec![],
                    inject_skills: vec![],
                    hidden: false,
                    origin: Some(origin.to_owned()),
                    channel_platform: None,
                },
            )
            .await?;
        if delivery.completed {
            return Ok(AttemptOutcome {
                conversation_id: conversation_id.to_owned(),
                text: delivery.result_text,
                ok: delivery.result_ok.unwrap_or(false),
                tokens: self.conv.take_turn_tokens(conversation_id),
            });
        }
        if !self
            .await_turn(conversation_id, timeout, Duration::from_millis(500))
            .await
        {
            return Ok(AttemptOutcome {
                conversation_id: conversation_id.to_owned(),
                text: None,
                ok: false,
                tokens: self.conv.take_turn_tokens(conversation_id),
            });
        }
        let _ = self
            .await_turn(
                conversation_id,
                Duration::from_secs(5),
                Duration::from_millis(25),
            )
            .await;
        if let Some(receipt) = self
            .execution_port
            .delivery_result(owner_id, conversation_id, operation_id)
            .await?
            .filter(|receipt| receipt.completed)
        {
            return Ok(AttemptOutcome {
                conversation_id: conversation_id.to_owned(),
                text: receipt.result_text,
                ok: receipt.result_ok.unwrap_or(false),
                tokens: self.conv.take_turn_tokens(conversation_id),
            });
        }
        let text = self
            .recent_messages(owner_id, conversation_id)
            .await
            .as_ref()
            .and_then(latest_assistant_text);
        Ok(AttemptOutcome {
            conversation_id: conversation_id.to_owned(),
            ok: text.is_some(),
            text,
            tokens: self.conv.take_turn_tokens(conversation_id),
        })
    }
}

#[async_trait]
impl AttemptRunner for ConversationAttemptRunner {
    #[allow(clippy::too_many_arguments)]
    async fn execute(
        &self,
        owner_id: &str,
        participant: &ExecutionParticipant,
        execution_model_pool: &[ExecutionModelRef],
        workspace_dir: Option<&str>,
        step_title: &str,
        tool_policy: AgentToolPolicy,
        delegation_policy: DelegationPolicy,
        delegation_depth: i64,
        decision_policy: DecisionPolicy,
        attempt_creation_key: &str,
        brief: &str,
        step_spec: &str,
        timeout: Duration,
        on_started: AttemptStarted,
    ) -> Result<AttemptOutcome, AppError> {
        let (Some(provider_id), Some(model)) =
            (participant.provider_id.clone(), participant.model.clone())
        else {
            return Err(AppError::BadRequest(
                "execution participant needs a provider and model".to_owned(),
            ));
        };
        ProviderId::try_from(provider_id.as_str()).map_err(|_| {
            AppError::BadRequest(
                "execution participant has a non-canonical provider_id".to_owned(),
            )
        })?;
        if model.trim().is_empty() || model.trim() != model {
            return Err(AppError::BadRequest(
                "execution participant has an invalid model".to_owned(),
            ));
        }
        let provider = ProviderWithModel {
            provider_id,
            model: model.clone(),
            use_model: Some(model),
        };

        let mut extra = build_agent_extra(
            brief,
            workspace_dir,
            participant.system_prompt.as_deref(),
            &participant.enabled_skills,
            &participant.disabled_builtin_skills,
            tool_policy,
            delegation_depth >= MAX_AGENT_DELEGATION_DEPTH,
        );
        if let Some(snapshot) = participant.preset_snapshot.as_ref() {
            extra["preset_id"] = Value::String(snapshot.preset_id.clone());
            extra["preset_revision"] = Value::Number(snapshot.preset_revision.into());
            extra["preset_snapshot"] = serde_json::to_value(snapshot)
                .map_err(|error| AppError::Internal(format!("encode preset snapshot: {error}")))?;
        }

        let request = CreateConversationRequest {
            r#type: AgentType::Nomi,
            name: Some(format!("协作 · {}", step_title.trim())),
            model: Some(provider),
            source: None,
            channel_chat_id: None,
            preset_id: None,
            preset_overrides: None,
            delegation_policy: if delegation_depth >= MAX_AGENT_DELEGATION_DEPTH {
                DelegationPolicy::Disabled
            } else {
                delegation_policy
            },
            execution_model_pool: Some(ExecutionModelPool::Range {
                models: execution_model_pool.to_vec(),
            }),
            decision_policy,
            execution_template_id: None,
            extra,
        };
        let created = if let Some(snapshot) = participant.preset_snapshot.clone() {
            self.conv
                .create_from_preset_snapshot_idempotent(
                    owner_id,
                    request,
                    snapshot,
                    attempt_creation_key,
                )
                .await
        } else {
            self.conv
                .create_idempotent(owner_id, request, attempt_creation_key)
                .await
        };
        let conversation = match created {
            Ok(conversation) => conversation,
            Err(error) => {
                if let Err(cleanup_error) = self
                    .conv
                    .discard_unlinked_creation(owner_id, attempt_creation_key)
                    .await
                {
                    tracing::warn!(%cleanup_error, "failed to discard partially-created attempt conversation");
                }
                return Err(error);
            }
        };

        // This callback is awaited before the Agent can start. An outbox/link
        // failure leaves no untracked in-flight turn.
        if let Err(error) = on_started(conversation.id.clone()).await {
            // If the link commit succeeded but its acknowledgement was lost,
            // the Conversation deletion guard rejects this cleanup.  Otherwise
            // the creation key and row are removed together, leaving no orphan.
            match self
                .conv
                .discard_unlinked_creation(owner_id, attempt_creation_key)
                .await
            {
                Ok(()) => {}
                Err(AppError::Conflict(_)) => {}
                Err(cleanup_error) => {
                    tracing::warn!(%cleanup_error, "failed to discard unlinked attempt conversation");
                }
            }
            return Err(error);
        }

        let operation_id = format!("{attempt_creation_key}:initial-turn");
        self.deliver_turn(
            owner_id,
            &conversation.id,
            &operation_id,
            step_spec,
            "agent_execution",
            timeout,
        )
        .await
    }

    async fn continue_with_input(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        input: &str,
        timeout: Duration,
    ) -> Result<AttemptOutcome, AppError> {
        self.deliver_turn(
            owner_id,
            conversation_id,
            operation_id,
            input,
            "agent_execution_decision",
            timeout,
        )
        .await
    }

    async fn discard_unlinked_creation(
        &self,
        owner_id: &str,
        attempt_creation_key: &str,
    ) -> Result<(), AppError> {
        self.conv
            .discard_unlinked_creation(owner_id, attempt_creation_key)
            .await
    }

    async fn read_final_output(&self, owner_id: &str, conversation_id: &str) -> Option<String> {
        self.recent_messages(owner_id, conversation_id)
            .await
            .as_ref()
            .and_then(latest_assistant_text)
    }

    async fn last_error_retryable(&self, owner_id: &str, conversation_id: &str) -> bool {
        self.recent_messages(owner_id, conversation_id)
            .await
            .as_ref()
            .is_some_and(latest_error_retryable)
    }

    async fn last_error_present(&self, owner_id: &str, conversation_id: &str) -> bool {
        self.recent_messages(owner_id, conversation_id)
            .await
            .as_ref()
            .is_some_and(latest_error_present)
    }

    async fn last_error_summary(&self, owner_id: &str, conversation_id: &str) -> Option<String> {
        self.recent_messages(owner_id, conversation_id)
            .await
            .as_ref()
            .and_then(latest_error_summary)
    }
}

/// Runtime configuration only. Execution/step/attempt identity is intentionally
/// absent: the durable `ConversationExecutionLink` is the sole relation source.
#[allow(clippy::too_many_arguments)]
fn build_agent_extra(
    brief: &str,
    workspace_dir: Option<&str>,
    persona: Option<&str>,
    enabled_skills: &[String],
    disabled_builtin_skills: &[String],
    tool_policy: AgentToolPolicy,
    exclude_delegation: bool,
) -> Value {
    let restricted = tool_policy_allowed_tools(tool_policy);
    let mut extra = json!({
        "session_mode": "yolo",
        "system_prompt": brief,
        "preset_enabled_skills": enabled_skills,
        "exclude_auto_inject_skills": disabled_builtin_skills,
    });
    if let Some(tools) = restricted {
        extra["allowed_tools"] = json!(tools);
    }
    if exclude_delegation {
        // Subtractive gateway projection: depth stays private in SQLite, while
        // the ceiling Attempt never receives nomi_delegate in MCP tools/list.
        extra["gateway_excluded_tools"] = json!(["nomi_delegate"]);
    }
    if let Some(persona) = persona.map(str::trim).filter(|value| !value.is_empty()) {
        extra["preset_rules"] = json!(persona);
    }
    if let Some(workspace) = workspace_dir.map(str::trim).filter(|value| !value.is_empty()) {
        extra["workspace"] = json!(workspace);
    }
    extra
}

fn tool_policy_allowed_tools(policy: AgentToolPolicy) -> Option<Vec<&'static str>> {
    match policy {
        AgentToolPolicy::Full => None,
        AgentToolPolicy::ReadOnly => Some(vec!["Read", "Grep", "Glob"]),
        AgentToolPolicy::ReadShell => Some(vec!["Read", "Grep", "Glob", "Bash"]),
    }
}

fn latest_assistant_text(value: &Value) -> Option<String> {
    match value {
        Value::Array(values) => values.iter().find_map(latest_assistant_text),
        Value::Object(map) => {
            let is_text = map.get("position").and_then(Value::as_str) == Some("left")
                && map.get("type").and_then(Value::as_str) == Some("text");
            if is_text
                && let Some(text) = map
                    .get("content")
                    .and_then(|content| content.get("content"))
                    .and_then(Value::as_str)
            {
                return Some(text.to_owned());
            }
            map.values().find_map(latest_assistant_text)
        }
        _ => None,
    }
}

fn latest_error_retryable(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().find_map(error_retryable_flag).unwrap_or(false),
        _ => error_retryable_flag(value).unwrap_or(false),
    }
}

fn error_retryable_flag(value: &Value) -> Option<bool> {
    let content = value.as_object()?.get("content")?;
    if content.get("type").and_then(Value::as_str) != Some("error") {
        return None;
    }
    Some(
        content
            .get("error")
            .and_then(|error| error.get("retryable"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    )
}

fn latest_error_present(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(error_marker_present),
        _ => error_marker_present(value),
    }
}

fn error_marker_present(value: &Value) -> bool {
    value
        .as_object()
        .and_then(|object| object.get("content"))
        .and_then(|content| content.get("type"))
        .and_then(Value::as_str)
        == Some("error")
}

fn latest_error_summary(value: &Value) -> Option<String> {
    match value {
        Value::Array(values) => values.iter().find_map(error_summary),
        _ => error_summary(value),
    }
}

fn error_summary(value: &Value) -> Option<String> {
    let content = value.as_object()?.get("content")?;
    if content.get("type").and_then(Value::as_str) != Some("error") {
        return None;
    }
    let error = content.get("error")?;
    match (
        error.get("code").and_then(Value::as_str),
        error.get("message").and_then(Value::as_str),
    ) {
        (Some(code), Some(message)) => Some(format!("{code}: {message}")),
        (Some(code), None) => Some(code.to_owned()),
        (None, Some(message)) => Some(message.to_owned()),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_extra_has_no_execution_identity_cache() {
        let extra = build_agent_extra(
            "brief",
            None,
            None,
            &[],
            &[],
            AgentToolPolicy::Full,
            false,
        );
        assert!(extra.get("execution_id").is_none());
        assert!(extra.get("step_id").is_none());
        assert!(extra.get("attempt_id").is_none());
        assert!(extra.get("delegation_depth").is_none());
    }

    #[test]
    fn recursion_ceiling_removes_delegate_without_exposing_depth() {
        let extra = build_agent_extra(
            "brief",
            None,
            None,
            &[],
            &[],
            AgentToolPolicy::Full,
            true,
        );
        assert_eq!(extra["gateway_excluded_tools"], json!(["nomi_delegate"]));
        assert!(extra.get("delegation_depth").is_none());
    }

    #[test]
    fn explicit_tool_policy_is_the_only_runtime_tool_narrowing() {
        assert_eq!(
            tool_policy_allowed_tools(AgentToolPolicy::ReadOnly).unwrap(),
            ["Read", "Grep", "Glob"]
        );
        assert_eq!(
            tool_policy_allowed_tools(AgentToolPolicy::ReadShell).unwrap(),
            ["Read", "Grep", "Glob", "Bash"]
        );
        assert!(tool_policy_allowed_tools(AgentToolPolicy::Full).is_none());
    }
}
