//! Long-horizon implementation of the single Nomi runtime contract.
//!
//! This adapter supplies adaptive execution/event projection to the shared engine
//! lifecycle SDK. Production composition supplies admitted Session/Broker/Kernel
//! ports; neither adapter nor SDK creates a second persistence/authority owner.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use nomifun_agent_runtime::{
    AgentEngine, AgentEngineError, AgentEngineEvent, AgentEventSink, AgentModelPort,
    AgentToolInvoker, AgentTurnRequest, AgentTurnTerminal, EngineBinding,
};
use nomifun_common::{AgentKillReason, AgentType, AppError, ConversationStatus, TimestampMs};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::protocol::events::{
    AgentStreamEvent, TextEventData, ThinkingEventData, ToolCallEventData, ToolCallStatus,
};
use crate::protocol::send_error::AgentSendError;
use crate::engine_sdk::{EngineProgress, EngineSessionDriver, EngineTurnOutcome, EngineTurnOutput, EngineTurnTerminal, HostedAgentRuntime};
use crate::types::{AgentRuntimeBuildOptions, SendMessageData};
use crate::{
    AgentRuntimeControl, OfficialAgentRuntime, RuntimeTeardown,
};

/// Per-Session, host-admitted ports. Implementations must validate the durable
/// root message, principal, snapshot, route and tool plan against the existing
/// Session owner. They must not infer authority from user/model JSON.
#[async_trait]
pub trait UnifiedRuntimeHost: Send + Sync {
    /// Recreate host-frozen transient resources at an accepted-Turn boundary
    /// through the registry's existing teardown/join boundary. A pristine
    /// warmup can be replaced before its first accepted input as well.
    /// This is not a request to replace an active or paused Turn.
    fn requires_turn_boundary_recycle(&self) -> bool { false }

    async fn recoverable_preparation_step(&self, _message:&SendMessageData) -> Result<Option<u16>,AppError> { Ok(None) }
    async fn suspend_after_cleanup_failure(&self, _message: &SendMessageData) -> Result<bool,AppError> { Ok(false) }
    fn supports_execution_checkpoints(&self) -> bool { false }
    async fn execution_pressure(&self, _message: &SendMessageData)
        -> Result<nomifun_agent_runtime::AgentExecutionPressure, AppError> {
        Ok(Default::default())
    }
    async fn save_execution_checkpoint(
        &self, _message: &SendMessageData, _checkpoint: &nomifun_agent_runtime::AgentExecutionCheckpoint,
    ) -> Result<Option<nomifun_agent_runtime::AgentCheckpointReceipt>, AppError> {
        Ok(None)
    }
    async fn queue_steer(&self, _delivery: crate::RuntimeSteerDelivery) -> Result<bool, AppError> {
        Err(AppError::BadRequest("Nomi host does not support receipt-bound steering".into()))
    }
    /// Return the same canonical active set used by tool admission. A host
    /// without a materialized Snapshot must explicitly return None.
    fn capability_activation_snapshot(
        &self,
    ) -> Result<Option<crate::AgentCapabilityActivationSnapshot>, AppError>;

    /// Assemble canonical history/context and claim the exact turn. The token
    /// fences all preparation, model and tool work for this logical turn.
    async fn prepare_turn(
        &self,
        message: &SendMessageData,
        cancellation: CancellationToken,
    ) -> Result<AgentTurnRequest, AppError>;

    /// Record engine semantics in the existing owner's event/history chain
    /// before broadcasting a UI projection. No separate engine rollout store.
    /// Return this exact canonical event's acknowledgement sequence when the
    /// host has one. Hosts without such a receipt return None, never a head.
    async fn record_event(
        &self,
        message: &SendMessageData,
        event: &AgentEngineEvent,
    ) -> Result<Option<u64>, AppError>;

    /// Optional display references after a canonical commit. Read failures do
    /// not confer execution authority and must not fail an otherwise valid Turn.
    async fn timeline_updates(
        &self, _message: &SendMessageData,
    ) -> Result<Vec<nomifun_agent_contracts::MessageTimelineUpdate>, AppError> {
        Ok(Vec::new())
    }

    /// Hosts accepting concurrent steering must override this and atomically
    /// check the inbox and persist admission. False must record no ToolStarted.
    async fn admit_tool(
        &self,
        message: &SendMessageData,
        event: &AgentEngineEvent,
    ) -> Result<bool, AppError> {
        if !matches!(event, AgentEngineEvent::ToolStarted { .. }) {
            return Err(AppError::BadRequest("tool admission requires ToolStarted".into()));
        }
        self.record_event(message, event).await?;
        Ok(true)
    }

    /// Required on every exit, including preparation failure and cancellation.
    /// Success proves turn-scoped tools/processes are quiescent.
    async fn cleanup_turn(&self, message: &SendMessageData) -> Result<(), AppError>;

    /// Required, idempotent proof that all Session-owned resources have exited.
    /// A failure retains the registry's teardown quarantine.
    async fn cleanup_session(&self) -> Result<(), AppError>;
}

fn contract_error(error: AgentEngineError) -> AppError {
    match error {
        error @ (AgentEngineError::Model { .. }
        | AgentEngineError::ModelStreamEndedWithoutTerminal
        | AgentEngineError::InvalidModelEvent(_)) => {
            AppError::BadGateway(format!("Nomi runtime: {error}"))
        }
        error @ (AgentEngineError::TurnAlreadyRunning | AgentEngineError::SessionDisposed) => {
            AppError::Conflict(format!("Nomi runtime: {error}"))
        }
        error => AppError::Internal(format!("Nomi runtime: {error}")),
    }
}

pub struct UnifiedAgentRuntime {
    runtime: HostedAgentRuntime,
    driver: Arc<UnifiedSessionDriver>,
}

struct UnifiedSessionDriver {
    owner_id: String,
    engine: Arc<AgentEngine>,
    binding: EngineBinding,
    model: Arc<dyn AgentModelPort>,
    tools: Arc<dyn AgentToolInvoker>,
    host: Arc<dyn UnifiedRuntimeHost>,
    started_turn: std::sync::atomic::AtomicBool,
    closed_terminal: std::sync::atomic::AtomicBool,
}

impl UnifiedAgentRuntime {
    /// The adaptive loop plugs into the same lifecycle SDK as source-integrated
    /// engines; only its execution/context policy and semantic codec differ.
    pub fn new(
        options: &AgentRuntimeBuildOptions,
        engine: Arc<AgentEngine>,
        binding: EngineBinding,
        model: Arc<dyn AgentModelPort>,
        tools: Arc<dyn AgentToolInvoker>,
        host: Arc<dyn UnifiedRuntimeHost>,
    ) -> Result<Self, AppError> {
        if binding.agent_session_id().as_ref() != options.conversation_id {
            return Err(AppError::Conflict("Nomi runtime Session binding mismatch".into()));
        }
        engine.open_session(binding.clone(), model.clone(), tools.clone(), None)
            .map_err(contract_error)?;
        let driver = Arc::new(UnifiedSessionDriver {
            owner_id: options.user_id.clone(), engine, binding, model, tools, host,
            started_turn: std::sync::atomic::AtomicBool::new(false),
            closed_terminal: std::sync::atomic::AtomicBool::new(false),
        });
        Ok(Self { runtime: HostedAgentRuntime::new(options, driver.clone())?, driver })
    }
}

struct PendingToolProjection {
    public: ToolCallEventData,
    arguments: serde_json::Value,
}

struct TurnProjection {
    host: Arc<dyn UnifiedRuntimeHost>,
    message: SendMessageData,
    output: EngineTurnOutput,
    calls: Mutex<BTreeMap<String, PendingToolProjection>>,
    terminal: Mutex<Option<AgentEngineEvent>>,
    last_model_step: std::sync::atomic::AtomicU16,
}

#[async_trait]
impl AgentEventSink for TurnProjection {
    fn supports_checkpoints(&self) -> bool { self.host.supports_execution_checkpoints() }
    async fn execution_pressure(&self) -> Result<nomifun_agent_runtime::AgentExecutionPressure, AgentEngineError> {
        self.host.execution_pressure(&self.message).await
            .map_err(|error| AgentEngineError::EventSink(error.to_string()))
    }
    async fn save_checkpoint(&self, checkpoint: nomifun_agent_runtime::AgentExecutionCheckpoint)
        -> Result<Option<nomifun_agent_runtime::AgentCheckpointReceipt>, AgentEngineError> {
        self.host.save_execution_checkpoint(&self.message, &checkpoint).await
            .map_err(|error| AgentEngineError::EventSink(error.to_string()))
    }
    async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError> {
        match &event {
            AgentEngineEvent::ModelStepStarted { step,.. } => self.last_model_step.store(*step,std::sync::atomic::Ordering::Release),
            AgentEngineEvent::ExecutionResumed { model_steps,.. } => self.last_model_step.store(*model_steps,std::sync::atomic::Ordering::Release),
            _ => {}
        }
        if matches!(
            event,
            AgentEngineEvent::TurnCompleted { .. }
                | AgentEngineEvent::TurnInputRequired { .. }
                | AgentEngineEvent::TurnCancelled { .. }
                | AgentEngineEvent::TurnPaused { .. }
                | AgentEngineEvent::TurnFailed { .. }
        ) {
            // Publication of a terminal must wait for proven tool/process exit.
            *self.terminal.lock().unwrap_or_else(|e| e.into_inner()) = Some(event);
            return Ok(());
        }
        let event_seq = self.host
            .record_event(&self.message, &event)
            .await
            .map_err(|error| AgentEngineError::InvalidContract(error.to_string()))?;
        match &event {
            AgentEngineEvent::ModelStepStarted { step, .. } => { self.output.record_model_steps(*step); }
            AgentEngineEvent::ExecutionResumed { model_steps, .. } => { self.output.record_model_steps(*model_steps); }
            _ => {}
        }
        self.project(event, event_seq)?;
        if event_seq.is_some() { self.publish_timeline().await; }
        Ok(())
    }

    async fn admit_tool(&self, event: AgentEngineEvent) -> Result<bool, AgentEngineError> {
        let admitted = self.host.admit_tool(&self.message, &event).await
            .map_err(|error| AgentEngineError::InvalidContract(error.to_string()))?;
        if admitted { self.project(event, None)?; self.publish_timeline().await; }
        Ok(admitted)
    }
}

impl TurnProjection {
    async fn publish_timeline(&self) {
        // Presentation cannot become an unbounded wait before an admitted
        // owner invocation. Dropping this read never drops execution evidence.
        match tokio::time::timeout(std::time::Duration::from_millis(100),
            self.host.timeline_updates(&self.message)).await {
            Ok(Ok(updates)) if !updates.is_empty() => {
                self.output.publish(EngineProgress::CanonicalTimeline(updates));
            }
            Ok(Ok(_)) | Err(_) => {},
            Ok(Err(error)) => tracing::warn!(%error, "canonical timeline display could not be refreshed"),
        }
    }

    fn project(&self, event: AgentEngineEvent, event_seq: Option<u64>) -> Result<(), AgentEngineError> {
        let projected = match event {
            // The host has committed the full plan. Re-read the canonical
            // snapshot, including lifecycle and reset state, for presentation.
            AgentEngineEvent::PlanUpdated { .. } | AgentEngineEvent::TurnWaiting { .. }
                | AgentEngineEvent::TurnWaitResolved { .. } => Some(EngineProgress::TaskPlanChanged),
            AgentEngineEvent::TurnStarted { .. } | AgentEngineEvent::ExecutionResumed { .. } => Some(EngineProgress::Started),
            AgentEngineEvent::OutputTextDelta { step, text } | AgentEngineEvent::CompletionDelivered { step, text } => {
                Some(EngineProgress::Text(TextEventData { content: text, step: Some(step) }))
            }
            AgentEngineEvent::ReasoningDelta { step, text } => {
                Some(EngineProgress::Thinking(ThinkingEventData {
                    content: text,
                    step: Some(step),
                    subject: None,
                    duration: None,
                    status: Some("thinking".into()),
                }))
            }
            AgentEngineEvent::ToolCallCompleted { call, .. } => {
                self.calls.lock().unwrap_or_else(|e| e.into_inner()).insert(
                    call.call_id.as_ref().to_owned(),
                    PendingToolProjection {
                        public: ToolCallEventData {
                            identity: Default::default(),
                            call_id: call.call_id.as_ref().to_owned(),
                            description: nomifun_engine_core::tool_display_target(None, None, &call.name, &call.arguments.0),
                            name: call.name,
                            status: ToolCallStatus::Running,
                            artifacts: Vec::new(),
                        },
                        arguments: call.arguments.0,
                    },
                );
                None
            }
            AgentEngineEvent::ModelOutputTruncated { discarded_tool_call_ids, .. }
            | AgentEngineEvent::ModelResponseRejected { discarded_tool_call_ids, .. }
            | AgentEngineEvent::VoiceModelStepSuperseded {discarded_tool_call_ids,..} => {
                let mut calls = self.calls.lock().unwrap_or_else(|e| e.into_inner());
                for id in discarded_tool_call_ids { calls.remove(id.as_ref()); }
                None
            }
            AgentEngineEvent::ToolStarted { call_id, capability_id, action_id, .. } => {
                let mut calls = self.calls.lock().unwrap_or_else(|e| e.into_inner());
                calls.get_mut(call_id.as_ref())
                    .filter(|call| !call.public.call_id.starts_with("agent-instructions:"))
                    .filter(|call| call.public.name != nomifun_agent_runtime::FINISH_WITHOUT_REPLY_TOOL_NAME)
                    .map(|pending| {
                        let call = &mut pending.public;
                        call.identity.capability_id = Some(capability_id.as_ref().to_owned());
                        call.identity.action_id = Some(action_id.as_ref().to_owned());
                        call.description = nomifun_engine_core::tool_display_target(
                            Some(capability_id.as_ref()), Some(action_id.as_ref()), &call.name, &pending.arguments);
                        call.identity.display.operation = nomifun_engine_core::tool_display_operation(Some(capability_id.as_ref()), &pending.arguments);
                        EngineProgress::ToolCall(call.clone())
                    })
            }
            AgentEngineEvent::ToolCompleted { result, runtime_hold, .. } => {
                let pending = self
                    .calls
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(result.call_id.as_ref())
                    .ok_or_else(|| {
                        AgentEngineError::InvalidModelEvent(
                            "tool result has no admitted call".to_owned(),
                        )
                    })?;
                let mut call = pending.public;
                // These are engine-owned instruction preflight reads, not
                // model-selected work. Keep their durable observations but do
                // not flood the conversation with synthetic read/error rows.
                if call.call_id.starts_with("agent-instructions:")
                    || call.name == nomifun_agent_runtime::FINISH_WITHOUT_REPLY_TOOL_NAME {
                    return Ok(());
                }
                call.status = if result.is_error {
                    ToolCallStatus::Error
                } else {
                    ToolCallStatus::Completed
                };
                call.identity.result_seq = event_seq;
                call.identity.runtime_hold = runtime_hold;
                call.identity.permission_required = nomifun_engine_core::project_system_permission_required(
                    call.identity.capability_id.as_deref(), call.identity.action_id.as_deref(),
                    result.is_error, &result.output_text(),
                );
                call.identity.error_code = call.identity.permission_required.as_ref()
                    .map(|_| nomifun_engine_core::SYSTEM_PERMISSION_REQUIRED.to_owned());
                let operation = call.identity.display.operation.take();
                call.identity.display = nomifun_engine_core::tool_result_display(
                    call.identity.capability_id.as_deref(), call.identity.action_id.as_deref(), &result);
                call.identity.display.operation = operation;
                if call.identity.runtime_hold.is_some() || call.identity.permission_required.is_some() { call.identity.display.error = None; }
                if call.identity.display.cancelled { call.status = ToolCallStatus::Canceled; }
                call.identity.display.denied_stop_conversation_id = nomifun_engine_core::tool_denied_stop_target(
                    &call.name, &pending.arguments, &result);
                if let Some(target) = nomifun_engine_core::tool_result_target(
                    call.identity.capability_id.as_deref(), call.identity.action_id.as_deref(), &result) {
                    call.description = Some(target);
                }
                Some(EngineProgress::ToolCall(call))
            }
            // Semantics are recorded above even when there is no UI projection.
            _ => None,
        };
        if let Some(event) = projected {
            self.output.publish(event);
        }
        Ok(())
    }
}

#[async_trait]
impl EngineSessionDriver for UnifiedSessionDriver {
    async fn run_turn(
        &self,
        message: &SendMessageData,
        cancellation: CancellationToken,
        output: EngineTurnOutput,
    ) -> Result<EngineTurnOutcome, AppError> {
        self.started_turn.store(true, std::sync::atomic::Ordering::Release);
        self.closed_terminal.store(false, std::sync::atomic::Ordering::Release);
        let projection = Arc::new(TurnProjection {
            host: self.host.clone(), message: message.clone(), output,
            calls: Mutex::new(BTreeMap::new()), terminal: Mutex::new(None),
            last_model_step: std::sync::atomic::AtomicU16::new(0),
        });
        let session = self.engine.open_session(self.binding.clone(), self.model.clone(),
            self.tools.clone(), Some(projection.clone())).map_err(contract_error)?;
        let request = match self.host.prepare_turn(message, cancellation.clone()).await {
            Ok(request) => request,
            Err(error) => {
                if let Some(model_steps) = self.host.recoverable_preparation_step(message).await? {
                    return Ok(EngineTurnOutcome { model_steps,terminal:EngineTurnTerminal::Paused { reason:"EXECUTION_PREPARATION_BLOCKED".into() } });
                }
                return Err(error);
            }
        };
        if request.principal.principal_kind != "user" || request.principal.principal_id != self.owner_id {
            return Err(AppError::Conflict("Nomi turn principal differs from its Session owner".into()));
        }
        let execution = session.run_turn_cancellable(request, cancellation).await;
        let pending = projection.terminal.lock().unwrap_or_else(|e| e.into_inner()).take();
        match execution {
            Ok(result) => {
                let outcome = EngineTurnOutcome {
                    model_steps: result.model_steps,
                    terminal: match result.terminal {
                        AgentTurnTerminal::Completed { finish_reason } => EngineTurnTerminal::Completed { finish_reason },
                        AgentTurnTerminal::InputRequired { request } => EngineTurnTerminal::InputRequired { request },
                        AgentTurnTerminal::Cancelled => EngineTurnTerminal::Cancelled,
                        AgentTurnTerminal::Paused { reason } => EngineTurnTerminal::Paused { reason },
                    },
                };
                if pending.as_ref() != Some(&runtime_terminal(&outcome)) {
                    return Err(AppError::Conflict("Nomi result and terminal event disagree or terminal is absent".into()));
                }
                Ok(outcome)
            }
            Err(AgentEngineError::Cancelled) => Ok(EngineTurnOutcome::cancelled(
                match pending { Some(AgentEngineEvent::TurnCancelled { model_steps }) => model_steps,
                    _ => projection.last_model_step.load(std::sync::atomic::Ordering::Acquire) })),
            Err(AgentEngineError::TurnFailed(message)) => {
                let model_steps = match pending {
                    Some(AgentEngineEvent::TurnFailed { model_steps, message: recorded, failure: None }) if recorded == message => model_steps,
                    _ => return Err(AppError::Conflict("Nomi failure has no matching terminal record".into())),
                };
                Ok(EngineTurnOutcome { model_steps, terminal: EngineTurnTerminal::Failed { message, failure: None } })
            }
            Err(AgentEngineError::WaitOwnerUnavailable) => Ok(EngineTurnOutcome {
                model_steps:projection.last_model_step.load(std::sync::atomic::Ordering::Acquire),
                terminal:EngineTurnTerminal::Paused {reason:"EXECUTION_WAIT_OWNER_UNAVAILABLE".into()},
            }),
            Err(error @ (AgentEngineError::Model { .. } | AgentEngineError::ModelStreamEndedWithoutTerminal | AgentEngineError::InvalidModelEvent(_))) => {
                // The model failure ends this Turn only after the existing
                // cleanup and canonical receipt barriers. Completed effects
                // stay recorded; the next user input is a new Turn.
                let (message, failure) = model_turn_failure(error);
                Ok(EngineTurnOutcome { model_steps:projection.last_model_step.load(std::sync::atomic::Ordering::Acquire),
                    terminal:EngineTurnTerminal::Failed { message, failure: Some(failure) } })
            }
            Err(error) => Err(contract_error(error)),
        }
    }

    fn capability_activation_snapshot(&self) -> Result<Option<crate::AgentCapabilityActivationSnapshot>, AppError> {
        self.host.capability_activation_snapshot()
    }
    fn supports_steering_context(&self) -> bool { true }
    async fn queue_steer(&self, delivery: crate::RuntimeSteerDelivery) -> Result<bool, AppError> {
        self.host.queue_steer(delivery).await
    }
    async fn cleanup_turn(&self, message: &SendMessageData) -> Result<(), AppError> {
        self.host.cleanup_turn(message).await
    }
    async fn suspend_after_cleanup_failure(&self, message: &SendMessageData) -> Result<bool,AppError> {
        self.closed_terminal.store(false, std::sync::atomic::Ordering::Release);
        self.host.suspend_after_cleanup_failure(message).await
    }
    async fn record_terminal(&self, message: &SendMessageData, outcome: &EngineTurnOutcome) -> Result<(), AppError> {
        self.host.record_event(message, &runtime_terminal(outcome)).await?;
        // SDK Finished also represents a resumable pause. Only a successfully
        // committed closed terminal permits replacing this local driver.
        // InputRequired closes this Turn and hands the next input to its own
        // canonical admission; it does not retain a paused execution to resume.
        self.closed_terminal.store(!matches!(outcome.terminal, EngineTurnTerminal::Paused { .. }),
            std::sync::atomic::Ordering::Release);
        Ok(())
    }
    async fn cleanup_session(&self) -> Result<(), AppError> { self.host.cleanup_session().await }
}

fn model_turn_failure(error: AgentEngineError) -> (String, nomifun_agent_runtime::AgentTurnFailure) {
    use nomifun_agent_runtime::AgentTurnFailure;
    match error {
        AgentEngineError::Model { code, message, diagnostic } => (message, AgentTurnFailure::Model { code, diagnostic }),
        AgentEngineError::ModelStreamEndedWithoutTerminal => (
            "Model stream ended without a terminal event".into(), AgentTurnFailure::ModelStreamEndedWithoutTerminal,
        ),
        AgentEngineError::InvalidModelEvent(message) => (message, AgentTurnFailure::InvalidModelEvent),
        _ => unreachable!("only typed model failures enter model Turn settlement"),
    }
}

fn runtime_terminal(outcome: &EngineTurnOutcome) -> AgentEngineEvent {
    match &outcome.terminal {
        EngineTurnTerminal::Completed { finish_reason } => AgentEngineEvent::TurnCompleted {
            model_steps: outcome.model_steps, finish_reason: finish_reason.clone(),
        },
        EngineTurnTerminal::InputRequired { request } => AgentEngineEvent::TurnInputRequired {
            model_steps: outcome.model_steps, request: request.clone(),
        },
        EngineTurnTerminal::Cancelled => AgentEngineEvent::TurnCancelled { model_steps: outcome.model_steps },
        EngineTurnTerminal::Paused { reason } => AgentEngineEvent::TurnPaused { model_steps: outcome.model_steps, reason: reason.clone() },
        EngineTurnTerminal::Failed { message, failure } => AgentEngineEvent::TurnFailed {
            model_steps: outcome.model_steps, message: message.clone(), failure: failure.clone(),
        },
    }
}

#[async_trait]
impl AgentRuntimeControl for UnifiedAgentRuntime {
    fn agent_type(&self) -> AgentType { self.runtime.agent_type() }
    fn conversation_id(&self) -> &str { self.runtime.conversation_id() }
    fn workspace(&self) -> &str { self.runtime.workspace() }
    fn status(&self) -> Option<ConversationStatus> { self.runtime.status() }
    fn is_transport_healthy(&self) -> bool { self.runtime.is_transport_healthy() }
    fn last_activity_at(&self) -> TimestampMs { self.runtime.last_activity_at() }
    fn touch_activity(&self) { self.runtime.touch_activity(); }
    fn capability_activation_snapshot(&self) -> Result<Option<crate::AgentCapabilityActivationSnapshot>, AppError> {
        self.runtime.capability_activation_snapshot()
    }
    fn subscribe(&self) -> broadcast::Receiver<AgentStreamEvent> { self.runtime.subscribe() }
    async fn send_message(&self, message: SendMessageData) -> Result<(), AgentSendError> { self.runtime.send_message(message).await }
    async fn cancel(&self) -> Result<(), AppError> { self.runtime.cancel().await }
    fn kill(&self, reason: Option<AgentKillReason>) -> Result<(), AppError> { self.runtime.kill(reason) }
}

#[async_trait]
impl OfficialAgentRuntime for UnifiedAgentRuntime {
    fn requires_turn_boundary_recycle(&self) -> bool {
        let replaceable = match self.runtime.status() {
            None => !self.driver.started_turn.load(std::sync::atomic::Ordering::Acquire),
            Some(ConversationStatus::Finished) => self.driver.closed_terminal.load(std::sync::atomic::Ordering::Acquire),
            _ => false,
        };
        replaceable && self.driver.host.requires_turn_boundary_recycle()
    }
    fn supports_steering_context(&self) -> bool { self.runtime.supports_steering_context() }
    async fn steer_with_receipt(&self, delivery: crate::RuntimeSteerDelivery) -> Result<bool, AppError> {
        self.runtime.steer_with_receipt(delivery).await
    }
    fn kill_and_wait(&self, reason: Option<AgentKillReason>) -> RuntimeTeardown { self.runtime.kill_and_wait(reason) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_state::AgentRuntimeState;
    use crate::protocol::events::TurnStopReason;
    use nomifun_chat_model_broker::ChatFinishReason;
    use crate::runtime_sessions::OfficialRuntimeOpener;
    use crate::{
        AgentRuntimeHandle, AgentRuntimeSessions, InMemoryAgentRuntimeSessions,
    };
    use nomifun_agent_contracts::{
        AgentSessionId, ChatRouteIdentity, DigestHex, EventId, ModelRouteId, OperationId,
        PrincipalRef, ResolvedSnapshotId, ResolvedSnapshotRef, RuntimeBindingId, VersionString,
    };
    use nomifun_chat_model_broker::{
        ChatCausality, ChatContentPart, ChatMessage, ChatModelError, ChatModelEvent,
        ChatModelInput, ChatModelRequest, ChatResponseFormat, ChatRole, ChatToolChoice,
        PromptCachePolicy,
    };
    use nomifun_agent_runtime::{
        AgentEngineBuild, AgentModelStream, AgentToolInvocation, AgentToolPlan,
        AgentToolResult, EngineBuildId,
    };
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    const OWNER: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const SESSION: &str = "0190f5fe-7c00-7a00-8000-000000000002";

    #[test]
    fn model_failure_preserves_typed_cause_and_original_diagnostic() {
        use nomifun_chat_model_broker::ChatModelErrorCode;
        use nomifun_agent_runtime::AgentTurnFailure;
        assert_eq!(model_turn_failure(AgentEngineError::Model {
            code: ChatModelErrorCode::AuthenticationFailed, message: "private provider body".into(),
            diagnostic: None,
        }), ("private provider body".into(), AgentTurnFailure::Model { code: ChatModelErrorCode::AuthenticationFailed, diagnostic: None }));
        assert_eq!(model_turn_failure(AgentEngineError::Model {
            code: ChatModelErrorCode::InvalidRequest, message: "private request".into(),
            diagnostic: None,
        }), ("private request".into(), AgentTurnFailure::Model { code: ChatModelErrorCode::InvalidRequest, diagnostic: None }));
        assert_eq!(model_turn_failure(AgentEngineError::InvalidModelEvent("raw output".into())),
            ("raw output".into(), AgentTurnFailure::InvalidModelEvent));
        let mut diagnostic = nomifun_agent_contracts::ModelFailureDiagnostic::new(
            nomifun_agent_contracts::ModelFailureReason::ConnectionFailed);
        diagnostic.transport_detail = Some("connection refused (os error 61)".into());
        assert_eq!(model_turn_failure(AgentEngineError::Model {
            code:ChatModelErrorCode::ProviderUnavailable, message:"Safe fixed failure".into(), diagnostic:Some(diagnostic.clone()),
        }), ("Safe fixed failure".into(), AgentTurnFailure::Model {code:ChatModelErrorCode::ProviderUnavailable, diagnostic:Some(diagnostic)}));
    }

    #[test]
    fn nomi_context_overflow_is_not_misclassified_as_an_unknown_upstream_conflict() {
        let error = contract_error(AgentEngineError::ContextTooLarge {
            limit: 8192,
            actual: 8193,
        });
        assert!(matches!(error, AppError::Internal(message)
            if message == "Nomi runtime: Nomi context is 8193 bytes, above the 8192 byte limit"));
    }

    #[test]
    fn local_runtime_context_failure_keeps_nomifun_ownership() {
        assert!(matches!(
            contract_error(AgentEngineError::WorkspaceContext("instruction scope unavailable".into())),
            AppError::Internal(message) if message.contains("instruction scope unavailable")
        ));
        assert!(matches!(
            contract_error(AgentEngineError::InvalidModelEvent("bad provider frame".into())),
            AppError::BadGateway(message) if message.contains("bad provider frame")
        ));
    }

    fn options() -> AgentRuntimeBuildOptions {
        static WORKSPACE: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        let workspace = WORKSPACE
            .get_or_init(|| tempfile::tempdir().unwrap())
            .path();
        AgentRuntimeBuildOptions {
            user_id: OWNER.to_owned(),
            agent_type: AgentType::Nomi,
            workspace: workspace.to_string_lossy().into_owned(),
            model: None,
            conversation_id: SESSION.to_owned(),
            delegation_policy: Default::default(),
            extra: serde_json::json!({}),
            conversation_created_at: None,
            workspace_binding_lease: Some(
                nomifun_knowledge::WorkspaceBindingLease::acquire_unbound(workspace, SESSION)
                    .unwrap(),
            ),
        }
    }

    fn message() -> SendMessageData {
        SendMessageData {
            content: "hello".to_owned(),
            msg_id: "message".to_owned(),
            source_message_id: Some("root".to_owned()),
            files: Vec::new(),
            inject_skills: Vec::new(),
            origin: None,
        }
    }

    fn engine() -> Arc<AgentEngine> {
        Arc::new(
            AgentEngine::new(AgentEngineBuild {
                build_id: EngineBuildId::from("test-build"),
                build_digest: DigestHex::from("a".repeat(64)),
            })
            .unwrap(),
        )
    }

    fn binding() -> EngineBinding {
        engine()
            .bind(
                AgentSessionId::from(SESSION),
                RuntimeBindingId::from("binding"),
                ResolvedSnapshotRef {
                    snapshot_id: ResolvedSnapshotId::from("snapshot"),
                    snapshot_digest: DigestHex::from("b".repeat(64)),
                },
            )
            .unwrap()
    }

    struct Host {
        recycle_completed_turns: AtomicBool,
        prepared_inputs: Mutex<Vec<String>>,
        allow_user_input: AtomicBool,
        fail_terminal_record: AtomicBool,
        fail_record: AtomicBool,
        fail_timeline_read: AtomicBool,
        timeline_reads: AtomicUsize,
        events: Mutex<Vec<AgentEngineEvent>>,
        cleanup_turns: AtomicUsize,
        cleanup_sessions: AtomicUsize,
        fail_cleanup: AtomicBool,
        pending_preparation: AtomicBool,
        prepare_failure_at: AtomicUsize,
        block_cleanup: AtomicBool,
        cleanup_entered: tokio::sync::Notify,
        cleanup_release: tokio::sync::Semaphore,
        entered: tokio::sync::Notify,
    }

    impl Host {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                recycle_completed_turns: AtomicBool::new(false),
                prepared_inputs: Mutex::new(Vec::new()),
                allow_user_input: AtomicBool::new(false),
                fail_terminal_record: AtomicBool::new(false),
                fail_record: AtomicBool::new(false),
                fail_timeline_read: AtomicBool::new(false),
                timeline_reads: AtomicUsize::new(0),
                events: Mutex::new(Vec::new()),
                cleanup_turns: AtomicUsize::new(0),
                cleanup_sessions: AtomicUsize::new(0),
                fail_cleanup: AtomicBool::new(false),
                pending_preparation: AtomicBool::new(false),
                prepare_failure_at: AtomicUsize::new(usize::MAX),
                entered: tokio::sync::Notify::new(),
                block_cleanup: AtomicBool::new(false),
                cleanup_entered: tokio::sync::Notify::new(),
                cleanup_release: tokio::sync::Semaphore::new(0),
            })
        }
    }

    #[async_trait]
    impl UnifiedRuntimeHost for Host {
        fn requires_turn_boundary_recycle(&self) -> bool {
            self.recycle_completed_turns.load(Ordering::Acquire)
        }
        fn capability_activation_snapshot(
            &self,
        ) -> Result<Option<crate::AgentCapabilityActivationSnapshot>, AppError> {
            Ok(None)
        }

        async fn prepare_turn(
            &self,
            message: &SendMessageData,
            _cancellation: CancellationToken,
        ) -> Result<AgentTurnRequest, AppError> {
            self.prepared_inputs.lock().unwrap().push(message.content.clone());
            self.entered.notify_one();
            if self.pending_preparation.load(Ordering::Acquire) {
                return std::future::pending().await;
            }
            // Model the production host's awaited receipt, recovery, model,
            // attachment, history and context preparation boundaries.
            for stage in 0..6 {
                tokio::task::yield_now().await;
                if self.prepare_failure_at.load(Ordering::Acquire) == stage {
                    return Err(AppError::Conflict(format!(
                        "pre-active preparation stage {stage} failed"
                    )));
                }
            }
            let route =
                ChatRouteIdentity::new("preset@1", "agent_chat", ModelRouteId::from("route"), 1);
            let request = AgentTurnRequest::new(
                ChatModelRequest {
                    contract_version: VersionString::from(
                        nomifun_chat_model_broker::CHAT_MODEL_CONTRACT_VERSION,
                    ),
                    route: route.clone(),
                    causality: ChatCausality {
                        agent_session_id: AgentSessionId::from(SESSION),
                        turn_operation_id: OperationId::from(message.msg_id.clone()),
                        causation_event_id: EventId::from(
                            message.source_message_id.clone().unwrap(),
                        ),
                        resolved_snapshot_ref: binding().resolved_snapshot_ref().clone(),
                        route_identity: route,
                        operation_id: OperationId::from("model"),
                    },
                    input: ChatModelInput {
                        instructions: vec!["test".to_owned()],
                        messages: vec![ChatMessage {
                            role: ChatRole::User,
                            content: vec![ChatContentPart::Text {
                                text: message.content.clone(),
                            }],
                            provider_round_id: None,
                        }],
                        tools: Vec::new(),
                        tool_choice: ChatToolChoice::None,
                        parallel_tool_calls: None,
                        max_output_tokens: Some(100),
                        reasoning: None,
                        prompt_cache: PromptCachePolicy::Disabled,
                        response_format: ChatResponseFormat::Text,
                        requested_output_modalities: BTreeSet::new(),
                        provider_round_parent: None,
                        preserve_native_responses_items: false,
                        metadata: Default::default(),
                    },
                },
                AgentToolPlan::default(),
                PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: OWNER.to_owned(),
                },
                1,
            );
            Ok(if self.allow_user_input.load(Ordering::Acquire) {
                request.with_user_input_request_port(Arc::new(AllowUserInput))
            } else { request })
        }
        async fn record_event(
            &self,
            _message: &SendMessageData,
            event: &AgentEngineEvent,
        ) -> Result<Option<u64>, AppError> {
            if self.fail_record.load(Ordering::Acquire) {
                return Err(AppError::Conflict("canonical journal unavailable".into()));
            }
            if matches!(
                event,
                AgentEngineEvent::TurnCompleted { .. }
                    | AgentEngineEvent::TurnInputRequired { .. }
                    | AgentEngineEvent::TurnCancelled { .. }
                    | AgentEngineEvent::TurnFailed { .. }
            ) {
                assert!(
                    self.cleanup_turns.load(Ordering::Acquire) > 0,
                    "terminal must follow cleanup proof"
                );
                if self.fail_terminal_record.load(Ordering::Acquire) {
                    return Err(AppError::Conflict("terminal receipt unavailable".into()));
                }
            }
            let mut events = self.events.lock().unwrap();
            events.push(event.clone());
            Ok(Some(events.len() as u64))
        }
        async fn timeline_updates(&self, _: &SendMessageData)
            -> Result<Vec<nomifun_agent_contracts::MessageTimelineUpdate>, AppError> {
            self.timeline_reads.fetch_add(1, Ordering::SeqCst);
            if self.fail_timeline_read.load(Ordering::Acquire) {
                return Err(AppError::Conflict("presentation read unavailable".into()));
            }
            Ok(Vec::new())
        }
        async fn cleanup_turn(&self, _message: &SendMessageData) -> Result<(), AppError> {
            if self.block_cleanup.load(Ordering::Acquire) {
                self.cleanup_entered.notify_one();
                self.cleanup_release.acquire().await.unwrap().forget();
            }
            self.cleanup_turns.fetch_add(1, Ordering::SeqCst);
            if self.fail_cleanup.load(Ordering::Acquire) {
                return Err(AppError::Conflict("process has not exited".to_owned()));
            }
            Ok(())
        }
        async fn cleanup_session(&self) -> Result<(), AppError> {
            self.cleanup_sessions.fetch_add(1, Ordering::SeqCst);
            if self.fail_cleanup.load(Ordering::Acquire) {
                return Err(AppError::Conflict("process has not exited".to_owned()));
            }
            Ok(())
        }
    }

    struct Model {
        pending: bool,
        panics: bool,
        opened: tokio::sync::Notify,
    }
    #[async_trait]
    impl AgentModelPort for Model {
        async fn open_stream(
            &self,
            _request: ChatModelRequest,
            _cancellation: CancellationToken,
        ) -> Result<AgentModelStream, ChatModelError> {
            self.opened.notify_one();
            assert!(!self.panics, "provider panic fixture");
            if self.pending {
                return std::future::pending().await;
            }
            Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ChatModelEvent::OutputTextDelta {
                    text: "reply".to_owned(),
                }),
                Ok(ChatModelEvent::Completed {
                    finish_reason: ChatFinishReason::Completed,
                }),
            ])))
        }
    }

    struct NoTools;
    #[async_trait]
    impl AgentToolInvoker for NoTools {
        async fn invoke(
            &self,
            _request: AgentToolInvocation,
            _cancel: CancellationToken,
        ) -> Result<AgentToolResult, AgentEngineError> {
            panic!("an empty admitted plan must not invoke tools")
        }
    }

    #[derive(Debug)]
    struct AllowUserInput;
    #[async_trait]
    impl nomifun_agent_runtime::AgentUserInputRequestPort for AllowUserInput {
        async fn check(&self, causality: &ChatCausality, request: &nomifun_agent_contracts::AgentUserInputRequest)
            -> Result<Option<String>, AgentEngineError> {
            assert_eq!(causality.agent_session_id.as_ref(), SESSION);
            assert_eq!(request.request_id.as_ref(), "ask-report-path");
            Ok(None)
        }
    }

    struct InputRequestModel;
    #[async_trait]
    impl AgentModelPort for InputRequestModel {
        async fn open_stream(&self, request: ChatModelRequest, _: CancellationToken)
            -> Result<AgentModelStream, ChatModelError> {
            assert!(request.input.tools.iter().any(|tool| tool.name == "request_user_input"));
            Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ChatModelEvent::ToolCallCompleted { call: nomifun_chat_model_broker::ChatToolCall {
                    call_id: "ask-report-path".into(), name: "request_user_input".into(),
                    arguments: nomifun_agent_contracts::StrictJsonValue(serde_json::json!({
                        "question":"Where should the report be saved?", "remaining_work":"Save and verify the report."
                    })), provider_metadata: None,
                }}),
                Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::ToolCalls }),
            ])))
        }
    }

    fn input_request_runtime(host: Arc<Host>) -> UnifiedAgentRuntime {
        host.allow_user_input.store(true, Ordering::Release);
        static WORKSPACE: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        let workspace = WORKSPACE.get_or_init(|| tempfile::tempdir().unwrap());
        // This SDK-only host has no filesystem or Knowledge owner to lease.
        let options = AgentRuntimeBuildOptions {
            user_id:OWNER.into(), agent_type:AgentType::Nomi,
            workspace:workspace.path().to_string_lossy().into_owned(), conversation_id:SESSION.into(),
            model:None, delegation_policy:Default::default(), extra:serde_json::json!({}),
            conversation_created_at:None, workspace_binding_lease:None,
        };
        UnifiedAgentRuntime::new(&options, engine(), binding(), Arc::new(InputRequestModel), Arc::new(NoTools), host).unwrap()
    }

    async fn input_request_events(events: &mut broadcast::Receiver<AgentStreamEvent>) -> Vec<AgentStreamEvent> {
        tokio::time::timeout(Duration::from_secs(3), async {
            let mut received = Vec::new();
            loop {
                let event = events.recv().await.unwrap();
                let terminal = matches!(event, AgentStreamEvent::Finish(_) | AgentStreamEvent::Error(_));
                received.push(event);
                if terminal { return received; }
            }
        }).await.unwrap()
    }

    #[tokio::test]
    async fn input_required_waits_for_cleanup_and_receipt_then_publishes_note_before_finish_without_success_gate() {
        let host = Host::new();
        host.recycle_completed_turns.store(true, Ordering::Release);
        host.block_cleanup.store(true, Ordering::Release);
        let runtime = input_request_runtime(host.clone());
        let mut events = runtime.subscribe();
        runtime.send_message(message()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), host.cleanup_entered.notified()).await.unwrap();
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(&event, AgentStreamEvent::Tips(data) if data.user_input_request.is_some()));
            assert!(!matches!(event, AgentStreamEvent::Finish(_) | AgentStreamEvent::Error(_)));
        }
        assert_eq!(runtime.status(), Some(ConversationStatus::Running));
        assert!(!runtime.requires_turn_boundary_recycle(), "cleanup still owns the original Turn");
        assert!(runtime.send_message(message()).await.is_err(), "cleanup retains original ownership");
        assert!(!host.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::TurnInputRequired { .. })));
        host.cleanup_release.add_permits(1);
        let received = input_request_events(&mut events).await;
        let tip = received.iter().position(|event| matches!(event, AgentStreamEvent::Tips(data) if data.user_input_request.is_some())).unwrap();
        let finish = received.iter().position(|event| matches!(event, AgentStreamEvent::Finish(data) if data.stop_reason == Some(TurnStopReason::InputRequired))).unwrap();
        assert!(tip < finish);
        let AgentStreamEvent::Tips(data) = &received[tip] else { unreachable!() };
        let request = data.user_input_request.as_ref().unwrap();
        assert_eq!(request.question, "Where should the report be saved?");
        let recorded = host.events.lock().unwrap();
        assert_eq!(recorded.iter().filter(|event| matches!(event, AgentEngineEvent::TurnInputRequired { .. })).count(), 1);
        assert!(!recorded.iter().any(|event| matches!(event, AgentEngineEvent::TurnCompleted { .. } | AgentEngineEvent::CompletionReview { .. })));
        drop(recorded);
        assert_eq!(runtime.status(), Some(ConversationStatus::Finished));
        assert!(runtime.requires_turn_boundary_recycle(), "recorded InputRequired closes this Turn before the next input admission");
        runtime.kill_and_wait(None).await.unwrap();
    }

    #[tokio::test]
    async fn input_required_cleanup_or_terminal_record_failure_never_publishes_the_question_or_releases_input() {
        for fail_cleanup in [true, false] {
            let host = Host::new();
            host.recycle_completed_turns.store(true, Ordering::Release);
            host.fail_cleanup.store(fail_cleanup, Ordering::Release);
            host.fail_terminal_record.store(!fail_cleanup, Ordering::Release);
            let runtime = input_request_runtime(host.clone());
            let mut events = runtime.subscribe();
            runtime.send_message(message()).await.unwrap();
            let received = input_request_events(&mut events).await;
            assert!(matches!(received.last(), Some(AgentStreamEvent::Error(_))));
            assert!(!received.iter().any(|event| matches!(event, AgentStreamEvent::Tips(data) if data.user_input_request.is_some())));
            assert!(!host.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::TurnInputRequired { .. })));
            assert!(!runtime.is_transport_healthy());
            assert!(!runtime.requires_turn_boundary_recycle(), "unconfirmed cleanup or terminal receipt cannot permit a normal boundary recycle");
            assert!(runtime.send_message(message()).await.is_err());
            host.fail_cleanup.store(false, Ordering::Release);
            host.fail_terminal_record.store(false, Ordering::Release);
            runtime.kill_and_wait(None).await.unwrap();
        }
    }

    #[tokio::test]
    async fn input_required_cancel_during_cleanup_records_cancelled_without_a_question_note() {
        let host = Host::new();
        host.block_cleanup.store(true, Ordering::Release);
        let runtime = input_request_runtime(host.clone());
        let mut events = runtime.subscribe();
        runtime.send_message(message()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), host.cleanup_entered.notified()).await.unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(20), runtime.cancel()).await.is_err());
        host.cleanup_release.add_permits(1);
        let received = input_request_events(&mut events).await;
        assert!(matches!(received.last(), Some(AgentStreamEvent::Finish(data)) if data.stop_reason == Some(TurnStopReason::Cancelled)));
        assert!(!received.iter().any(|event| matches!(event, AgentStreamEvent::Tips(data) if data.user_input_request.is_some())));
        let recorded = host.events.lock().unwrap();
        assert!(recorded.iter().any(|event| matches!(event, AgentEngineEvent::TurnCancelled { .. })));
        assert!(!recorded.iter().any(|event| matches!(event, AgentEngineEvent::TurnInputRequired { .. } | AgentEngineEvent::TurnCompleted { .. })));
        drop(recorded);
        assert!(runtime.is_transport_healthy());
        runtime.kill_and_wait(None).await.unwrap();
    }

    fn runtime(host: Arc<Host>, model: Arc<Model>) -> UnifiedAgentRuntime {
        UnifiedAgentRuntime::new(
            &options(),
            engine(),
            binding(),
            model,
            Arc::new(NoTools),
            host,
        )
        .unwrap()
    }

    fn model(pending: bool, panics: bool) -> Arc<Model> {
        Arc::new(Model {
            pending,
            panics,
            opened: tokio::sync::Notify::new(),
        })
    }

    async fn terminal(events: &mut broadcast::Receiver<AgentStreamEvent>) -> AgentStreamEvent {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let event = events.recv().await.unwrap();
                if matches!(
                    event,
                    AgentStreamEvent::Finish(_) | AgentStreamEvent::Error(_)
                ) {
                    return event;
                }
            }
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn gateway_model_failure_records_typed_failed_turn_after_cleanup() {
        use nomifun_agent_contracts::{ModelFailureDiagnostic, ModelFailureReason};
        use nomifun_chat_model_broker::{ChatModelErrorCode, ChatRetryDirective};
        struct GatewayFailureModel { code: ChatModelErrorCode, reason: ModelFailureReason }
        #[async_trait]
        impl AgentModelPort for GatewayFailureModel {
            async fn open_stream(&self, _: ChatModelRequest, _: CancellationToken) -> Result<AgentModelStream, ChatModelError> {
                let mut error = ChatModelError::new(self.code, "Safe fixed model failure", ChatRetryDirective::Never);
                error.diagnostic = Some(ModelFailureDiagnostic::new(self.reason));
                Err(error)
            }
        }
        for (reason, code, agent_code) in [
            (ModelFailureReason::InsufficientBalance, ChatModelErrorCode::ProviderUnavailable, "USER_LLM_PROVIDER_BILLING_REQUIRED"),
            (ModelFailureReason::SubscriptionExpired, ChatModelErrorCode::ProviderUnavailable, "USER_LLM_PROVIDER_BILLING_REQUIRED"),
            (ModelFailureReason::ModelNotInPlan, ChatModelErrorCode::ProviderUnavailable, "USER_LLM_PROVIDER_BILLING_REQUIRED"),
            (ModelFailureReason::ExpiredKey, ChatModelErrorCode::AuthenticationFailed, "USER_LLM_PROVIDER_AUTH_FAILED"),
            (ModelFailureReason::RateLimited, ChatModelErrorCode::RateLimited, "USER_LLM_PROVIDER_RATE_LIMITED"),
        ] {
            let host = Host::new();
            let runtime = UnifiedAgentRuntime::new(&options(), engine(), binding(),
                Arc::new(GatewayFailureModel { code, reason }), Arc::new(NoTools), host.clone()).unwrap();
            let mut events = runtime.subscribe();
            runtime.send_message(message()).await.unwrap();
            let failure = terminal(&mut events).await;
            let AgentStreamEvent::Error(failure) = failure else { panic!("a model error must end as a failed Turn"); };
            assert_eq!(serde_json::to_value(failure.code).unwrap(), agent_code);
            assert_eq!(failure.provider_diagnostic.as_ref().unwrap().reason, reason);
            assert!(failure.detail.is_none());
            assert_eq!(failure.retryable, Some(false));
            assert_eq!(host.cleanup_turns.load(Ordering::Acquire), 1, "failure follows cleanup");
            assert!(host.events.lock().unwrap().iter().any(|event| matches!(event,
                AgentEngineEvent::TurnFailed { failure: Some(nomifun_agent_runtime::AgentTurnFailure::Model { code: recorded, .. }), .. }
                    if recorded == &code)));
            assert!(!host.events.lock().unwrap().iter().any(|event| matches!(event, AgentEngineEvent::TurnPaused { .. })));
            assert_eq!(runtime.status(), Some(ConversationStatus::Finished));
            assert!(runtime.is_transport_healthy());
            runtime.kill_and_wait(None).await.unwrap();
        }
    }

    #[tokio::test]
    async fn interrupted_model_response_keeps_partial_output_and_finishes_the_failed_turn() {
        struct InterruptedModel;
        #[async_trait]
        impl AgentModelPort for InterruptedModel {
            async fn open_stream(&self, _: ChatModelRequest, _: CancellationToken) -> Result<AgentModelStream, ChatModelError> {
                Ok(Box::pin(futures_util::stream::iter(vec![
                    Ok(ChatModelEvent::OutputTextDelta { text: "Completed work remains available.".into() }),
                    Err(ChatModelError::new(nomifun_chat_model_broker::ChatModelErrorCode::ProviderUnavailable,
                        "original temporary provider failure", nomifun_chat_model_broker::ChatRetryDirective::Never)),
                ])))
            }
        }
        let host = Host::new();
        let runtime = UnifiedAgentRuntime::new(&options(), engine(), binding(),
            Arc::new(InterruptedModel), Arc::new(NoTools), host.clone()).unwrap();
        let mut events = runtime.subscribe();
        runtime.send_message(message()).await.unwrap();
        let AgentStreamEvent::Error(error) = terminal(&mut events).await else { panic!("terminal failure expected"); };
        assert_eq!(error.code, Some(nomifun_api_types::AgentErrorCode::UserLlmProviderUnavailable));
        assert!(error.detail.is_none(), "opaque provider prose is not an approved public diagnostic");
        assert!(!error.message.contains("original temporary provider failure"));
        let recorded = host.events.lock().unwrap();
        assert!(recorded.iter().any(|event| matches!(event, AgentEngineEvent::OutputTextDelta { text, .. }
            if text == "Completed work remains available.")));
        assert!(recorded.iter().any(|event| matches!(event, AgentEngineEvent::TurnFailed { model_steps: 1, .. })));
        assert!(!recorded.iter().any(|event| matches!(event, AgentEngineEvent::TurnPaused { .. })));
        drop(recorded);
        assert_eq!(runtime.status(), Some(ConversationStatus::Finished));
        runtime.kill_and_wait(None).await.unwrap();
    }

    #[tokio::test]
    async fn timeline_read_failure_does_not_change_tool_admission_results_or_turn_status() {
        let host = Host::new();
        host.fail_timeline_read.store(true, Ordering::Release);
        let state = AgentRuntimeState::new(SESSION, "projection-workspace", 32);
        let turn = state.reset_for_new_turn(ConversationStatus::Running);
        let projection = TurnProjection {
            host: host.clone(), message: message(), output: EngineTurnOutput::new(state.clone(), turn),
            calls: Mutex::new(BTreeMap::new()), terminal: Mutex::new(None),
            last_model_step: std::sync::atomic::AtomicU16::new(0),
        };
        let mut events = state.subscribe();
        let proposed = AgentEngineEvent::ToolCallCompleted { step: 1,
            call: nomifun_chat_model_broker::ChatToolCall { call_id: "read".into(), name: "read_file".into(),
                arguments: nomifun_agent_contracts::StrictJsonValue(serde_json::json!({"path":"a"})), provider_metadata: None } };
        projection.emit(proposed.clone()).await.unwrap();
        let admitted = AgentEngineEvent::ToolStarted { step: 1, call_id: "read".into(),
            capability_id: "workspace.files".into(), action_id: "workspace.files/read".into() };
        assert!(projection.admit_tool(admitted.clone()).await.unwrap());
        let completed = AgentEngineEvent::ToolCompleted { step: 1, runtime_hold: None,
            result: AgentToolResult::text("read".into(), "owner observation", false) };
        projection.emit(completed.clone()).await.unwrap();
        assert_eq!(host.events.lock().unwrap().as_slice(), &[proposed, admitted, completed]);
        assert_eq!(host.timeline_reads.load(Ordering::Acquire), 3);
        assert!(matches!(events.try_recv().unwrap(), AgentStreamEvent::ToolCall(data) if data.status == ToolCallStatus::Running));
        assert!(matches!(events.try_recv().unwrap(), AgentStreamEvent::ToolCall(data) if data.status == ToolCallStatus::Completed));
        assert!(events.try_recv().is_err());
        assert!(projection.terminal.lock().unwrap().is_none());
        assert_eq!(state.status(), Some(ConversationStatus::Running));

        host.fail_record.store(true, Ordering::Release);
        assert!(projection.emit(AgentEngineEvent::ReasoningDelta { step: 2, text: "uncommitted".into() }).await.is_err());
        assert_eq!(host.timeline_reads.load(Ordering::Acquire), 3, "failed canonical writes cannot trigger metadata publication");
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn silent_robot_finish_remains_canonical_without_a_user_visible_control_row() {
        let host = Host::new();
        let state = AgentRuntimeState::new(SESSION, "silent-projection-workspace", 32);
        let turn = state.reset_for_new_turn(ConversationStatus::Running);
        let projection = TurnProjection {
            host: host.clone(), message: message(), output: EngineTurnOutput::new(state.clone(), turn),
            calls: Mutex::new(BTreeMap::new()), terminal: Mutex::new(None),
            last_model_step: std::sync::atomic::AtomicU16::new(0),
        };
        let mut stream = state.subscribe();
        for (id, name, capability, action) in [
            ("motion", "robot_motion", "robot", "robot/motion"),
            ("quiet", nomifun_agent_runtime::FINISH_WITHOUT_REPLY_TOOL_NAME, "engine.control", "engine.control/finish"),
        ] {
            projection.emit(AgentEngineEvent::ToolCallCompleted { step: 1,
                call: nomifun_chat_model_broker::ChatToolCall { call_id: id.into(), name: name.into(),
                    arguments: nomifun_agent_contracts::StrictJsonValue(serde_json::json!({})), provider_metadata: None } }).await.unwrap();
            projection.emit(AgentEngineEvent::ToolStarted { step: 1, call_id: id.into(),
                capability_id: capability.into(), action_id: action.into() }).await.unwrap();
            projection.emit(AgentEngineEvent::ToolCompleted { step: 1, runtime_hold: None,
                result: AgentToolResult::text(id.into(), "accepted", false) }).await.unwrap();
        }
        projection.emit(AgentEngineEvent::SilentCompletion { step: 1, tool_call_ids: vec!["motion".into()] }).await.unwrap();
        let recorded = host.events.lock().unwrap();
        assert!(recorded.iter().any(|event| matches!(event, AgentEngineEvent::SilentCompletion {step:1,tool_call_ids}
            if tool_call_ids.as_slice()==[nomifun_chat_model_broker::ToolCallId::from("motion")])));
        assert!(recorded.iter().any(|event| matches!(event, AgentEngineEvent::ToolCompleted {result,..} if result.call_id.as_ref()=="quiet")));
        drop(recorded);
        let mut visible = Vec::new();
        while let Ok(event) = stream.try_recv() { visible.push(event); }
        assert_eq!(visible.len(), 2, "only the real device running/result rows are user-facing");
        assert!(visible.iter().all(|event| matches!(event, AgentStreamEvent::ToolCall(call) if call.call_id=="motion")));
        assert!(projection.calls.lock().unwrap().is_empty(), "hidden controls still settle their pending projection entries");
    }

    #[tokio::test]
    async fn task_plan_change_is_published_only_after_its_canonical_record() {
        let host = Host::new();
        let state = AgentRuntimeState::new(SESSION, "projection-workspace", 32);
        let turn = state.reset_for_new_turn(ConversationStatus::Running);
        let projection = TurnProjection {
            host: host.clone(), message: message(), output: EngineTurnOutput::new(state.clone(), turn),
            calls: Mutex::new(BTreeMap::new()), terminal: Mutex::new(None),
            last_model_step: std::sync::atomic::AtomicU16::new(0),
        };
        let mut events = state.subscribe();
        let mut plan = nomifun_agent_runtime::AgentPlan::default();
        plan.revision = 1;
        plan.steps.push(nomifun_agent_runtime::AgentPlanStep {
            step: "Verify".into(), status: nomifun_agent_runtime::AgentPlanStatus::Blocked,
        });
        let event = AgentEngineEvent::PlanUpdated { plan };
        host.fail_record.store(true, Ordering::Release);
        assert!(projection.emit(event.clone()).await.is_err());
        assert!(host.events.lock().unwrap().is_empty());
        assert!(events.try_recv().is_err(), "uncommitted progress must not be broadcast");
        host.fail_record.store(false, Ordering::Release);
        projection.emit(event.clone()).await.unwrap();
        assert_eq!(host.events.lock().unwrap().as_slice(), &[event]);
        assert!(matches!(events.try_recv().unwrap(), AgentStreamEvent::TaskPlanChanged));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn public_text_commit_preserves_reasoning_content_without_synthetic_completion_frames() {
        let host = Host::new();
        let state = AgentRuntimeState::new(SESSION, "projection-workspace", 32);
        let turn = state.reset_for_new_turn(ConversationStatus::Running);
        let projection = TurnProjection {
            host: host.clone(), message: message(), output: EngineTurnOutput::new(state.clone(), turn),
            calls: Mutex::new(BTreeMap::new()), terminal: Mutex::new(None),
            last_model_step: std::sync::atomic::AtomicU16::new(0),
        };
        let mut events = state.subscribe();
        projection.emit(AgentEngineEvent::ReasoningDelta {
            step: 1, text: "Inspect the workspace".into(),
        }).await.unwrap();
        let AgentStreamEvent::Thinking(running) = events.try_recv().unwrap() else {
            panic!("expected thinking delta");
        };
        assert_eq!(running.step, Some(1));
        assert_eq!(running.status.as_deref(), Some("thinking"));

        let text = AgentEngineEvent::OutputTextDelta { step: 1, text: "I will inspect the files.".into() };
        host.fail_record.store(true, Ordering::Release);
        assert!(projection.emit(text.clone()).await.is_err());
        assert!(events.try_recv().is_err(), "uncommitted text must not close reasoning");
        host.fail_record.store(false, Ordering::Release);
        projection.emit(text).await.unwrap();
        assert!(matches!(events.try_recv().unwrap(), AgentStreamEvent::Text(_)));
        assert!(events.try_recv().is_err());
        assert_eq!(state.status(), Some(ConversationStatus::Running));

        projection.emit(AgentEngineEvent::OutputTextDelta { step: 1, text: "More progress".into() }).await.unwrap();
        assert!(matches!(events.try_recv().unwrap(), AgentStreamEvent::Text(_)));
        assert!(events.try_recv().is_err(), "display references alone own canonical phase completion");
    }

    #[tokio::test]
    async fn thinking_deltas_do_not_fabricate_phase_completion_or_turn_terminal() {
        let host = Host::new();
        let state = AgentRuntimeState::new(SESSION, "projection-workspace", 32);
        let turn = state.reset_for_new_turn(ConversationStatus::Running);
        let projection = TurnProjection {
            host, message: message(), output: EngineTurnOutput::new(state.clone(), turn),
            calls: Mutex::new(BTreeMap::new()), terminal: Mutex::new(None),
            last_model_step: std::sync::atomic::AtomicU16::new(0),
        };
        let mut events = state.subscribe();
        projection.emit(AgentEngineEvent::ReasoningDelta { step: 1, text: "Inspect".into() }).await.unwrap();
        assert!(matches!(events.try_recv().unwrap(), AgentStreamEvent::Thinking(data) if data.content == "Inspect"));
        projection.emit(AgentEngineEvent::ModelStepStarted { step: 2,
            operation_id: OperationId::from("model:2"), correction_source: None }).await.unwrap();
        assert!(events.try_recv().is_err(), "only canonical timeline references close a displayed phase");
        projection.emit(AgentEngineEvent::TurnCancelled { model_steps: 2 }).await.unwrap();
        assert!(events.try_recv().is_err(), "terminal publication waits for owner cleanup");
        assert_eq!(state.status(), Some(ConversationStatus::Running));
    }

    #[tokio::test]
    async fn tool_projection_retains_call_identity_and_path_without_private_arguments_or_output() {
        use nomifun_agent_contracts::{ActionId, CapabilityId, StrictJsonValue};
        use nomifun_chat_model_broker::{ChatToolCall, ToolCallId};

        let host = Host::new();
        let state = AgentRuntimeState::new(SESSION, "projection-workspace", 32);
        let turn = state.reset_for_new_turn(ConversationStatus::Running);
        let projection = TurnProjection {
            host: host.clone(),
            message: message(),
            output: EngineTurnOutput::new(state.clone(), turn),
            calls: Mutex::new(BTreeMap::new()),
            terminal: Mutex::new(None),
            last_model_step: std::sync::atomic::AtomicU16::new(0),
        };
        let mut events = state.subscribe();
        let call_id = ToolCallId::from("tool-call");
        projection
            .emit(AgentEngineEvent::ToolCallCompleted {
                step: 1,
                call: ChatToolCall {
                    call_id: call_id.clone(),
                    name: "read_file".to_owned(),
                    arguments: StrictJsonValue(serde_json::json!({"path": "README.md"})),
                    provider_metadata: None,
                },
            })
            .await
            .unwrap();
        assert!(
            events.try_recv().is_err(),
            "completed arguments alone do not prove tool execution"
        );
        projection
            .emit(AgentEngineEvent::ToolStarted {
                step: 1,
                call_id: call_id.clone(),
                capability_id: CapabilityId::from("workspace.files"),
                action_id: ActionId::from("workspace.files/read"),
            })
            .await
            .unwrap();
        let AgentStreamEvent::ToolCall(started) = events.recv().await.unwrap() else {
            panic!("expected tool start");
        };
        assert_eq!(started.status, ToolCallStatus::Running);
        assert_eq!(started.description.as_deref(), Some("README.md"));
        assert!(serde_json::to_value(&started).unwrap().get("args").is_none());
        assert_eq!(started.identity.capability_id.as_deref(), Some("workspace.files"));
        assert_eq!(started.identity.action_id.as_deref(), Some("workspace.files/read"));
        projection
            .emit(AgentEngineEvent::ToolCompleted { runtime_hold: None,
                step: 1,
                result: AgentToolResult::text(call_id, "file unavailable", true),
            })
            .await
            .unwrap();
        let AgentStreamEvent::ToolCall(completed) = events.recv().await.unwrap() else {
            panic!("expected tool result");
        };
        assert_eq!(completed.call_id, started.call_id);
        assert_eq!(completed.name, started.name);
        assert_eq!(completed.identity.capability_id, started.identity.capability_id);
        assert_eq!(completed.identity.action_id, started.identity.action_id);
        assert_eq!(completed.description, started.description);
        assert_eq!(completed.status, ToolCallStatus::Error);
        assert!(serde_json::to_value(&completed).unwrap().get("output").is_none());
        assert_eq!(host.events.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn public_tool_frames_keep_file_targets_without_call_or_result_bodies() {
        use nomifun_agent_contracts::{ActionId, CapabilityId, StrictJsonValue};
        use nomifun_chat_model_broker::ChatToolCall;
        let host = Host::new();
        let state = AgentRuntimeState::new(SESSION, "product-file-projection", 32);
        let turn = state.reset_for_new_turn(ConversationStatus::Running);
        let projection = TurnProjection { host:host.clone(),message:message(),output:EngineTurnOutput::new(state.clone(),turn),
            calls:Mutex::new(BTreeMap::new()),terminal:Mutex::new(None),last_model_step:std::sync::atomic::AtomicU16::new(0) };
        let mut events = state.subscribe();
        for (index, (name, action, arguments, target)) in [
            ("write_file", "workspace.files/write", serde_json::json!({"path":"notes/a  b.md","content":"PRIVATE_TOOL_BODY"}), Some("notes/a  b.md")),
            ("apply_patch", "workspace.files/patch", serde_json::json!({"files":[{"path":"src/app.rs","edits":[{"old_text":"before","new_text":"PRIVATE_TOOL_BODY"}]}]}), Some("src/app.rs")),
        ].into_iter().enumerate() {
            let call_id = format!("file-{index}");
            projection.emit(AgentEngineEvent::ToolCallCompleted {step:1,call:ChatToolCall {
                call_id:call_id.clone().into(),name:name.into(),arguments:StrictJsonValue(arguments),provider_metadata:None }}).await.unwrap();
            projection.emit(AgentEngineEvent::ToolStarted {step:1,call_id:call_id.clone().into(),
                capability_id:CapabilityId::from("workspace.files"),action_id:ActionId::from(action) }).await.unwrap();
            let AgentStreamEvent::ToolCall(started) = events.recv().await.unwrap() else { panic!("start") };
            assert_eq!(started.description.as_deref(), target);
            projection.emit(AgentEngineEvent::ToolCompleted {step:1,runtime_hold:None,
                result:AgentToolResult::text(call_id.into(), "PRIVATE_TOOL_BODY", false) }).await.unwrap();
            let AgentStreamEvent::ToolCall(completed) = events.recv().await.unwrap() else { panic!("result") };
            assert_eq!(completed.description, started.description);
            for call in [started, completed] {
                let value = serde_json::to_value(call).unwrap();
                assert!(!value.to_string().contains("PRIVATE_TOOL_BODY"));
                for field in ["args","input","output"] { assert!(value.get(field).is_none()); }
            }
        }
        assert!(serde_json::to_string(&*host.events.lock().unwrap()).unwrap().contains("PRIVATE_TOOL_BODY"),
            "display projection must not modify canonical model observations");
    }

    #[tokio::test]
    async fn internal_instruction_read_failure_is_recorded_without_a_chat_tool_row() {
        use nomifun_agent_contracts::{ActionId, CapabilityId, StrictJsonValue};
        use nomifun_chat_model_broker::{ChatToolCall, ToolCallId};

        let host = Host::new();
        let runtime = runtime(host.clone(), model(false, false));
        let state = AgentRuntimeState::new(SESSION, "projection-workspace", 32);
        let turn = state.reset_for_new_turn(ConversationStatus::Running);
        let projection = TurnProjection {
            host: host.clone(),
            message: message(),
            output: EngineTurnOutput::new(state.clone(), turn),
            calls: Mutex::new(BTreeMap::new()),
            terminal: Mutex::new(None),
            last_model_step: std::sync::atomic::AtomicU16::new(0),
        };
        let mut events = state.subscribe();
        let call_id = ToolCallId::from("agent-instructions:100");
        projection.emit(AgentEngineEvent::ToolCallCompleted {
            step: 0,
            call: ChatToolCall {
                call_id: call_id.clone(),
                name: "read_file".into(),
                arguments: StrictJsonValue(serde_json::json!({"format":"instruction_scope","path":"."})),
                provider_metadata: None,
            },
        }).await.unwrap();
        projection.emit(AgentEngineEvent::ToolStarted {
            step: 0,
            call_id: call_id.clone(),
            capability_id: CapabilityId::from("workspace.files"),
            action_id: ActionId::from("workspace.files/read"),
        }).await.unwrap();
        projection.emit(AgentEngineEvent::ToolCompleted { runtime_hold: None,
            step: 0,
            result: AgentToolResult::text(call_id, "instruction discovery failed", true),
        }).await.unwrap();

        assert!(events.try_recv().is_err());
        assert_eq!(host.events.lock().unwrap().len(), 3);
        runtime.kill_and_wait(None).await.unwrap();
    }

    #[tokio::test]
    async fn system_permission_live_projection_requires_a_failed_admitted_computer_call() {
        use nomifun_agent_contracts::{ActionId, CapabilityId, StrictJsonValue};
        use nomifun_chat_model_broker::{ChatToolCall, ToolCallId};
        let host = Host::new();
        let state = AgentRuntimeState::new(SESSION, "permission-projection", 32);
        let turn = state.reset_for_new_turn(ConversationStatus::Running);
        let projection = TurnProjection {
            host: host.clone(), message: message(), output: EngineTurnOutput::new(state.clone(), turn),
            calls: Mutex::new(BTreeMap::new()), terminal: Mutex::new(None),
            last_model_step: std::sync::atomic::AtomicU16::new(0),
        };
        let body = serde_json::json!({"schema":"system-permission-required.v1","capability":"computer",
            "action_id":"computer/observe","operation":"screenshot","permissions":["screen_recording"],
            "execution":"not_executed"}).to_string();
        let carrier = nomifun_engine_core::system_permission_error_carrier("SYSTEM_PERMISSION_REQUIRED", &body).unwrap();
        let mut events = state.subscribe();
        for (index, (capability, is_error, output)) in [
            ("computer", true, carrier.as_str()), ("computer", false, carrier.as_str()),
            ("browser", true, carrier.as_str()), ("computer", true, body.as_str()),
        ].into_iter().enumerate() {
            let call_id = ToolCallId::from(format!("permission-{index}"));
            projection.emit(AgentEngineEvent::ToolCallCompleted { step: 1, call: ChatToolCall {
                call_id: call_id.clone(), name: "computer".into(), arguments: StrictJsonValue(serde_json::json!({"action":"screenshot"})),
                provider_metadata: None,
            }}).await.unwrap();
            projection.emit(AgentEngineEvent::ToolStarted { step: 1, call_id: call_id.clone(),
                capability_id: CapabilityId::from(capability), action_id: ActionId::from("computer/observe"),
            }).await.unwrap();
            let AgentStreamEvent::ToolCall(started) = events.recv().await.unwrap() else { panic!("start") };
            assert!(started.identity.permission_required.is_none());
            assert!(started.identity.result_seq.is_none());
            projection.emit(AgentEngineEvent::ToolCompleted { runtime_hold: None, step: 1,
                result: AgentToolResult::text(call_id, output, is_error),
            }).await.unwrap();
            let AgentStreamEvent::ToolCall(completed) = events.recv().await.unwrap() else { panic!("result") };
            assert_eq!(completed.identity.result_seq, Some((index as u64 + 1) * 3));
            assert_eq!(completed.identity.permission_required.is_some(), index == 0);
            assert_eq!(completed.identity.error_code.as_deref(), (index == 0).then_some("SYSTEM_PERMISSION_REQUIRED"));
        }
    }

    #[tokio::test]
    async fn text_turn_publishes_terminal_after_owner_cleanup_and_teardown_is_idempotent() {
        let host = Host::new();
        let runtime = runtime(host.clone(), model(false, false));
        let mut events = runtime.subscribe();
        runtime.send_message(message()).await.unwrap();
        assert!(
            matches!(terminal(&mut events).await, AgentStreamEvent::Finish(data) if data.stop_reason == Some(TurnStopReason::EndTurn))
        );
        runtime.cancel().await.unwrap(); // waits for the owned turn task, even after terminal publication
        runtime.send_message(message()).await.unwrap();
        assert!(matches!(
            terminal(&mut events).await,
            AgentStreamEvent::Finish(_)
        ));
        runtime.kill_and_wait(None).await.unwrap();
        runtime.kill_and_wait(None).await.unwrap();
        assert_eq!(host.cleanup_turns.load(Ordering::Acquire), 2);
        assert_eq!(host.cleanup_sessions.load(Ordering::Acquire), 1);
        assert!(!runtime.is_transport_healthy());
        assert!(runtime.send_message(message()).await.is_err());
        assert_eq!(
            host.events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| matches!(event, AgentEngineEvent::OutputTextDelta { .. }))
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn host_turn_boundary_recycle_rebuilds_only_selected_completed_runtimes() {
        for recycle in [false, true] {
            let hosts = Arc::new(Mutex::new(Vec::<Arc<Host>>::new()));
            let built = hosts.clone();
            let factory: OfficialRuntimeOpener = Arc::new(move |options| {
                assert_eq!(options.conversation_id, SESSION);
                assert_eq!(options.user_id, OWNER);
                assert_eq!(options.extra["retained_choice"], "same-agent-and-resources");
                let host = Host::new();
                host.recycle_completed_turns.store(recycle, Ordering::Release);
                built.lock().unwrap().push(host.clone());
                Box::pin(async move {
                    let runtime = UnifiedAgentRuntime::new(&options, engine(), binding(),
                        model(false, false), Arc::new(NoTools), host)?;
                    Ok(AgentRuntimeHandle::Official(Arc::new(runtime)))
                })
            });
            let registry = InMemoryAgentRuntimeSessions::new(factory);
            for generation in 1..=2 {
                let mut launch = options();
                launch.extra = serde_json::json!({"retained_choice":"same-agent-and-resources"});
                let handle = registry.get_or_create_runtime_for_turn(SESSION, generation,
                    CancellationToken::new(), launch).await.unwrap();
                if generation == 1 {
                    assert_eq!(handle.requires_turn_boundary_recycle(), recycle,
                        "an unused device warmup can be replaced before a later admission");
                }
                assert_eq!(handle.conversation_id(), SESSION);
                let mut events = handle.subscribe();
                let mut input = message();
                input.content = format!("same Session input {generation}");
                input.msg_id = format!("message-{generation}");
                input.source_message_id = Some(format!("root-{generation}"));
                handle.send_message(input).await.unwrap();
                assert!(matches!(terminal(&mut events).await, AgentStreamEvent::Finish(data)
                    if data.stop_reason == Some(TurnStopReason::EndTurn)));
                handle.cancel().await.unwrap();
                assert_eq!(handle.requires_turn_boundary_recycle(), recycle);
                if generation == 1 {
                    let mut launch = options();
                    launch.extra = serde_json::json!({"retained_choice":"same-agent-and-resources"});
                    assert!(registry.get_or_create_runtime_for_turn(SESSION, 2,
                        CancellationToken::new(), launch).await.is_err(),
                        "a new generation must not replace the still-owned old Turn");
                    assert_eq!(hosts.lock().unwrap().len(), 1);
                }
                registry.release_runtime_turn(SESSION, generation).await.unwrap();
            }
            let observed = hosts.lock().unwrap().clone();
            assert_eq!(observed.len(), if recycle { 2 } else { 1 });
            let inputs = observed.iter().flat_map(|host|
                host.prepared_inputs.lock().unwrap().clone()).collect::<Vec<_>>();
            assert_eq!(inputs, ["same Session input 1", "same Session input 2"]);
            if recycle {
                assert_eq!(observed[0].cleanup_sessions.load(Ordering::Acquire), 1,
                    "the old owner must finish cleanup before the successor is built");
            }
            registry.terminate_and_wait_result(SESSION, None).await.unwrap();
        }
    }

    #[tokio::test]
    async fn host_turn_boundary_recycle_preserves_active_turns() {
        let host = Host::new();
        host.recycle_completed_turns.store(true, Ordering::Release);
        let pending_model = model(true, false);
        let running_runtime = runtime(host, pending_model.clone());
        let mut events = running_runtime.subscribe();
        running_runtime.send_message(message()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), pending_model.opened.notified()).await.unwrap();
        assert!(!running_runtime.requires_turn_boundary_recycle(), "an active Turn cannot be recycled");
        running_runtime.cancel().await.unwrap();
        assert!(matches!(terminal(&mut events).await, AgentStreamEvent::Finish(_)));
        assert!(running_runtime.requires_turn_boundary_recycle());
        running_runtime.kill_and_wait(None).await.unwrap();

    }

    #[tokio::test]
    async fn host_turn_boundary_recycle_refreshes_an_unused_warmup_before_first_admission() {
        let hosts = Arc::new(Mutex::new(Vec::<Arc<Host>>::new()));
        let built = hosts.clone();
        let factory: OfficialRuntimeOpener = Arc::new(move |options| {
            let host = Host::new();
            host.recycle_completed_turns.store(true, Ordering::Release);
            built.lock().unwrap().push(host.clone());
            Box::pin(async move {
                Ok(AgentRuntimeHandle::Official(Arc::new(UnifiedAgentRuntime::new(
                    &options, engine(), binding(), model(false, false), Arc::new(NoTools), host,
                )?)))
            })
        });
        let registry = InMemoryAgentRuntimeSessions::new(factory);
        let warmup = registry.get_or_create_runtime(SESSION, options()).await.unwrap();
        assert!(warmup.requires_turn_boundary_recycle());
        registry.get_or_create_runtime(SESSION, options()).await.unwrap();
        assert_eq!(hosts.lock().unwrap().len(), 1, "a read warmup must not rebuild another read warmup");
        let current = registry.get_or_create_runtime_for_turn(SESSION, 1,
            CancellationToken::new(), options()).await.unwrap();
        assert_eq!(current.conversation_id(), SESSION);
        registry.get_or_create_runtime_for_turn(SESSION, 1,
            CancellationToken::new(), options()).await.unwrap();
        let built = hosts.lock().unwrap().clone();
        assert_eq!(built.len(), 2,
            "the first admission refreshes warmup once; its reserved generation must retain the new slot before send");
        assert_eq!(built[0].cleanup_sessions.load(Ordering::Acquire), 1);
        assert_eq!(built[0].cleanup_turns.load(Ordering::Acquire), 0, "warmup never owned an accepted Turn");
        registry.release_runtime_turn(SESSION, 1).await.unwrap();
        registry.terminate_and_wait_result(SESSION, None).await.unwrap();
    }

    #[tokio::test]
    async fn cancellation_covers_owner_preparation_and_model_open_without_killing_session() {
        for preparation in [true, false] {
            let host = Host::new();
            host.pending_preparation
                .store(preparation, Ordering::Release);
            let model = model(true, false);
            let runtime = runtime(host.clone(), model.clone());
            let mut events = runtime.subscribe();
            runtime.send_message(message()).await.unwrap();
            tokio::time::timeout(Duration::from_secs(3), async {
                if preparation {
                    host.entered.notified().await;
                } else {
                    model.opened.notified().await;
                }
            })
            .await
            .unwrap();
            assert!(runtime.send_message(message()).await.is_err());
            tokio::time::timeout(Duration::from_secs(3), runtime.cancel())
                .await
                .unwrap()
                .unwrap();
            assert!(
                matches!(terminal(&mut events).await, AgentStreamEvent::Finish(data) if data.stop_reason == Some(TurnStopReason::Cancelled))
            );
            assert!(runtime.is_transport_healthy());
            let recorded = host.events.lock().unwrap();
            let expected = if preparation { 0 } else { 1 };
            assert_eq!(recorded.iter().filter_map(|event| match event {
                AgentEngineEvent::TurnCancelled { model_steps } => Some(*model_steps), _ => None,
            }).collect::<Vec<_>>(), vec![expected], "cancellation must retain admitted model progress");
            drop(recorded);
            runtime.kill_and_wait(None).await.unwrap();
        }
    }

    #[tokio::test]
    async fn cancellation_after_model_continuation_retains_both_recorded_steps() {
        struct ContinueThenWait { opened: AtomicUsize, waiting: tokio::sync::Notify }
        #[async_trait]
        impl AgentModelPort for ContinueThenWait {
            async fn open_stream(&self, _request: ChatModelRequest, _cancellation: CancellationToken)
                -> Result<AgentModelStream, ChatModelError> {
                if self.opened.fetch_add(1, Ordering::AcqRel) == 0 {
                    return Ok(Box::pin(futures_util::stream::iter(vec![
                        Ok(ChatModelEvent::OutputTextDelta { text:"partial reply".into() }),
                        Ok(ChatModelEvent::Completed { finish_reason:ChatFinishReason::MaxOutputTokens }),
                    ])));
                }
                self.waiting.notify_one();
                std::future::pending().await
            }
        }
        let host = Host::new();
        let model = Arc::new(ContinueThenWait { opened:AtomicUsize::new(0),waiting:tokio::sync::Notify::new() });
        let runtime = UnifiedAgentRuntime::new(&options(), engine(), binding(), model.clone(), Arc::new(NoTools), host.clone()).unwrap();
        let mut stream = runtime.subscribe();
        runtime.send_message(message()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), model.waiting.notified()).await.unwrap();
        runtime.cancel().await.unwrap();
        assert!(matches!(terminal(&mut stream).await, AgentStreamEvent::Finish(data)
            if data.stop_reason == Some(TurnStopReason::Cancelled)));
        let recorded = host.events.lock().unwrap();
        assert_eq!(recorded.iter().filter_map(|event| match event {
            AgentEngineEvent::ModelStepStarted { step,.. } => Some(*step), _ => None,
        }).collect::<Vec<_>>(), vec![1,2]);
        assert_eq!(recorded.iter().filter_map(|event| match event {
            AgentEngineEvent::TurnCancelled { model_steps } => Some(*model_steps), _ => None,
        }).collect::<Vec<_>>(), vec![2]);
        drop(recorded);
        runtime.kill_and_wait(None).await.unwrap();
    }

    #[tokio::test]
    async fn cancellation_before_the_first_driver_poll_still_cleans_and_records_terminal() {
        let host = Host::new();
        let runtime = runtime(host.clone(), model(true, false));
        let mut events = runtime.subscribe();
        runtime.send_message(message()).await.unwrap();
        runtime.cancel().await.unwrap();
        assert!(matches!(
            terminal(&mut events).await,
            AgentStreamEvent::Finish(data)
                if data.stop_reason == Some(TurnStopReason::Cancelled)
        ));
        assert_eq!(host.cleanup_turns.load(Ordering::Acquire), 1);
        assert!(host.events.lock().unwrap().iter().any(|event|
            matches!(event, AgentEngineEvent::TurnCancelled { .. })));
        assert!(runtime.is_transport_healthy());
        runtime.kill_and_wait(None).await.unwrap();
    }

    #[tokio::test]
    async fn every_pre_active_preparation_failure_cleans_and_records_one_terminal() {
        for stage in 0..6 {
            let host = Host::new();
            host.prepare_failure_at.store(stage, Ordering::Release);
            let runtime = runtime(host.clone(), model(false, false));
            let mut events = runtime.subscribe();
            runtime.send_message(message()).await.unwrap();
            assert!(matches!(terminal(&mut events).await, AgentStreamEvent::Error(_)));
            assert_eq!(host.cleanup_turns.load(Ordering::Acquire), 1, "stage {stage}");
            assert_eq!(
                host.events.lock().unwrap().iter().filter(|event|
                    matches!(event, AgentEngineEvent::TurnFailed { .. })).count(),
                1,
                "stage {stage}",
            );
            assert!(runtime.is_transport_healthy(), "stage {stage}");
            runtime.kill_and_wait(None).await.unwrap();
        }
    }

    #[tokio::test]
    async fn terminal_and_teardown_wait_for_owner_proof_even_if_waiter_is_dropped() {
        let host = Host::new();
        host.block_cleanup.store(true, Ordering::Release);
        let runtime = runtime(host.clone(), model(false, false));
        let mut events = runtime.subscribe();
        runtime.send_message(message()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), host.cleanup_entered.notified())
            .await
            .unwrap();
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(
                event,
                AgentStreamEvent::Finish(_) | AgentStreamEvent::Error(_)
            ));
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(20), runtime.kill_and_wait(None))
                .await
                .is_err()
        );
        assert_eq!(host.cleanup_sessions.load(Ordering::Acquire), 0);
        assert!(runtime.send_message(message()).await.is_err());
        host.cleanup_release.add_permits(1);
        tokio::time::timeout(Duration::from_secs(3), runtime.kill_and_wait(None))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(host.cleanup_sessions.load(Ordering::Acquire), 1);
        assert!(
            matches!(terminal(&mut events).await, AgentStreamEvent::Finish(data) if data.stop_reason == Some(TurnStopReason::Cancelled))
        );
    }

    #[tokio::test]
    async fn provider_panic_is_terminal_and_still_cleans_up() {
        let host = Host::new();
        let runtime = runtime(host.clone(), model(false, true));
        let mut events = runtime.subscribe();
        runtime.send_message(message()).await.unwrap();
        assert!(matches!(
            terminal(&mut events).await,
            AgentStreamEvent::Error(_)
        ));
        runtime.kill_and_wait(None).await.unwrap();
        assert_eq!(host.cleanup_turns.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn session_pool_retains_official_driver_quarantine_until_real_owner_cleanup_succeeds() {
        let host = Host::new();
        host.fail_cleanup.store(true, Ordering::Release);
        let calls = Arc::new(AtomicUsize::new(0));
        let host_capture = host.clone();
        let calls_capture = calls.clone();
        let factory: OfficialRuntimeOpener = Arc::new(move |_| {
            let host = host_capture.clone();
            calls_capture.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                Ok(AgentRuntimeHandle::Official(Arc::new(runtime(
                    host,
                    model(false, false),
                ))))
            })
        });
        let registry = InMemoryAgentRuntimeSessions::new(factory);
        let handle = registry
            .get_or_create_runtime(SESSION, options())
            .await
            .unwrap();
        let mut events = handle.subscribe();
        handle.send_message(message()).await.unwrap();
        assert!(matches!(
            terminal(&mut events).await,
            AgentStreamEvent::Error(_)
        ));
        assert!(
            registry
                .terminate_and_wait_result(SESSION, None)
                .await
                .is_err()
        );
        assert!(
            registry
                .get_or_create_runtime(SESSION, options())
                .await
                .is_err()
        );
        assert_eq!(calls.load(Ordering::Acquire), 1);
        host.fail_cleanup.store(false, Ordering::Release);
        registry
            .terminate_and_wait_result(SESSION, None)
            .await
            .unwrap();
    }

}
