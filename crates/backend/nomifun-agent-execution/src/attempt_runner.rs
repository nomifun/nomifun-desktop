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
use nomifun_api_types::{
    ConversationResponse, CreateConversationRequest, ExecutionModelPool, ExecutionModelRef,
    ExecutionParticipant,
    AgentResolvedSnapshot, SendMessageRequest,
};
use nomifun_common::{
    AgentToolPolicy, AgentType, AppError, DecisionPolicy, DelegationPolicy,
    MAX_AGENT_DELEGATION_DEPTH, ProviderId,
    ProviderWithModel,
};
use nomifun_db::AgentExecutionTurnAuthority;
use serde_json::{Value, json};

use crate::delivery::{AgentExecutionDelivery, AgentExecutionTurnOutput};

const DELIVERY_RECEIPT_GRACE: Duration = Duration::from_secs(5);
const DELIVERY_RECEIPT_WAIT_POLL: Duration = Duration::from_millis(500);
const DELIVERY_RECEIPT_POLL: Duration = Duration::from_millis(100);
pub(crate) const MISSING_DELIVERY_RECEIPT_CODE: &str = "agent_delivery_receipt_missing";

/// Async callback invoked immediately after the Agent conversation is created
/// and before its first message is sent. The scheduler uses it to persist the
/// attempt link and make cancellation/recovery race-free.
pub(crate) type AttemptStarted = Box<
    dyn FnOnce(
            String,
        ) -> Pin<Box<dyn Future<Output = Result<AgentExecutionTurnAuthority, AppError>> + Send>>
        + Send,
>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AttemptSessionTarget {
    /// Ordinary collaboration owns a separate immutable audit transcript.
    ChildAttempt,
    /// AutoWork drives the exact AgentSession selected by the user. The
    /// Session is not created, renamed, or later cleaned up as an Attempt.
    AutomationLead { conversation_id: String },
}

fn attempt_turn_input(
    session_target: &AttemptSessionTarget,
    canonical: bool,
    brief: &str,
    step_spec: &str,
) -> Result<(String, &'static str, bool), AppError> {
    if matches!(session_target, AttemptSessionTarget::AutomationLead { .. }) {
        return Ok((step_spec.to_owned(), "autowork", true));
    }
    let content = if canonical {
        serde_json::to_string(&json!({ "task_brief": brief, "step_spec": step_spec }))
            .map_err(|error| AppError::Internal(format!("encode Attempt input: {error}")))?
    } else {
        step_spec.to_owned()
    };
    Ok((content, "agent_execution", false))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttemptOutcome {
    pub conversation_id: String,
    pub text: Option<String>,
    /// Verified artifact paths observed in this attempt's current turn.
    pub output_files: Vec<String>,
    pub ok: bool,
    pub tokens: Option<i64>,
    /// Structured terminal delivery result. When present, this is more
    /// authoritative than scraping the transcript for an error marker.
    pub error: Option<String>,
    pub error_code: Option<String>,
    pub error_retryable: Option<bool>,
}

#[async_trait]
pub(crate) trait AttemptRunner: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    async fn execute(
        &self,
        owner_id: &str,
        session_target: AttemptSessionTarget,
        participant: &ExecutionParticipant,
        execution_model_pool: &[ExecutionModelRef],
        workspace_dir: Option<&str>,
        step_title: &str,
        tool_policy: AgentToolPolicy,
        managed_process_only: bool,
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
        _authority: AgentExecutionTurnAuthority,
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

    /// Recover only a terminal output verified through the same canonical query
    /// as ordinary settlement. The caller supplies the public operation key.
    async fn recover_outcome(
        &self,
        _owner_id: &str,
        _conversation_id: &str,
        _operation_id: &str,
    ) -> Result<Option<RecoveredAttemptOutcome>, AppError> {
        Ok(None)
    }

    async fn read_adoptable_output(
        &self,
        _owner_id: &str,
        _conversation_id: &str,
    ) -> Result<Option<AttemptOutcome>, AppError> {
        Ok(None)
    }

}

/// Narrow, stateless typed Session command/query surface used by Agent
/// Execution.
///
/// Implementations own no session facts and cannot mint a second identity.
/// Durable turn authority remains with the canonical session owner. The
/// application supplies the implementation from its single Session owner.
#[async_trait]
pub trait AgentExecutionSessionPort: Send + Sync {
    async fn create_idempotent(
        &self,
        owner_id: &str,
        request: CreateConversationRequest,
        creation_key: &str,
    ) -> Result<ConversationResponse, AppError>;

    async fn create_from_agent_snapshot_idempotent(
        &self,
        owner_id: &str,
        request: CreateConversationRequest,
        snapshot: AgentResolvedSnapshot,
        creation_key: &str,
    ) -> Result<ConversationResponse, AppError>;

    async fn discard_unlinked_creation(
        &self,
        owner_id: &str,
        creation_key: &str,
    ) -> Result<(), AppError>;

    async fn deliver_turn(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        authority: AgentExecutionTurnAuthority,
        request: SendMessageRequest,
    ) -> Result<AgentExecutionDelivery, AppError>;

    async fn delivery_result(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
    ) -> Result<Option<AgentExecutionDelivery>, AppError>;

    /// An exact public operation key, or the latest closed Turn for explicit
    /// adoption. Implementations authorize the Session before resolving its
    /// canonical operation and read only canonical events, payloads and effects.
    async fn read_turn_output(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: Option<&str>,
    ) -> Result<Option<AgentExecutionTurnOutput>, AppError>;

    async fn get(
        &self,
        owner_id: &str,
        conversation_id: &str,
    ) -> Result<ConversationResponse, AppError>;

    fn take_turn_tokens(&self, conversation_id: &str) -> Option<i64>;

    async fn cancel_for_execution(
        &self,
        owner_id: &str,
        conversation_id: &str,
    ) -> Result<(), AppError>;

    /// Stop one persisted canonical Turn using the durable effect identity.
    /// Replays and delayed delivery must never select the current successor.
    async fn cancel_turn_for_execution(
        &self,
        owner_id: &str,
        conversation_id: &str,
        cancellation_operation_id: &str,
        target_operation_id: &str,
    ) -> Result<(), AppError>;

    async fn steer_turn(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        request: SendMessageRequest,
    ) -> Result<String, AppError>;

    async fn steer_turn_for_execution(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        target_operation_id: &str,
        request: SendMessageRequest,
    ) -> Result<String, AppError>;

    async fn project_assistant_message_idempotent(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        content: &str,
        origin: &str,
    ) -> Result<String, AppError>;
}

/// Production adapter.  All runtime/turn work goes through the typed session
/// surface above; this type only performs the create/send/wait/read choreography
/// for one attempt.
pub(crate) struct AgentSessionAttemptRunner {
    session: Arc<dyn AgentExecutionSessionPort>,
}

impl AgentSessionAttemptRunner {
    pub fn new(session: Arc<dyn AgentExecutionSessionPort>) -> Self {
        Self { session }
    }

    /// Wait only on the operation-scoped durable receipt exposed by the
    /// execution port. Runtime idleness is observational and can race receipt
    /// finalization; it must not be a second completion authority.
    async fn await_delivery_receipt(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        timeout: Duration,
    ) -> Result<Option<AgentExecutionDelivery>, AppError> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(receipt) = self
                .session
                .delivery_result(owner_id, conversation_id, operation_id)
                .await?
                .filter(|receipt| receipt.completed || receipt.paused_reason.is_some())
            {
                return Ok(Some(receipt));
            }
            if Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(DELIVERY_RECEIPT_WAIT_POLL).await;
        }

        // Preserve the existing short commit grace: a runtime owner can finish
        // immediately before its durable receipt finalizer commits.
        let grace_deadline = Instant::now() + DELIVERY_RECEIPT_GRACE;
        loop {
            if let Some(receipt) = self
                .session
                .delivery_result(owner_id, conversation_id, operation_id)
                .await?
                .filter(|receipt| receipt.completed || receipt.paused_reason.is_some())
            {
                return Ok(Some(receipt));
            }
            if Instant::now() >= grace_deadline {
                break;
            }
            tokio::time::sleep(DELIVERY_RECEIPT_POLL).await;
        }
        Ok(None)
    }

    #[allow(clippy::too_many_arguments)]
    async fn deliver_turn(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        authority: AgentExecutionTurnAuthority,
        content: &str,
        origin: &str,
        hidden: bool,
        timeout: Duration,
    ) -> Result<AttemptOutcome, AppError> {
        let delivery = self
            .session
            .deliver_turn(
                owner_id,
                conversation_id,
                operation_id,
                authority,
                SendMessageRequest {
                    plugin_delivery: None,
                    content: content.to_owned(),
                    files: vec![],
                    inject_skills: vec![],
                    hidden,
                    origin: Some(origin.to_owned()),
                    channel_platform: None,
                },
            )
            .await?;
        let boundary_message_id = delivery.message_id.clone();
        let receipt = if delivery.completed || delivery.paused_reason.is_some() {
            Some(delivery)
        } else {
            self.await_delivery_receipt(owner_id, conversation_id, operation_id, timeout)
                .await?
        };
        if let Some(receipt) = receipt {
            if !receipt.completed && let Some(reason) = receipt.paused_reason {
                return Ok(paused_delivery_outcome(
                    conversation_id, &reason, self.session.take_turn_tokens(conversation_id),
                ));
            }
            let output = self.session.read_turn_output(
                owner_id, conversation_id, Some(operation_id),
            ).await?;
            let Some(output) = output.filter(|output| {
                output.delivery.completed && output.delivery.message_id == receipt.message_id
            }) else {
                return Ok(missing_delivery_receipt_outcome(
                    conversation_id, self.session.take_turn_tokens(conversation_id),
                ));
            };
            return Ok(completed_delivery_outcome(
                conversation_id, output, self.session.take_turn_tokens(conversation_id),
            ));
        }
        // Runtime state and transcript contents are not completion evidence.
        // Without the exact operation receipt, fail closed and let the
        // scheduler report the ambiguous result without replaying its effects.
        tracing::warn!(
            conversation_id,
            operation_id,
            boundary_message_id,
            "agent turn reached its receipt deadline without a completed delivery receipt"
        );
        Ok(missing_delivery_receipt_outcome(
            conversation_id,
            self.session.take_turn_tokens(conversation_id),
        ))
    }


}

#[async_trait]
impl AttemptRunner for AgentSessionAttemptRunner {
    #[allow(clippy::too_many_arguments)]
    async fn execute(
        &self,
        owner_id: &str,
        session_target: AttemptSessionTarget,
        participant: &ExecutionParticipant,
        execution_model_pool: &[ExecutionModelRef],
        workspace_dir: Option<&str>,
        step_title: &str,
        tool_policy: AgentToolPolicy,
        managed_process_only: bool,
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

        let canonical = participant.agent_snapshot.as_ref()
            .is_some_and(|snapshot| snapshot.canonical_binding.is_some());
        let mut extra = if canonical {
            // The immutable Agent owns persona, skills and capability grants.
            // An Attempt supplies task data and a separate subtractive ceiling.
            let constraints = nomifun_api_types::ExecutionConstraints {
                version: 1,
                tool_scope: tool_policy,
                exclude_delegation: delegation_depth >= MAX_AGENT_DELEGATION_DEPTH,
                managed_process_only,
            };
            let mut extra = json!({ nomifun_api_types::EXECUTION_CONSTRAINTS_KEY: constraints });
            if let Some(workspace) = workspace_dir.map(str::trim).filter(|value| !value.is_empty()) {
                extra["workspace"] = json!(workspace);
            }
            extra
        } else { build_agent_extra(
            brief,
            workspace_dir,
            participant.system_prompt.as_deref(),
            &participant.enabled_skills,
            &participant.disabled_builtin_skills,
            tool_policy,
            managed_process_only,
            delegation_depth >= MAX_AGENT_DELEGATION_DEPTH,
        ) };
        if let Some(snapshot) = participant.agent_snapshot.as_ref() {
            extra["preset_id"] = Value::String(snapshot.preset_id.clone());
            extra["preset_revision"] = Value::Number(snapshot.preset_revision.into());
            extra["agent_snapshot"] = serde_json::to_value(snapshot)
                .map_err(|error| AppError::Internal(format!("encode preset snapshot: {error}")))?;
        }

        let creates_child = matches!(&session_target, AttemptSessionTarget::ChildAttempt);
        let conversation = match &session_target {
            AttemptSessionTarget::ChildAttempt => {
                let request = CreateConversationRequest {
                    r#type: AgentType::Nomi,
                    name: Some(format!("协作 · {}", step_title.trim())),
                    model: Some(provider),
                    source: None,
                    channel_chat_id: None,
                    preset_id: None,
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
                let created = if let Some(snapshot) = participant.agent_snapshot.clone() {
                    self.session
                        .create_from_agent_snapshot_idempotent(
                            owner_id,
                            request,
                            snapshot,
                            attempt_creation_key,
                        )
                        .await
                } else {
                    self.session
                        .create_idempotent(owner_id, request, attempt_creation_key)
                        .await
                };
                match created {
                    Ok(conversation) => conversation,
                    Err(error) => {
                        if let Err(cleanup_error) = self
                            .session
                            .discard_unlinked_creation(owner_id, attempt_creation_key)
                            .await
                        {
                            tracing::warn!(%cleanup_error, "failed to discard partially-created attempt conversation");
                        }
                        return Err(error);
                    }
                }
            }
            AttemptSessionTarget::AutomationLead { conversation_id } => {
                let conversation = self.session.get(owner_id, conversation_id).await?;
                if conversation.conversation_id != *conversation_id {
                    return Err(AppError::Conflict(
                        "AutoWork Session lookup returned a different AgentSession".to_owned(),
                    ));
                }
                if conversation.agent_snapshot.as_ref() != participant.agent_snapshot.as_ref() {
                    return Err(AppError::Conflict(
                        "AutoWork lead Agent snapshot changed after execution admission".to_owned(),
                    ));
                }
                conversation
            }
        };

        // This callback is awaited before the Agent can start. An outbox/link
        // failure leaves no untracked in-flight turn.
        let authority = match on_started(conversation.conversation_id.clone()).await {
            Ok(authority) => authority,
            Err(error) => {
            // If the link commit succeeded but its acknowledgement was lost,
            // the Conversation deletion guard rejects this cleanup.  Otherwise
            // the creation key and row are removed together, leaving no orphan.
            if creates_child {
                match self
                    .session
                    .discard_unlinked_creation(owner_id, attempt_creation_key)
                    .await
                {
                    Ok(()) => {}
                    Err(AppError::Conflict(_)) => {}
                    Err(cleanup_error) => {
                        tracing::warn!(%cleanup_error, "failed to discard unlinked attempt conversation");
                    }
                }
            }
            return Err(error);
            }
        };

        let operation_id = format!("{attempt_creation_key}:initial-turn");
        // Durable user input, not a replacement for the Agent's system rules.
        // JSON boundaries preserve arbitrary brief/step text without delimiters
        // that can be closed by the task itself. Retries encode the same input.
        let (task_input, origin, hidden) =
            attempt_turn_input(&session_target, canonical, brief, step_spec)?;
        self.deliver_turn(
            owner_id,
            &conversation.conversation_id,
            &operation_id,
            authority,
            &task_input,
            origin,
            hidden,
            timeout,
        )
        .await
    }

    async fn continue_with_input(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        authority: AgentExecutionTurnAuthority,
        input: &str,
        timeout: Duration,
    ) -> Result<AttemptOutcome, AppError> {
        self.deliver_turn(
            owner_id,
            conversation_id,
            operation_id,
            authority,
            input,
            "agent_execution_decision",
            false,
            timeout,
        )
        .await
    }

    async fn discard_unlinked_creation(
        &self,
        owner_id: &str,
        attempt_creation_key: &str,
    ) -> Result<(), AppError> {
        self.session
            .discard_unlinked_creation(owner_id, attempt_creation_key)
            .await
    }

    async fn read_adoptable_output(
        &self, owner_id: &str, conversation_id: &str,
    ) -> Result<Option<AttemptOutcome>, AppError> {
        Ok(self.session.read_turn_output(owner_id, conversation_id, None).await?
            .filter(|output| output.delivery.completed)
            .map(|output| completed_delivery_outcome(conversation_id, output, None)))
    }

    async fn recover_outcome(
        &self, owner_id: &str, conversation_id: &str, operation_id: &str,
    ) -> Result<Option<RecoveredAttemptOutcome>, AppError> {
        let Some(output) = self.session.read_turn_output(
            owner_id, conversation_id, Some(operation_id),
        ).await?.filter(|output| output.delivery.completed) else { return Ok(None); };
        let terminal_event_id = output.terminal_event_id.clone().ok_or_else(||
            AppError::Conflict("terminal AgentExecution output has no canonical boundary".into()))?;
        Ok(Some(RecoveredAttemptOutcome {
            canonical_operation_id: output.canonical_operation_id.clone(),
            terminal_event_id,
            outcome: completed_delivery_outcome(conversation_id, output, None),
        }))
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
    managed_process_only: bool,
    exclude_delegation: bool,
) -> Value {
    let restricted = managed_process_only
        .then(managed_process_allowed_tools)
        .or_else(|| tool_policy_allowed_tools(tool_policy));
    let system_prompt = if managed_process_only {
        format!(
            "{brief}\n\n\
             ## Managed process lifecycle authority (strict)\n\
             The only workspace action tools available for this Attempt are: {}. \
             Start the requested process exactly once, carry its exact process_id through \
             observation and cleanup, and do not call file, search, exec_command/Bash, VCS, \
             Artifact, delegation, or discovery tools. Runtime control tools such as \
             report_completion remain available but grant no workspace authority.",
            restricted
                .as_ref()
                .expect("managed process tools are present")
                .iter()
                .map(|tool| format!("`{tool}`"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        restricted
        .as_ref()
        .map(|tools| {
            format!(
                "{brief}\n\n\
                 ## Execution tool authority (strict)\n\
                 This is a restricted execution Attempt. The only callable tools for this \
                 task are: {}. ToolSearch may be used only to discover one of those tools. \
                 Do not call, preview, or emit progress for Bash, update_plan, Write, Edit, \
                 ApplyPatch, or any other tool unless its exact name appears in that list. \
                 A tool name mentioned in general instructions is not permission.",
                tools
                    .iter()
                    .map(|tool| format!("`{tool}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .unwrap_or_else(|| brief.to_owned())
    };
    let mut extra = json!({
        "system_prompt": system_prompt,
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

fn managed_process_allowed_tools() -> Vec<&'static str> {
    vec![
        "start_process",
        "poll_process",
        "write_process_stdin",
        "close_process_stdin",
        "resize_process",
        "cancel_process",
    ]
}

/// Runtime idleness and transcript contents are observational only. The
/// operation-scoped durable receipt is the sole authority which may mark an
/// Agent turn successful, so its absence always produces a failed outcome.
fn paused_delivery_outcome(conversation_id: &str, reason: &str, tokens: Option<i64>) -> AttemptOutcome {
    AttemptOutcome {
        conversation_id: conversation_id.to_owned(), text: None, output_files: Vec::new(),
        ok: false, tokens,
        error: Some(format!("agent_execution_paused: Agent turn paused ({reason}); no completed delivery exists. Automatic task replay is blocked.")),
        error_code: Some("agent_execution_paused".into()), error_retryable: Some(false),
    }
}

fn missing_delivery_receipt_outcome(
    conversation_id: &str,
    tokens: Option<i64>,
) -> AttemptOutcome {
    AttemptOutcome {
        conversation_id: conversation_id.to_owned(),
        text: None,
        output_files: Vec::new(),
        ok: false,
        tokens,
        error: Some(
            "Agent turn ended without a completed delivery receipt; automatic retry is blocked because the outcome is ambiguous"
                .to_owned(),
        ),
        error_code: Some(MISSING_DELIVERY_RECEIPT_CODE.to_owned()),
        error_retryable: Some(false),
    }
}

fn completed_delivery_outcome(
    conversation_id: &str,
    output: AgentExecutionTurnOutput,
    tokens: Option<i64>,
) -> AttemptOutcome {
    let mut receipt = output.delivery;
    if receipt.result_ok == Some(true) && !output.integrity_ok {
        // The Turn already completed. Verification failure cannot authorize
        // replaying successful external effects in a new Attempt.
        receipt.result_error = Some("Agent output receipts could not be verified".to_owned());
        receipt.result_error_code = Some("agent_artifact_verification_failed".to_owned());
        receipt.result_error_retryable = Some(false);
    }
    AttemptOutcome {
        conversation_id: conversation_id.to_owned(),
        text: receipt.result_text,
        output_files: output.output_files,
        ok: receipt.result_ok.unwrap_or(false) && output.integrity_ok,
        tokens,
        error: receipt.result_error,
        error_code: receipt.result_error_code,
        error_retryable: receipt.result_error_retryable,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecoveredAttemptOutcome {
    pub outcome: AttemptOutcome,
    pub canonical_operation_id: String,
    pub terminal_event_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_common::{ConversationStatus, TimestampMs, generate_id};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    const CONVERSATION_ID: &str = "0190f5fe-7c00-7a00-8000-000000000201";
    const CURRENT_USER_TURN_ID: &str = "0190f5fe-7c00-7a00-8000-000000000212";

    struct RecordingSessionPort {
        conversation: ConversationResponse,
        create_calls: AtomicUsize,
        delivered: Mutex<Option<(String, SendMessageRequest)>>,
        receipt: Option<AgentExecutionDelivery>,
    }

    #[async_trait]
    impl AgentExecutionSessionPort for RecordingSessionPort {
        async fn create_idempotent(
            &self,
            _owner_id: &str,
            _request: CreateConversationRequest,
            _creation_key: &str,
        ) -> Result<ConversationResponse, AppError> {
            self.create_calls.fetch_add(1, Ordering::SeqCst);
            Err(AppError::Conflict("unexpected child Session creation".to_owned()))
        }

        async fn create_from_agent_snapshot_idempotent(
            &self,
            _owner_id: &str,
            _request: CreateConversationRequest,
            _snapshot: AgentResolvedSnapshot,
            _creation_key: &str,
        ) -> Result<ConversationResponse, AppError> {
            self.create_calls.fetch_add(1, Ordering::SeqCst);
            Err(AppError::Conflict("unexpected child Session creation".to_owned()))
        }

        async fn discard_unlinked_creation(
            &self,
            _owner_id: &str,
            _creation_key: &str,
        ) -> Result<(), AppError> {
            Ok(())
        }

        async fn deliver_turn(
            &self,
            _owner_id: &str,
            conversation_id: &str,
            _operation_id: &str,
            _authority: AgentExecutionTurnAuthority,
            request: SendMessageRequest,
        ) -> Result<AgentExecutionDelivery, AppError> {
            *self.delivered.lock().unwrap() = Some((conversation_id.to_owned(), request));
            Err(AppError::Conflict("captured AutoWork turn".to_owned()))
        }

        async fn delivery_result(
            &self,
            _owner_id: &str,
            _conversation_id: &str,
            _operation_id: &str,
        ) -> Result<Option<AgentExecutionDelivery>, AppError> {
            Ok(self.receipt.clone())
        }

        async fn read_turn_output(
            &self, _owner_id: &str, _conversation_id: &str, _operation_id: Option<&str>,
        ) -> Result<Option<AgentExecutionTurnOutput>, AppError> { Ok(None) }

        async fn get(
            &self,
            _owner_id: &str,
            conversation_id: &str,
        ) -> Result<ConversationResponse, AppError> {
            assert_eq!(conversation_id, self.conversation.conversation_id);
            Ok(self.conversation.clone())
        }

        fn take_turn_tokens(&self, _conversation_id: &str) -> Option<i64> {
            None
        }

        async fn cancel_for_execution(
            &self,
            _owner_id: &str,
            _conversation_id: &str,
        ) -> Result<(), AppError> {
            Ok(())
        }

        async fn cancel_turn_for_execution(
            &self,
            _owner_id: &str,
            _conversation_id: &str,
            _cancellation_operation_id: &str,
            _target_operation_id: &str,
        ) -> Result<(), AppError> {
            Ok(())
        }

        async fn steer_turn(
            &self,
            _owner_id: &str,
            _conversation_id: &str,
            _operation_id: &str,
            _request: SendMessageRequest,
        ) -> Result<String, AppError> {
            Ok(generate_id())
        }

        async fn steer_turn_for_execution(
            &self,
            _owner_id: &str,
            _conversation_id: &str,
            _operation_id: &str,
            _target_operation_id: &str,
            _request: SendMessageRequest,
        ) -> Result<String, AppError> {
            Ok(generate_id())
        }

        async fn project_assistant_message_idempotent(
            &self,
            _owner_id: &str,
            _conversation_id: &str,
            _operation_id: &str,
            _content: &str,
            _origin: &str,
        ) -> Result<String, AppError> {
            Ok(generate_id())
        }
    }

    #[test]
    fn autowork_turn_keeps_the_requirement_on_the_main_session_boundary() {
        let target = AttemptSessionTarget::AutomationLead {
            conversation_id: CONVERSATION_ID.to_owned(),
        };
        let (content, origin, hidden) =
            attempt_turn_input(&target, true, "shared execution wrapper", "exact requirement")
                .unwrap();
        assert_eq!(content, "exact requirement");
        assert_eq!(origin, "autowork");
        assert!(hidden, "the queue instruction is not a user-authored chat message");

        let (content, origin, hidden) = attempt_turn_input(
            &AttemptSessionTarget::ChildAttempt,
            true,
            "shared execution wrapper",
            "child step",
        )
        .unwrap();
        assert!(content.contains("task_brief"));
        assert_eq!(origin, "agent_execution");
        assert!(!hidden);
    }

    fn recording_session(receipt: Option<AgentExecutionDelivery>) -> Arc<RecordingSessionPort> {
        let conversation = ConversationResponse {
            conversation_id: CONVERSATION_ID.to_owned(),
            name: "main Agent".to_owned(),
            r#type: AgentType::Nomi,
            model: None,
            reasoning_effort: None,
            status: ConversationStatus::Finished,
            runtime: None,
            source: None,
            pinned: false,
            pinned_at: None,
            channel_chat_id: None,
            preset_id: None,
            preset_revision: None,
            agent_snapshot: None,
            delegation_policy: DelegationPolicy::Automatic,
            execution_model_pool: None,
            decision_policy: DecisionPolicy::Automatic,
            execution_template_id: None,
            linked_execution_id: None,
            execution_step_id: None,
            execution_attempt_id: None,
            created_at: TimestampMs::from(1),
            modified_at: TimestampMs::from(1),
            extra: json!({}),
        };
        Arc::new(RecordingSessionPort {
            conversation,
            create_calls: AtomicUsize::new(0),
            delivered: Mutex::new(None),
            receipt,
        })
    }

    #[tokio::test]
    async fn canonical_pause_stops_receipt_wait_without_claiming_completion_or_retry() {
        let receipt = AgentExecutionDelivery {
            message_id: CURRENT_USER_TURN_ID.into(), replayed: true, completed: false,
            paused_reason: Some("EXECUTION_MODEL_INVALID_REQUEST".into()),
            result_ok: None, result_text: None, result_error: None,
            result_error_code: None, result_error_retryable: None,
        };
        let runner = AgentSessionAttemptRunner::new(recording_session(Some(receipt.clone())));
        let observed = tokio::time::timeout(Duration::from_secs(1), runner.await_delivery_receipt(
            "owner", CONVERSATION_ID, "operation", Duration::from_secs(30 * 60),
        )).await.expect("a canonical pause must not wait for the full attempt deadline").unwrap().unwrap();
        assert_eq!(observed, receipt);
        let outcome = paused_delivery_outcome(CONVERSATION_ID, observed.paused_reason.as_deref().unwrap(), Some(7));
        assert!(!outcome.ok);
        assert_eq!(outcome.error_retryable, Some(false));
        assert_eq!(outcome.error_code.as_deref(), Some("agent_execution_paused"));
        assert!(outcome.output_files.is_empty());
        assert!(outcome.text.is_none());
    }

    #[tokio::test]
    async fn autowork_reuses_the_bound_session_without_calling_session_creation() {
        let session = recording_session(None);
        let runner = AgentSessionAttemptRunner::new(session.clone());
        let participant = ExecutionParticipant {
            participant_id: generate_id(),
            execution_id: generate_id(),
            source_agent_id: generate_id(),
            preset_id: None,
            preset_revision: None,
            agent_snapshot: None,
            provider_id: Some(generate_id()),
            model: Some("model".to_owned()),
            role: Some("requirement_owner".to_owned()),
            capability: None,
            constraints: None,
            description: None,
            system_prompt: None,
            enabled_skills: Vec::new(),
            disabled_builtin_skills: Vec::new(),
            sort_order: 0,
            introduced_in_revision: 0,
            retired_in_revision: None,
            created_at: 1,
        };
        let execution_id = participant.execution_id.clone();
        let step_id = generate_id();
        let attempt_id = generate_id();
        let callback_attempt_id = attempt_id.clone();
        let outcome = runner
            .execute(
                "owner",
                AttemptSessionTarget::AutomationLead {
                    conversation_id: CONVERSATION_ID.to_owned(),
                },
                &participant,
                &[],
                None,
                "Requirement",
                AgentToolPolicy::Full,
                false,
                DelegationPolicy::Automatic,
                0,
                DecisionPolicy::Automatic,
                &attempt_id,
                "shared wrapper",
                "[AutoWork] perform exact requirement",
                Duration::from_millis(1),
                Box::new(move |conversation_id| {
                    Box::pin(async move {
                        assert_eq!(conversation_id, CONVERSATION_ID);
                        Ok(AgentExecutionTurnAuthority {
                            execution_id,
                            step_id,
                            attempt_id: callback_attempt_id,
                            expected_step_version: 1,
                            expected_attempt_version: 1,
                            lease_owner: "lease".to_owned(),
                        })
                    })
                }),
            )
            .await;
        assert!(matches!(outcome, Err(AppError::Conflict(message)) if message == "captured AutoWork turn"));
        assert_eq!(session.create_calls.load(Ordering::SeqCst), 0);
        let (conversation_id, request) = session.delivered.lock().unwrap().take().unwrap();
        assert_eq!(conversation_id, CONVERSATION_ID);
        assert_eq!(request.origin.as_deref(), Some("autowork"));
        assert!(request.hidden);
        assert_eq!(request.content, "[AutoWork] perform exact requirement");
    }

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
            false,
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

    #[test]
    fn restricted_attempt_prompt_declares_exact_tool_authority() {
        let extra = build_agent_extra(
            "inspect the workspace",
            None,
            None,
            &[],
            &[],
            AgentToolPolicy::ReadOnly,
            false,
            false,
        );
        let prompt = extra["system_prompt"].as_str().unwrap();
        assert!(prompt.contains("`Read`, `Grep`, `Glob`"));
        assert!(prompt.contains("Do not call, preview, or emit progress for Bash"));
        assert!(prompt.contains("unless its exact name appears in that list"));
    }

    #[test]
    fn managed_process_attempt_exposes_only_lifecycle_actions() {
        let extra = build_agent_extra(
            "start and stop helper",
            None,
            None,
            &[],
            &[],
            AgentToolPolicy::Full,
            true,
            false,
        );
        assert_eq!(
            extra["allowed_tools"],
            json!([
                "start_process",
                "poll_process",
                "write_process_stdin",
                "close_process_stdin",
                "resize_process",
                "cancel_process"
            ])
        );
        let prompt = extra["system_prompt"].as_str().unwrap();
        assert!(prompt.contains("Managed process lifecycle authority"));
        assert!(prompt.contains("do not call file, search, exec_command/Bash"));
        assert!(prompt.contains("report_completion remain available"));
    }

    #[test]
    fn receipt_verification_failure_does_not_authorize_replay() {
        let output = AgentExecutionTurnOutput {
            canonical_operation_id: "operation".into(), terminal_event_id: Some("terminal".into()),
            delivery: AgentExecutionDelivery {
                message_id: CURRENT_USER_TURN_ID.into(), replayed: true, completed: true,
                paused_reason: None, result_ok: Some(true), result_text: Some("done".into()),
                result_error: None, result_error_code: None, result_error_retryable: Some(false),
            },
            output_files: Vec::new(), integrity_ok: false,
        };
        let outcome = completed_delivery_outcome(CONVERSATION_ID, output, Some(7));
        assert!(!outcome.ok);
        assert_eq!(outcome.error_code.as_deref(), Some("agent_artifact_verification_failed"));
        assert_eq!(outcome.error_retryable, Some(false));
        assert_eq!(outcome.tokens, Some(7));
    }
}
