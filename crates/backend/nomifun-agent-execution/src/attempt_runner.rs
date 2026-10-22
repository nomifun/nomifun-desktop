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
use std::time::Duration;
use tokio::time::Instant;

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
pub(crate) use nomifun_db::AgentExecutionAttemptDisposition as AttemptDisposition;
use serde_json::{Value, json};

use crate::delivery::{AgentExecutionDelivery, AgentExecutionTurnOutput};

const DELIVERY_RECEIPT_GRACE: Duration = Duration::from_secs(5);
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
    /// Source-authored child preserves exact canonical request provenance and
    /// any established Companion affiliation. Resources alone grant neither.
    SourceChildAttempt { source: crate::AgentExecutionSource },
    /// An explicit new work unit continues the original child's own history.
    ExistingChild { target: nomifun_db::AgentExecutionContinuationTarget },
    /// AutoWork drives the exact AgentSession selected by the user. The
    /// Session is not created, renamed, or later cleaned up as an Attempt.
    AutomationLead { conversation_id: String },
}

enum ChildInputPreparation<'a> {
    Delegated(&'a crate::AgentExecutionSource),
    Continuation,
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
    Ok((content, "agent_execution", matches!(session_target,AttemptSessionTarget::SourceChildAttempt { .. })))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttemptOutcome {
    pub conversation_id: String,
    pub user_input_request: Option<nomifun_agent_contracts::AgentUserInputRequest>,
    /// Exact source of a handoff, retained only for canonical settlement CAS.
    pub canonical_terminal: Option<(String, String)>,
    pub text: Option<String>,
    /// Verified artifact paths observed in this attempt's current turn.
    pub output_files: Vec<String>,
    pub disposition: AttemptDisposition,
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

    /// Follow an already admitted Turn. This may read/wait only; it cannot
    /// deliver the original input or admit a replacement Attempt.
    async fn observe_existing_turn(
        &self,
        _owner_id: &str,
        _conversation_id: &str,
        _operation_id: &str,
    ) -> Result<AttemptOutcome, AppError> {
        Err(AppError::Conflict("existing canonical Turn observer is unavailable".into()))
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

    /// Verify the source's exact admitted Tool/Effect and preserve its frozen
    /// Snapshot and product affiliation. The input entry does not choose a
    /// different Agent role or widen the child's resources.
    async fn create_from_source_snapshot_idempotent(
        &self,
        _owner_id: &str,
        _request: CreateConversationRequest,
        _snapshot: AgentResolvedSnapshot,
        _source: &crate::AgentExecutionSource,
        _creation_key: &str,
    ) -> Result<ConversationResponse, AppError> {
        Err(AppError::Conflict("canonical source-child creator is unavailable".into()))
    }

    /// Read the exact source input selections after the child link is committed.
    /// The normal child input owner still validates its frozen Skills and
    /// prepares attachments; this cannot choose resources or copy a transcript.
    async fn prepare_source_child_input(
        &self,
        _owner_id: &str,
        _child_session_id: &str,
        _source: Option<&crate::AgentExecutionSource>,
        _request: SendMessageRequest,
    ) -> Result<SendMessageRequest, AppError> {
        Err(AppError::Conflict("canonical source-child input preparer is unavailable".into()))
    }

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

    /// Subscribe to the existing owner before re-reading its exact receipt.
    /// Notifications are hints; a bounded fallback re-reads canonical progress
    /// even when no Runtime is attached or the notification stream closes.
    async fn observe_turn_progress(
        &self, owner_id: &str, conversation_id: &str, operation_id: &str,
        after_cursor: Option<u64>,
    ) -> Result<crate::AgentExecutionTurnProgress, AppError> {
        let delivery = self.delivery_result(owner_id, conversation_id, operation_id).await?;
        let progress = crate::AgentExecutionTurnProgress {
            found: delivery.is_some(), terminal: delivery.as_ref().is_some_and(|delivery| delivery.completed), cursor: 0,
            suspended: delivery.as_ref().is_some_and(|delivery| matches!(delivery.turn_outcome,
                Some(nomifun_agent_contracts::AgentTurnOutcome::Waiting { .. } | nomifun_agent_contracts::AgentTurnOutcome::Paused { .. }))),
        };
        if progress.found && !progress.terminal && after_cursor == Some(progress.cursor) {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        Ok(progress)
    }

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

    /// Return the child's own saved Snapshot only after its current binding
    /// matches the original Attempt's exact canonical admission.
    async fn get_original_child(&self,_owner_id:&str,_execution_id:&str,
        _target:&nomifun_db::AgentExecutionContinuationTarget)->Result<ConversationResponse,AppError> {
        Err(AppError::Conflict("original child admission reader is unavailable".into()))
    }

    fn take_turn_tokens(&self, conversation_id: &str) -> Option<i64>;

    /// Cleanup remains bound to the original Attempt invocation even if a
    /// successor has started in this Session. Never select the current Turn.
    async fn cancel_attempt_for_execution(
        &self, owner_id: &str, conversation_id: &str,
        attempt_id: &str, target_operation_id: &str,
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
    ) -> Result<crate::AgentExecutionSteerDelivery, AppError>;

    async fn project_assistant_message_idempotent(
        &self,
        owner_id: &str,
        conversation_id: &str,
        operation_id: &str,
        content: &str,
        origin: &str,
    ) -> Result<String, AppError>;

    /// Project the exact persisted DecisionRequested event to its original
    /// request. Recovery repeats this identity, never a newly authored question.
    async fn notify_execution_input_required_idempotent(&self, _owner_id: &str,
        _input: &crate::AgentExecutionInput) -> Result<(),AppError> {
        Err(AppError::Conflict("canonical execution-input notice writer is unavailable".into()))
    }
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
    ) -> Result<bool, AppError> {
        let mut grace_deadline = Instant::now() + timeout + DELIVERY_RECEIPT_GRACE;
        let mut observed_at = Instant::now();
        let mut was_waiting = false;
        let mut cursor = None;
        loop {
            let progress = self.session.observe_turn_progress(owner_id, conversation_id, operation_id, cursor).await?;
            cursor = Some(progress.cursor);
            let now = Instant::now();
            let waiting = progress.suspended;
            if was_waiting {
                // Human/resource waiting consumes no execution budget. Keep
                // observing this exact owner without replaying its tools.
                let suspended_for = now.saturating_duration_since(observed_at);
                grace_deadline += suspended_for;
            }
            observed_at = now;
            was_waiting = waiting;
            if !progress.found { return Ok(false); }
            if progress.terminal {
                return Ok(true);
            }
            if !waiting && now >= grace_deadline {
                return Ok(false);
            }
            // The port waits on the existing owner and re-reads its receipt.
            // No wait here hydrates transcript or resubmits business input.
        }
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
        preparation: Option<ChildInputPreparation<'_>>,
    ) -> Result<AttemptOutcome, AppError> {
        let request = SendMessageRequest {
            content: content.to_owned(),
            files: vec![],
            inject_skills: vec![],
            hidden,
            origin: Some(origin.to_owned()),
            channel_platform: None,
        };
        let request = match preparation {
            Some(preparation) => self.session.prepare_source_child_input(
                owner_id, conversation_id, match preparation {
                    ChildInputPreparation::Delegated(source) => Some(source),
                    ChildInputPreparation::Continuation => None,
                }, request,
            ).await?,
            None => request,
        };
        let delivery = self
            .session
            .deliver_turn(
                owner_id,
                conversation_id,
                operation_id,
                authority,
                request,
            )
            .await?;
        let boundary_message_id = delivery.message_id.clone();
        let completed = if delivery.completed {
            true
        } else {
            self.await_delivery_receipt(owner_id, conversation_id, operation_id, timeout)
                .await?
        };
        if completed {
            let output = self.session.read_turn_output(
                owner_id, conversation_id, Some(operation_id),
            ).await?;
            let Some(output) = output.filter(|output| {
                output.delivery.completed && output.delivery.message_id == boundary_message_id
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
                exclude_delegation: delegation_policy == DelegationPolicy::Disabled
                    || delegation_depth >= MAX_AGENT_DELEGATION_DEPTH,
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
            delegation_policy == DelegationPolicy::Disabled
                || delegation_depth >= MAX_AGENT_DELEGATION_DEPTH,
        ) };
        if let Some(snapshot) = participant.agent_snapshot.as_ref() {
            extra["preset_id"] = Value::String(snapshot.preset_id.clone());
            extra["preset_revision"] = Value::Number(snapshot.preset_revision.into());
            extra["agent_snapshot"] = serde_json::to_value(snapshot)
                .map_err(|error| AppError::Internal(format!("encode preset snapshot: {error}")))?;
        }

        let creates_child = matches!(&session_target, AttemptSessionTarget::ChildAttempt
            | AttemptSessionTarget::SourceChildAttempt { .. });
        let conversation = match &session_target {
            AttemptSessionTarget::ChildAttempt | AttemptSessionTarget::SourceChildAttempt { .. } => {
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
                let created = if let AttemptSessionTarget::SourceChildAttempt { source } = &session_target {
                    let snapshot = participant.agent_snapshot.clone().ok_or_else(||
                        AppError::Conflict("source child has no frozen Agent snapshot".into()))?;
                    self.session.create_from_source_snapshot_idempotent(
                        owner_id, request, snapshot, source, attempt_creation_key,
                    ).await
                } else if let Some(snapshot) = participant.agent_snapshot.clone() {
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
            AttemptSessionTarget::ExistingChild { target } => {
                let conversation=self.session.get_original_child(owner_id,&participant.execution_id,target).await?;
                if conversation.conversation_id!=target.conversation_id {
                    return Err(AppError::Conflict("original child reader returned a different Session".into()));
                }
                conversation
            }
            AttemptSessionTarget::AutomationLead { conversation_id } => {
                let conversation = self.session.get(owner_id, conversation_id).await?;
                if conversation.conversation_id != *conversation_id {
                    return Err(AppError::Conflict(
                        "existing Session lookup returned a different AgentSession".to_owned(),
                    ));
                }
                if conversation.agent_snapshot.as_ref() != participant.agent_snapshot.as_ref() {
                    return Err(AppError::Conflict(
                        "existing Agent snapshot changed after execution admission".to_owned(),
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
            match &session_target {
                AttemptSessionTarget::SourceChildAttempt { source } => Some(ChildInputPreparation::Delegated(source)),
                AttemptSessionTarget::ExistingChild { .. } => Some(ChildInputPreparation::Continuation),
                _ => None,
            },
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
            None,
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

    async fn observe_existing_turn(
        &self, owner_id: &str, conversation_id: &str, operation_id: &str,
    ) -> Result<AttemptOutcome, AppError> {
        let mut cursor = None;
        loop {
            let progress = self.session.observe_turn_progress(owner_id, conversation_id, operation_id, cursor).await?;
            if !progress.found {
                return Err(AppError::Conflict("original canonical Turn receipt is unavailable; automatic replay is blocked".into()));
            }
            cursor = Some(progress.cursor);
            if !progress.terminal { continue; }
            let output = self.session.read_turn_output(owner_id, conversation_id, Some(operation_id)).await?
                .filter(|output| output.delivery.completed)
                .ok_or_else(|| AppError::Conflict("original terminal Turn output is not yet verified".into()))?;
            return Ok(completed_delivery_outcome(conversation_id, output, None));
        }
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
fn missing_delivery_receipt_outcome(
    conversation_id: &str,
    tokens: Option<i64>,
) -> AttemptOutcome {
    AttemptOutcome {
        user_input_request: None, canonical_terminal: None,
        conversation_id: conversation_id.to_owned(),
        text: None,
        output_files: Vec::new(),
        disposition: AttemptDisposition::Failed,
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
    use nomifun_agent_contracts::AgentTurnOutcome;
    let public_result_valid = match receipt.turn_outcome.as_ref() {
        Some(AgentTurnOutcome::Answered { message_ids }) => !message_ids.is_empty() && receipt.user_input_request.is_none(),
        Some(AgentTurnOutcome::InputRequired { request_id }) => receipt.user_input_request.as_ref()
            .is_some_and(|request| request.request_id.as_ref() == request_id && request.validate().is_ok()),
        // Verified domain artifacts remain valid output even without a chat
        // answer. Only the existing canonical file receipts prove delivery.
        Some(AgentTurnOutcome::Unavailable) => !output.output_files.is_empty(),
        _ => false,
    };
    if receipt.result_ok == Some(true) && !public_result_valid {
        receipt.result_error = Some("Agent turn closed without an authoritative public result or verified artifact".into());
        receipt.result_error_code = Some("agent_turn_result_unavailable".into());
        receipt.result_error_retryable = Some(false);
    }
    if receipt.result_ok == Some(true) && !output.integrity_ok {
        // The Turn already completed. Verification failure cannot authorize
        // replaying successful external effects in a new Attempt.
        receipt.result_error = Some("Agent output receipts could not be verified".to_owned());
        receipt.result_error_code = Some("agent_artifact_verification_failed".to_owned());
        receipt.result_error_retryable = Some(false);
    }
    let disposition = if matches!(receipt.turn_outcome, Some(AgentTurnOutcome::Cancelled { .. })) {
        AttemptDisposition::Cancelled
    } else if receipt.result_ok == Some(true) && output.integrity_ok && public_result_valid {
        if receipt.user_input_request.is_some() { AttemptDisposition::InputRequired } else { AttemptDisposition::Delivered }
    } else { AttemptDisposition::Failed };
    AttemptOutcome {
        canonical_terminal: output.terminal_event_id.clone().map(|terminal| (output.canonical_operation_id.clone(), terminal)),
        user_input_request: receipt.user_input_request,
        conversation_id: conversation_id.to_owned(),
        text: receipt.result_text,
        output_files: output.output_files,
        disposition,
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
        created_request: Mutex<Option<CreateConversationRequest>>,
        request_source: Mutex<Option<crate::AgentExecutionSource>>,
        prepared_source: Mutex<Option<crate::AgentExecutionSource>>,
        delivered: Mutex<Option<(String, SendMessageRequest)>>,
        delivery_calls: AtomicUsize,
        receipt: Mutex<Option<AgentExecutionDelivery>>,
        output: Mutex<Option<AgentExecutionTurnOutput>>,
        output_reads: AtomicUsize,
        progress_reads: Mutex<Vec<(String, Option<u64>)>>,
        progress_waiting: tokio::sync::Notify,
        progress_release: tokio::sync::Notify,
        progress_error: Mutex<Option<AppError>>,
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
            request: CreateConversationRequest,
            _snapshot: AgentResolvedSnapshot,
            _creation_key: &str,
        ) -> Result<ConversationResponse, AppError> {
            self.create_calls.fetch_add(1, Ordering::SeqCst);
            *self.created_request.lock().unwrap() = Some(request);
            Err(AppError::Conflict("unexpected child Session creation".to_owned()))
        }

        async fn create_from_source_snapshot_idempotent(
            &self, owner_id: &str, request: CreateConversationRequest,
            snapshot: AgentResolvedSnapshot, source: &crate::AgentExecutionSource,
            creation_key: &str,
        ) -> Result<ConversationResponse, AppError> {
            *self.request_source.lock().unwrap() = Some(source.clone());
            self.create_from_agent_snapshot_idempotent(owner_id, request, snapshot, creation_key).await
        }

        async fn prepare_source_child_input(
            &self, _owner_id: &str, child_session_id: &str,
            source: Option<&crate::AgentExecutionSource>, mut request: SendMessageRequest,
        ) -> Result<SendMessageRequest, AppError> {
            assert_eq!(child_session_id, CONVERSATION_ID);
            assert!(request.files.is_empty() && request.inject_skills.is_empty());
            if let Some(source)=source {
                *self.prepared_source.lock().unwrap() = Some(source.clone());
                request.files.push("exact-source-image".into());
                request.inject_skills.push("exact-source-skill".into());
            }
            Ok(request)
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
            self.delivery_calls.fetch_add(1, Ordering::SeqCst);
            *self.delivered.lock().unwrap() = Some((conversation_id.to_owned(), request));
            self.receipt.lock().unwrap().clone().ok_or_else(|| AppError::Conflict("captured AutoWork turn".to_owned()))
        }

        async fn delivery_result(
            &self,
            _owner_id: &str,
            _conversation_id: &str,
            _operation_id: &str,
        ) -> Result<Option<AgentExecutionDelivery>, AppError> {
            Ok(self.receipt.lock().unwrap().clone())
        }

        async fn observe_turn_progress(
            &self, _owner_id: &str, _conversation_id: &str, operation_id: &str,
            after_cursor: Option<u64>,
        ) -> Result<crate::AgentExecutionTurnProgress, AppError> {
            if let Some(error) = self.progress_error.lock().unwrap().take() { return Err(error); }
            self.progress_reads.lock().unwrap().push((operation_id.to_owned(), after_cursor));
            if after_cursor.is_some() && self.output.lock().unwrap().is_none() {
                self.progress_waiting.notify_one();
                self.progress_release.notified().await;
            }
            let receipt = self.receipt.lock().unwrap().clone();
            Ok(crate::AgentExecutionTurnProgress { found: receipt.is_some(),
                terminal: self.output.lock().unwrap().is_some() || receipt.as_ref().is_some_and(|receipt| receipt.completed), cursor: 1,
                suspended: receipt.as_ref().is_some_and(|receipt| matches!(receipt.turn_outcome,
                    Some(nomifun_agent_contracts::AgentTurnOutcome::Waiting { .. } | nomifun_agent_contracts::AgentTurnOutcome::Paused { .. }))),
            })
        }

        async fn read_turn_output(
            &self, _owner_id: &str, _conversation_id: &str, _operation_id: Option<&str>,
        ) -> Result<Option<AgentExecutionTurnOutput>, AppError> {
            self.output_reads.fetch_add(1, Ordering::SeqCst);
            Ok(self.output.lock().unwrap().clone())
        }

        async fn get(
            &self,
            _owner_id: &str,
            conversation_id: &str,
        ) -> Result<ConversationResponse, AppError> {
            assert_eq!(conversation_id, self.conversation.conversation_id);
            Ok(self.conversation.clone())
        }

        async fn get_original_child(&self,owner_id:&str,_execution_id:&str,target:&nomifun_db::AgentExecutionContinuationTarget)
            ->Result<ConversationResponse,AppError> {
            self.get(owner_id,&target.conversation_id).await
        }

        fn take_turn_tokens(&self, _conversation_id: &str) -> Option<i64> {
            None
        }

        async fn cancel_attempt_for_execution(
            &self,
            _owner_id: &str,
            _conversation_id: &str,
            _attempt_id: &str,
            _target_operation_id: &str,
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
        ) -> Result<crate::AgentExecutionSteerDelivery, AppError> {
            Ok(crate::AgentExecutionSteerDelivery::Delivered { event_id:generate_id() })
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
        let sourced=AttemptSessionTarget::SourceChildAttempt{source:crate::AgentExecutionSource{
            agent_session_id:CONVERSATION_ID.into(),turn_operation_id:"source".into(),tool_operation_id:"tool".into(),effect_id:"effect".into()}};
        let (_,origin,hidden)=attempt_turn_input(&sourced,true,"host context","requested background task").unwrap();
        assert_eq!(origin,"agent_execution");assert!(hidden,"a background host task is not a second human utterance");
    }

    fn recording_session(receipt: Option<AgentExecutionDelivery>) -> Arc<RecordingSessionPort> {
        let conversation = ConversationResponse {
            app_project_id: None,
            conversation_id: CONVERSATION_ID.to_owned(),
            session_purpose: nomifun_agent_contracts::SessionPurpose::Conversation,
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
            created_request: Mutex::new(None),
            request_source: Mutex::new(None),
            prepared_source: Mutex::new(None),
            create_calls: AtomicUsize::new(0),
            delivered: Mutex::new(None),
            delivery_calls: AtomicUsize::new(0),
            receipt: Mutex::new(receipt),
            output: Mutex::new(None),
            output_reads: AtomicUsize::new(0),
            progress_reads: Mutex::new(Vec::new()),
            progress_waiting: tokio::sync::Notify::new(),
            progress_release: tokio::sync::Notify::new(),
            progress_error: Mutex::new(None),
        })
    }

    #[tokio::test]
    async fn source_child_uses_the_ordinary_input_with_exact_prepared_selections() {
        let session = recording_session(Some(AgentExecutionDelivery {
            message_id: CURRENT_USER_TURN_ID.into(), replayed: false, completed: true,
            turn_outcome: None, user_input_request: None, result_ok: None, result_text: None,
            result_error: None, result_error_code: None, result_error_retryable: None,
        }));
        let runner = AgentSessionAttemptRunner::new(session.clone());
        let source = crate::AgentExecutionSource { agent_session_id:"exact-parent".into(),
            turn_operation_id:"parent-turn".into(),tool_operation_id:"delegate-tool".into(),effect_id:"delegate-effect".into() };
        runner.deliver_turn("owner", CONVERSATION_ID, "child-initial-turn",
            AgentExecutionTurnAuthority { execution_id:generate_id(),step_id:generate_id(),attempt_id:generate_id(),
                expected_step_version:1,expected_attempt_version:1,lease_owner:"owner".into() },
            "child step remains its own input", "agent_execution", true, Duration::from_secs(1),
            Some(ChildInputPreparation::Delegated(&source))).await.unwrap();
        assert_eq!(session.prepared_source.lock().unwrap().as_ref(), Some(&source));
        let (child, input) = session.delivered.lock().unwrap().take().unwrap();
        assert_eq!(child, CONVERSATION_ID);
        assert_eq!(input.content, "child step remains its own input");
        assert_eq!(input.files, ["exact-source-image"]);
        assert_eq!(input.inject_skills, ["exact-source-skill"]);
        assert_eq!(session.delivery_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn recovery_observer_follows_the_original_receipt_without_delivery_or_history_polling() {
        let pending = AgentExecutionDelivery {
            message_id: CURRENT_USER_TURN_ID.into(), replayed: true, completed: false,
            turn_outcome: None, user_input_request: None, result_ok: None, result_text: None,
            result_error: None, result_error_code: None, result_error_retryable: None,
        };
        let session = recording_session(Some(pending.clone()));
        let runner = AgentSessionAttemptRunner::new(session.clone());
        let observer = runner.observe_existing_turn("owner", CONVERSATION_ID, "original-attempt:initial-turn");
        tokio::pin!(observer);
        tokio::select! {
            result = &mut observer => panic!("pending original Turn cannot settle: {result:?}"),
            _ = session.progress_waiting.notified() => {}
        }
        assert_eq!(session.output_reads.load(Ordering::SeqCst), 0, "pending observation does not hydrate canonical output");
        assert_eq!(session.create_calls.load(Ordering::SeqCst), 0);
        assert!(session.delivered.lock().unwrap().is_none());
        let completed = AgentExecutionDelivery { completed: true, result_ok: Some(true), result_text: Some("original result".into()),
            turn_outcome: Some(nomifun_agent_contracts::AgentTurnOutcome::Answered { message_ids: vec![CURRENT_USER_TURN_ID.into()] }), ..pending };
        *session.output.lock().unwrap() = Some(AgentExecutionTurnOutput {
            canonical_operation_id: "exact-original-operation".into(), terminal_event_id: Some("original-terminal".into()),
            delivery: completed, output_files: Vec::new(), integrity_ok: true, workspace_changed: false,
        });
        session.progress_release.notify_one();
        let result = observer.await.unwrap();
        assert_eq!(result.disposition, AttemptDisposition::Delivered);
        assert_eq!(result.text.as_deref(), Some("original result"));
        assert_eq!(result.canonical_terminal, Some(("exact-original-operation".into(), "original-terminal".into())));
        assert_eq!(session.output_reads.load(Ordering::SeqCst), 1);
        assert!(session.progress_reads.lock().unwrap().iter().all(|(operation, _)| operation == "original-attempt:initial-turn"));
        assert_eq!(session.create_calls.load(Ordering::SeqCst), 0);
        assert!(session.delivered.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn recovery_observer_missing_receipt_never_falls_back_to_delivery() {
        let session = recording_session(None);
        let runner = AgentSessionAttemptRunner::new(session.clone());
        assert!(matches!(runner.observe_existing_turn("owner", CONVERSATION_ID, "missing-original").await,
            Err(AppError::Conflict(_))));
        assert_eq!(session.output_reads.load(Ordering::SeqCst), 0);
        assert_eq!(session.create_calls.load(Ordering::SeqCst), 0);
        assert!(session.delivered.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn receipt_query_failure_after_delivery_cannot_resubmit_the_original_input() {
        let session = recording_session(Some(AgentExecutionDelivery {
            message_id: CURRENT_USER_TURN_ID.into(), replayed: false, completed: false,
            turn_outcome: None, user_input_request: None, result_ok: None, result_text: None,
            result_error: None, result_error_code: None, result_error_retryable: None,
        }));
        *session.progress_error.lock().unwrap() = Some(AppError::Timeout("canonical query unavailable".into()));
        let runner = AgentSessionAttemptRunner::new(session.clone());
        let result = runner.deliver_turn("owner", CONVERSATION_ID, "original-operation",
            AgentExecutionTurnAuthority { execution_id: generate_id(), step_id: generate_id(), attempt_id: generate_id(),
                expected_step_version: 1, expected_attempt_version: 1, lease_owner: "original-owner".into() },
            "original accepted input", "agent_execution", false, Duration::from_secs(1), None).await;
        assert!(matches!(result, Err(AppError::Timeout(_))));
        assert_eq!(session.delivery_calls.load(Ordering::SeqCst), 1);
        assert_eq!(session.output_reads.load(Ordering::SeqCst), 0);
        assert_eq!(session.create_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn receipt_observation_freezes_suspension_time_and_preserves_accumulated_running_budget() {
        let pending = AgentExecutionDelivery {
            message_id: CURRENT_USER_TURN_ID.into(), replayed: true, completed: false,
            turn_outcome: Some(nomifun_agent_contracts::AgentTurnOutcome::Paused { message_ids: Vec::new() }),
            user_input_request: None, result_ok: None, result_text: None, result_error: None,
            result_error_code: None, result_error_retryable: None,
        };
        let session = recording_session(Some(pending.clone()));
        let runner = AgentSessionAttemptRunner::new(session.clone());
        let wait = runner.await_delivery_receipt("owner", CONVERSATION_ID, "budget-original", Duration::from_secs(2));
        tokio::pin!(wait);
        tokio::select! {
            result = &mut wait => panic!("paused task cannot settle: {result:?}"),
            _ = session.progress_waiting.notified() => {}
        }
        tokio::time::advance(Duration::from_secs(60)).await;
        *session.receipt.lock().unwrap() = Some(AgentExecutionDelivery { turn_outcome: None, ..pending });
        session.progress_release.notify_one();
        tokio::select! {
            result = &mut wait => panic!("suspension cannot consume the running budget: {result:?}"),
            _ = session.progress_waiting.notified() => {}
        }
        tokio::time::advance(Duration::from_secs(4)).await;
        session.progress_release.notify_one();
        tokio::select! {
            result = &mut wait => panic!("the original remaining grace has not elapsed: {result:?}"),
            _ = session.progress_waiting.notified() => {}
        }
        tokio::time::advance(Duration::from_secs(4)).await;
        session.progress_release.notify_one();
        assert!(!wait.await.unwrap(), "repeated reads must not reset the original accumulated budget");
        assert_eq!(session.output_reads.load(Ordering::SeqCst), 0);
        assert!(session.delivered.lock().unwrap().is_none());
        assert_eq!(session.create_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn native_pause_keeps_the_exact_attempt_until_resume_or_cancel() {
        let paused = AgentExecutionDelivery {
            turn_outcome: Some(nomifun_agent_contracts::AgentTurnOutcome::Paused { message_ids: vec![] }),
            user_input_request: None, message_id: CURRENT_USER_TURN_ID.into(), replayed: true, completed: false,
            result_ok: None, result_text: None, result_error: None,
            result_error_code: None, result_error_retryable: None,
        };
        let session = recording_session(Some(paused.clone()));
        let runner = AgentSessionAttemptRunner::new(session.clone());
        let wait = runner.await_delivery_receipt("owner", CONVERSATION_ID, "operation", Duration::from_millis(1));
        tokio::pin!(wait);
        assert!(tokio::time::timeout(Duration::from_millis(30), &mut wait).await.is_err(),
            "a paused receipt must never settle or fail the Attempt");
        let completed = AgentExecutionDelivery { completed: true, result_ok: Some(true),
            turn_outcome: Some(nomifun_agent_contracts::AgentTurnOutcome::Answered { message_ids: vec![CURRENT_USER_TURN_ID.into()] }), ..paused };
        *session.receipt.lock().unwrap() = Some(completed.clone());
        session.progress_release.notify_one();
        let observed = tokio::time::timeout(Duration::from_secs(2), &mut wait).await.unwrap().unwrap();
        assert!(observed);
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

    #[tokio::test]
    async fn completed_child_followup_delivers_a_new_input_in_the_original_session() {
        let session=recording_session(None);
        let runner=AgentSessionAttemptRunner::new(session.clone());
        let participant=ExecutionParticipant {
            participant_id:generate_id(),execution_id:generate_id(),source_agent_id:generate_id(),
            preset_id:None,preset_revision:None,agent_snapshot:None,provider_id:Some(generate_id()),model:Some("model".into()),
            role:Some("original child".into()),capability:None,constraints:None,description:None,system_prompt:None,
            enabled_skills:vec![],disabled_builtin_skills:vec![],sort_order:0,introduced_in_revision:0,retired_in_revision:None,created_at:1,
        };
        let execution_id=participant.execution_id.clone();
        let step_id=generate_id();let attempt_id=generate_id();let callback_attempt=attempt_id.clone();
        let result=runner.execute("owner",AttemptSessionTarget::ExistingChild {target:nomifun_db::AgentExecutionContinuationTarget {
            conversation_id:CONVERSATION_ID.into(),original_step_id:generate_id(),original_attempt_id:generate_id()}},
            &participant,&[],None,"continued child",AgentToolPolicy::Full,false,DelegationPolicy::Automatic,1,
            DecisionPolicy::Automatic,&attempt_id,"the new Step","Continue with the next independent input",Duration::from_millis(1),
            Box::new(move|conversation|Box::pin(async move {
                assert_eq!(conversation,CONVERSATION_ID);
                Ok(AgentExecutionTurnAuthority {execution_id,step_id,attempt_id:callback_attempt,
                    expected_step_version:1,expected_attempt_version:1,lease_owner:"lease".into()})
            }))).await;
        assert!(matches!(result,Err(AppError::Conflict(message)) if message=="captured AutoWork turn"));
        assert_eq!(session.create_calls.load(Ordering::SeqCst),0,"continuing a child cannot fork its transcript");
        let (conversation,input)=session.delivered.lock().unwrap().take().unwrap();
        assert_eq!(conversation,CONVERSATION_ID);
        assert_eq!(input.origin.as_deref(),Some("agent_execution"));
        assert!(!input.hidden);
        assert_eq!(input.content,"Continue with the next independent input");
        assert!(session.request_source.lock().unwrap().is_none(),"the original source attachments are already in this child's history");
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

    #[tokio::test]
    async fn source_child_preserves_frozen_grants_and_user_policies() {
        for (delegation_policy, decision_policy) in [
            (DelegationPolicy::Disabled, DecisionPolicy::AskUser),
            (DelegationPolicy::Automatic, DecisionPolicy::Automatic),
        ] {
        let session = recording_session(None);
        let runner = AgentSessionAttemptRunner::new(session.clone());
        let preset = generate_id();
        let provider = generate_id();
        let snapshot: AgentResolvedSnapshot = serde_json::from_value(json!({
            "preset_id":preset,"preset_revision":3,"preset_name":"Companion",
            "canonical_binding":{
                "preset_revision_ref":{"preset_id":preset,"revision":3,"revision_digest":"a".repeat(64)},
                "resolved_snapshot_ref":{"snapshot_id":generate_id(),"snapshot_digest":"b".repeat(64)},
                "typed_resource_bindings":[],"binding_version":1
            },
            "resolved_model":{"provider_id":provider,"model":"companion-model"},
            "enabled_capabilities":["computer","robot"],"instructions":"companion rules"
        })).unwrap();
        let participant = ExecutionParticipant {
            participant_id:generate_id(), execution_id:generate_id(), source_agent_id:generate_id(),
            preset_id:Some(preset),preset_revision:Some(3),agent_snapshot:Some(snapshot.clone()),
            provider_id:Some(provider),model:Some("companion-model".into()),role:None,capability:None,
            constraints:None,description:None,system_prompt:None,enabled_skills:vec![],
            disabled_builtin_skills:vec![],sort_order:0,introduced_in_revision:0,
            retired_in_revision:None,created_at:1,
        };
        let source = crate::AgentExecutionSource { agent_session_id:CONVERSATION_ID.into(),
            turn_operation_id:"source-turn".into(),tool_operation_id:"source-tool".into(),effect_id:"source-effect".into() };
        let result = runner.execute("owner",AttemptSessionTarget::SourceChildAttempt { source:source.clone() },&participant,&[],None,
            "Background task",AgentToolPolicy::Full,false,delegation_policy,0,
            decision_policy,&generate_id(),"task context","do the requested work",
            Duration::from_secs(1),Box::new(|_|Box::pin(async { panic!("creation is deliberately intercepted") })),
        ).await;
        assert!(result.is_err());
        let request = session.created_request.lock().unwrap().take().unwrap();
        assert_eq!(request.delegation_policy,delegation_policy);
        assert_eq!(request.decision_policy,decision_policy);
        assert_eq!(request.extra[nomifun_api_types::EXECUTION_CONSTRAINTS_KEY]["exclude_delegation"].as_bool().unwrap_or(false),
            delegation_policy == DelegationPolicy::Disabled);
        assert_eq!(request.extra["agent_snapshot"],serde_json::to_value(snapshot).unwrap());
        assert_eq!(session.create_calls.load(Ordering::SeqCst),1);
        assert_eq!(session.request_source.lock().unwrap().as_ref(),Some(&source));
        assert!(session.delivered.lock().unwrap().is_none());
        }
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
    fn completed_input_handoff_keeps_its_exact_source_for_settlement_and_recovery() {
        let request = nomifun_agent_contracts::AgentUserInputRequest {
            request_id:"ask-name".into(), question:"姓名？".into(), remaining_work:"生成 PPT。".into(),
        };
        let output = AgentExecutionTurnOutput {
            canonical_operation_id:"original-operation".into(), terminal_event_id:Some("original-terminal".into()),
            delivery:AgentExecutionDelivery {
                turn_outcome: Some(nomifun_agent_contracts::AgentTurnOutcome::InputRequired { request_id: request.request_id.as_ref().to_owned() }),
                message_id:CURRENT_USER_TURN_ID.into(), replayed:true, completed:true,
                user_input_request:Some(request.clone()),
                result_ok:Some(true), result_text:Some("请补充姓名。".into()),
                result_error:None, result_error_code:None, result_error_retryable:Some(false),
            }, output_files:vec!["/workspace/draft.pptx".into()], integrity_ok:true, workspace_changed:false,
        };
        let outcome = completed_delivery_outcome(CONVERSATION_ID, output, Some(7));
        assert_eq!(outcome.disposition, AttemptDisposition::InputRequired);
        assert_eq!(outcome.user_input_request, Some(request));
        assert_eq!(outcome.canonical_terminal, Some(("original-operation".into(),"original-terminal".into())));
        assert!(!crate::scheduler::agent_outcome_can_complete(&outcome));
    }

    #[test]
    fn canonical_cancellation_is_typed_and_retains_exact_source_without_failure_classification() {
        for typed_cancelled in [true, false] {
            let output = AgentExecutionTurnOutput {
                canonical_operation_id: "original-operation".into(), terminal_event_id: Some("original-terminal".into()),
                delivery: AgentExecutionDelivery {
                    message_id: CURRENT_USER_TURN_ID.into(), replayed: true, completed: true,
                    turn_outcome: Some(if typed_cancelled { nomifun_agent_contracts::AgentTurnOutcome::Cancelled { message_ids:vec![] } }
                        else { nomifun_agent_contracts::AgentTurnOutcome::Failed { message_ids:vec![] } }),
                    user_input_request: None, result_ok: Some(false), result_text: None,
                    result_error: Some("The caller cancelled or a timeout occurred".into()),
                    result_error_code: Some(if typed_cancelled {"USER_LLM_PROVIDER_TIMEOUT"} else {"cancelled"}.into()),
                    result_error_retryable: Some(true),
                }, output_files: Vec::new(), integrity_ok: true, workspace_changed: false,
            };
            let outcome = completed_delivery_outcome(CONVERSATION_ID, output, Some(7));
            assert_eq!(outcome.disposition, if typed_cancelled {AttemptDisposition::Cancelled} else {AttemptDisposition::Failed});
            assert_eq!(outcome.canonical_terminal, Some(("original-operation".into(), "original-terminal".into())));
            assert!(!crate::scheduler::agent_outcome_can_complete(&outcome));
        }
    }

    #[test]
    fn receipt_verification_failure_does_not_authorize_replay() {
        let output = AgentExecutionTurnOutput {
            canonical_operation_id: "operation".into(), terminal_event_id: Some("terminal".into()),
            delivery: AgentExecutionDelivery {
                turn_outcome: Some(nomifun_agent_contracts::AgentTurnOutcome::Answered { message_ids: vec!["answer".into()] }),
                user_input_request: None,
                message_id: CURRENT_USER_TURN_ID.into(), replayed: true, completed: true, result_ok: Some(true), result_text: Some("done".into()),
                result_error: None, result_error_code: None, result_error_retryable: Some(false),
            },
            output_files: Vec::new(), integrity_ok: false, workspace_changed: false,
        };
        let outcome = completed_delivery_outcome(CONVERSATION_ID, output, Some(7));
        assert_eq!(outcome.disposition, AttemptDisposition::Failed);
        assert_eq!(outcome.error_code.as_deref(), Some("agent_artifact_verification_failed"));
        assert_eq!(outcome.error_retryable, Some(false));
        assert_eq!(outcome.tokens, Some(7));
    }

    #[test]
    fn lifecycle_success_without_a_public_result_cannot_complete_or_retry_an_attempt() {
        let output = AgentExecutionTurnOutput {
            canonical_operation_id: "operation".into(), terminal_event_id: Some("terminal".into()),
            delivery: AgentExecutionDelivery {
                message_id: CURRENT_USER_TURN_ID.into(), replayed: true, completed: true,
                turn_outcome: Some(nomifun_agent_contracts::AgentTurnOutcome::Unavailable),
                user_input_request: None, result_ok: Some(true),
                result_text: Some("Earlier progress is not a final result".into()),
                result_error: None, result_error_code: None, result_error_retryable: Some(false),
            }, output_files: vec![], integrity_ok: true, workspace_changed: false,
        };
        let outcome = completed_delivery_outcome(CONVERSATION_ID, output, None);
        assert_eq!(outcome.disposition, AttemptDisposition::Failed);
        assert!(!crate::scheduler::agent_outcome_can_complete(&outcome));
        assert_eq!(outcome.error_code.as_deref(), Some("agent_turn_result_unavailable"));
        assert_eq!(outcome.error_retryable, Some(false));
    }
}
