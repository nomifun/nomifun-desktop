//! Shared lifecycle for compiled-in execution engines. This module owns no
//! model strategy, tool registry, transcript database or permission authority.
//! A driver composes an engine with admitted platform ports; the runtime owns
//! task lifetime, UI generation fencing and cleanup-before-terminal ordering.

pub use crate::engine_tasks::{EngineOwnedTask, EngineTaskGroup};
pub use nomifun_engine_core::{
    EngineResourcePort, EngineResourceRead, EngineResourceImageRead, EngineResourceQuery,
    EngineContextContent, EngineContextResource, EngineEffectClass, EngineToolBinding,
    EngineToolError, EngineToolExposure, EngineToolInvocation, EngineToolInvoker, EngineToolPlan,
    EngineToolResult, KernelEngineToolInvoker, compile_engine_tool_plan,
};

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use nomifun_chat_model_broker::ChatFinishReason;
pub use nomifun_chat_model_broker::{BrokerEngineModelPort, EngineModelPort, EngineModelStream};
use nomifun_common::{AgentKillReason, AgentType, AppError, ConversationStatus, TimestampMs};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::protocol::events::{
    AgentStreamEvent, StartEventData, TextEventData, ThinkingEventData, ToolCallEventData,
    TurnStopReason,
};
use crate::protocol::send_error::AgentSendError;
use crate::runtime_state::{AgentRuntimeState, AgentRuntimeTurn};
use crate::types::{AgentRuntimeBuildOptions, SendMessageData};
use crate::{
    AgentCapabilityActivationSnapshot, AgentRuntimeControl, OfficialAgentRuntime,
    RuntimeBuildBinding, OfficialRuntimeFactory, RuntimeSteerDelivery, RuntimeTeardown,
};

/// Only nonterminal UI events are available to drivers. Persistence belongs to
/// the admitted host and must precede publication. No raw Finish/Error sender
/// is exposed, so an engine cannot accidentally bypass the cleanup barrier.
pub enum EngineProgress {
    Started,
    Text(TextEventData),
    Thinking(ThinkingEventData),
    ToolCall(ToolCallEventData),
    TaskPlanChanged,
}

#[derive(Clone)]
pub struct EngineTurnOutput {
    state: AgentRuntimeState,
    turn: AgentRuntimeTurn,
    progress: Arc<Mutex<TurnOutputProgress>>,
}

struct TurnOutputProgress {
    open: bool,
    model_steps: u16,
    /// A current-turn UI notice only. This is never part of EngineTurnOutcome,
    /// the Session journal, checkpoint, or retained settlement receipt.
    model_gateway_notice: Option<nomifun_api_types::AgentStreamErrorData>,
}

impl EngineTurnOutput {
    pub(crate) fn new(state: AgentRuntimeState, turn: AgentRuntimeTurn) -> Self {
        Self {
            state,
            turn,
            progress: Arc::new(Mutex::new(TurnOutputProgress { open: true, model_steps: 0, model_gateway_notice: None })),
        }
    }

    /// False means this turn no longer accepts progress. It never retargets a
    /// late event to the next turn, including while cleanup is still pending.
    pub fn publish(&self, progress: EngineProgress) -> bool {
        let progress_state = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        if !progress_state.open {
            return false;
        }
        let event = match progress {
            EngineProgress::Started => AgentStreamEvent::Start(StartEventData {
                session_id: Some(self.state.conversation_id().to_owned()),
            }),
            EngineProgress::Text(data) => AgentStreamEvent::Text(data),
            EngineProgress::Thinking(data) => AgentStreamEvent::Thinking(data),
            EngineProgress::ToolCall(data) => AgentStreamEvent::ToolCall(data),
            EngineProgress::TaskPlanChanged => AgentStreamEvent::TaskPlanChanged,
        };
        let published = self.state.emit_for_turn(self.turn, event);
        if published {
            self.state.bump_activity();
        }
        published
    }

    /// Retain host-recorded model progress even if cancellation drops the
    /// driver future. Late observations cannot update a closed turn.
    pub fn record_model_steps(&self, model_steps: u16) -> bool {
        let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        if !progress.open { return false; }
        progress.model_steps = progress.model_steps.max(model_steps);
        true
    }

    fn close(&self) -> u16 {
        let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        progress.open = false;
        progress.model_steps
    }

    pub(crate) fn record_model_gateway_failure(
        &self,
        code: nomifun_chat_model_broker::ChatModelErrorCode,
        message: &str,
    ) -> bool {
        let Some(error) = AgentSendError::from_model_gateway_failure(code, message) else { return false; };
        let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        if !progress.open { return false; }
        progress.model_gateway_notice = Some(error.into_stream_error());
        true
    }

    /// Called by the SDK only after cleanup and the unchanged pause receipt
    /// succeed. System is nonterminal; the existing Finish(Paused) follows.
    fn publish_settled_model_gateway_notice(&self) -> bool {
        let error = {
            let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
            if progress.open { return false; }
            progress.model_gateway_notice.take()
        };
        error.is_some_and(|error| self.state.emit_for_turn(self.turn, AgentStreamEvent::System(
            serde_json::json!({"kind":"model_gateway_account_action", "error":error})
        )))
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EngineTurnOutcome {
    pub model_steps: u16,
    pub terminal: EngineTurnTerminal,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum EngineTurnTerminal {
    Completed { finish_reason: ChatFinishReason },
    Cancelled,
    Paused { reason: String },
    Failed { message: String },
}

impl EngineTurnOutcome {
    pub fn cancelled(model_steps: u16) -> Self {
        Self {
            model_steps,
            terminal: EngineTurnTerminal::Cancelled,
        }
    }
    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            model_steps: 0,
            terminal: EngineTurnTerminal::Failed {
                message: message.into(),
            },
        }
    }
}

/// One exact, host-admitted Session. Implementations choose their own request
/// loop, planning and context strategy. They must use the platform's ports for
/// models, effects, history and durable turn admission; options/model JSON do
/// not confer authority. This is the trusted internal Driver seam, not a sandbox.
#[async_trait]
pub trait EngineSessionDriver: Send + Sync {
    /// Includes preparation. The outer runtime may drop this future on cancel;
    /// platform effect owners must retain any admitted work until cleanup.
    async fn run_turn(
        &self,
        message: &SendMessageData,
        cancellation: CancellationToken,
        output: EngineTurnOutput,
    ) -> Result<EngineTurnOutcome, AppError>;

    /// Required after every exit, including failed/partially cancelled prepare.
    /// Must join owned effects and prove resources quiescent, not just send kill.
    async fn cleanup_turn(&self, message: &SendMessageData) -> Result<(), AppError>;

    /// Optional nonterminal quarantine after failed cleanup. True must mean
    /// the host persisted a fenced pause with cleanup explicitly UNPROVEN.
    async fn suspend_after_cleanup_failure(&self, _message: &SendMessageData) -> Result<bool,AppError> { Ok(false) }

    /// Called only after cleanup proof. Store this exact outcome in the
    /// existing Session owner's journal; never start an independent Session DB.
    async fn record_terminal(
        &self,
        message: &SendMessageData,
        outcome: &EngineTurnOutcome,
    ) -> Result<(), AppError>;

    /// Idempotent, result-bearing release of all Session-scoped resources.
    /// Failed attempts can be retried, but a dropped waiter is not a retry.
    async fn cleanup_session(&self) -> Result<(), AppError>;

    fn capability_activation_snapshot(
        &self,
    ) -> Result<Option<AgentCapabilityActivationSnapshot>, AppError> {
        Ok(None)
    }

    fn supports_steering_context(&self) -> bool {
        false
    }

    async fn queue_steer(&self, _delivery: RuntimeSteerDelivery) -> Result<bool, AppError> {
        Err(AppError::BadRequest(
            "The selected engine does not support receipt-bound steering".into(),
        ))
    }
}

type Completion = Shared<BoxFuture<'static, Result<(), String>>>;
struct ActiveTurn {
    cancellation: CancellationToken,
    done: Arc<AtomicBool>,
    completion: Completion,
}
struct CompletionGuard(Arc<AtomicBool>);
impl Drop for CompletionGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
struct CleanupFlight {
    completion: Completion,
}
#[derive(Clone)]
struct PendingTurnSettlement {
    message: SendMessageData,
    outcome: EngineTurnOutcome,
    cleanup_proven: bool,
    cancellation: CancellationToken,
    /// First settlement rejection, retained with the exact Turn until its
    /// cleanup and terminal receipt are proven. This is diagnostic evidence,
    /// not permission to rerun the driver or release Session resources.
    failure: Option<String>,
}

impl PendingTurnSettlement {
    fn remember_failure(&mut self, stage: &str, error: impl std::fmt::Display) -> String {
        self.failure.get_or_insert_with(|| {
            crate::protocol::send_error::sanitize_error_detail(
                &nomi_redact::redact_secrets_owned(format!("{stage}: {error}")),
            )
        }).clone()
    }
}
struct SharedRuntime {
    state: AgentRuntimeState,
    closed: CancellationToken,
    active: Mutex<Option<ActiveTurn>>,
    cleanup: Mutex<Option<Arc<CleanupFlight>>>,
    pending_settlement: Mutex<Option<PendingTurnSettlement>>,
    driver: Arc<dyn EngineSessionDriver>,
}

/// Common Registered runtime used by official and source-integrated engines.
/// Existing registry remains the only cross-runtime Session coordinator.
pub struct HostedAgentRuntime {
    shared: Arc<SharedRuntime>,
}

impl HostedAgentRuntime {
    pub fn new(
        options: &AgentRuntimeBuildOptions,
        driver: Arc<dyn EngineSessionDriver>,
    ) -> Result<Self, AppError> {
        nomifun_common::UserId::parse(&options.user_id)
            .map_err(|_| AppError::BadRequest("Engine owner must be canonical".into()))?;
        nomifun_common::ConversationId::parse(&options.conversation_id)
            .map_err(|_| AppError::BadRequest("Engine Session must be canonical".into()))?;
        if options.workspace.trim().is_empty() {
            return Err(AppError::BadRequest(
                "Engine requires a host-resolved workspace".into(),
            ));
        }
        Ok(Self {
            shared: Arc::new(SharedRuntime {
                state: AgentRuntimeState::new(
                    options.conversation_id.clone(),
                    options.workspace.clone(),
                    2048,
                ),
                closed: CancellationToken::new(),
                active: Mutex::new(None),
                cleanup: Mutex::new(None),
                pending_settlement: Mutex::new(None),
                driver,
            }),
        })
    }
}

#[async_trait]
impl AgentRuntimeControl for HostedAgentRuntime {
    fn agent_type(&self) -> AgentType {
        AgentType::Nomi
    }
    fn conversation_id(&self) -> &str {
        self.shared.state.conversation_id()
    }
    fn workspace(&self) -> &str {
        self.shared.state.workspace()
    }
    fn status(&self) -> Option<ConversationStatus> {
        self.shared.state.status()
    }
    fn is_transport_healthy(&self) -> bool {
        !self.shared.closed.is_cancelled() && self.shared.state.is_transport_healthy()
    }
    fn last_activity_at(&self) -> TimestampMs {
        self.shared.state.last_activity_at()
    }
    fn touch_activity(&self) {
        self.shared.state.bump_activity();
    }
    fn capability_activation_snapshot(
        &self,
    ) -> Result<Option<AgentCapabilityActivationSnapshot>, AppError> {
        self.shared.driver.capability_activation_snapshot()
    }
    fn subscribe(&self) -> broadcast::Receiver<AgentStreamEvent> {
        self.shared.state.subscribe()
    }

    async fn send_message(&self, message: SendMessageData) -> Result<(), AgentSendError> {
        let mut active = self.shared.active.lock().unwrap_or_else(|e| e.into_inner());
        if !self.is_transport_healthy() {
            return Err(AgentSendError::stream_broken(
                "Engine has been closed or quarantined",
            ));
        }
        if active
            .as_ref()
            .is_some_and(|turn| !turn.done.load(Ordering::Acquire))
        {
            return Err(AgentSendError::from_app_error(AppError::Conflict(
                "Engine turn is already running".into(),
            )));
        }
        let cancellation = self.shared.closed.child_token();
        let requested_cancellation = cancellation.clone();
        let task_cancellation = cancellation.child_token();
        let done = Arc::new(AtomicBool::new(false));
        let guard = CompletionGuard(done.clone());
        let shared = self.shared.clone();
        let turn = shared.state.reset_for_new_turn(ConversationStatus::Running);
        shared.state.bump_activity();
        let output = EngineTurnOutput::new(shared.state.clone(), turn);
        let task = tokio::spawn(async move {
            let _guard = guard;
            let execution = AssertUnwindSafe(async {
                tokio::select! {
                    biased;
                    _ = task_cancellation.cancelled() => Ok(EngineTurnOutcome::cancelled(0)),
                    result = shared.driver.run_turn(&message, task_cancellation.clone(), output.clone()) => result,
                }
            }).catch_unwind().await.unwrap_or_else(|_| Err(AppError::Internal("Engine turn panicked".into())));
            let recorded_model_steps = output.close();
            task_cancellation.cancel();
            let mut outcome = execution.unwrap_or_else(|error| EngineTurnOutcome::failed(error.to_string()));
            outcome.model_steps = outcome.model_steps.max(recorded_model_steps);
            if requested_cancellation.is_cancelled() { outcome.terminal = EngineTurnTerminal::Cancelled; }
            *shared.pending_settlement.lock().unwrap_or_else(|e|e.into_inner()) = Some(PendingTurnSettlement {
                message: message.clone(), outcome: outcome.clone(), cleanup_proven: false,
                cancellation: requested_cancellation.clone(),
                failure: None,
            });
            let cleanup = AssertUnwindSafe(shared.driver.cleanup_turn(&message))
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err(AppError::Internal("Engine turn cleanup panicked".into())));
            if let Err(error) = cleanup {
                let mut cause = error.to_string();
                if let Some(pending) = shared.pending_settlement.lock().unwrap_or_else(|e|e.into_inner()).as_mut() {
                    cause = pending.remember_failure("Engine turn cleanup failed", &error);
                }
                let suspended = AssertUnwindSafe(shared.driver.suspend_after_cleanup_failure(&message)).catch_unwind().await;
                if matches!(suspended,Ok(Ok(true))) {
                    // The host retained a fenced nonterminal cleanup witness.
                    *shared.pending_settlement.lock().unwrap_or_else(|e|e.into_inner()) = None;
                    shared.state.mark_transport_broken();
                    shared.state.emit_finish_for_turn(turn,Some(shared.state.conversation_id().to_owned()),Some(TurnStopReason::Paused));
                    return;
                }
                break_transport(&shared, turn, cause);
                return;
            }
            if requested_cancellation.is_cancelled() {
                outcome.terminal = EngineTurnTerminal::Cancelled;
            }
            if let Some(pending) = shared.pending_settlement.lock().unwrap_or_else(|e|e.into_inner()).as_mut() {
                pending.outcome = outcome.clone();
                pending.cleanup_proven = true;
            }
            let record = AssertUnwindSafe(shared.driver.record_terminal(&message, &outcome))
                .catch_unwind()
                .await;
            let record_failure = match record {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(error.to_string()),
                Err(_) => Some("Engine terminal recording panicked".to_owned()),
            };
            if let Some(error) = record_failure {
                let mut cause = error.clone();
                if let Some(pending) = shared.pending_settlement.lock().unwrap_or_else(|e|e.into_inner()).as_mut() {
                    cause = pending.remember_failure("Engine terminal receipt failed", &error);
                }
                break_transport(
                    &shared,
                    turn,
                    cause,
                );
                return;
            }
            *shared.pending_settlement.lock().unwrap_or_else(|e|e.into_inner()) = None;
            let reason = match outcome.terminal {
                EngineTurnTerminal::Completed { finish_reason } => match finish_reason {
                    ChatFinishReason::Completed => TurnStopReason::EndTurn,
                    ChatFinishReason::MaxOutputTokens => TurnStopReason::MaxTokens,
                    ChatFinishReason::Refusal => TurnStopReason::Refusal,
                    ChatFinishReason::Cancelled => TurnStopReason::Cancelled,
                    ChatFinishReason::ToolCalls => TurnStopReason::MaxTurnRequests,
                },
                EngineTurnTerminal::Cancelled => TurnStopReason::Cancelled,
                EngineTurnTerminal::Paused { .. } => {
                    if !requested_cancellation.is_cancelled() {
                        output.publish_settled_model_gateway_notice();
                    }
                    TurnStopReason::Paused
                }
                EngineTurnTerminal::Failed { message } => {
                    shared.state.emit_error_data_for_turn(
                        turn,
                        AgentSendError::from_engine_turn_failure(message)
                            .into_stream_error(),
                    );
                    return;
                }
            };
            shared.state.emit_finish_for_turn(
                turn,
                Some(shared.state.conversation_id().to_owned()),
                Some(reason),
            );
        });
        // The active slot owns this future. A strong SharedRuntime capture
        // would cycle through active.completion when nobody waits on a turn.
        let state = Arc::downgrade(&self.shared);
        let completion = async move {
            task.await.map_err(|error| {
                let message = error.to_string();
                if let Some(state) = state.upgrade() {
                    break_transport(&state, turn, message.clone());
                }
                message
            })
        }
        .boxed()
        .shared();
        *active = Some(ActiveTurn {
            cancellation,
            done,
            completion,
        });
        Ok(())
    }

    async fn cancel(&self) -> Result<(), AppError> {
        let completion = self
            .shared
            .active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|turn| {
                turn.cancellation.cancel();
                turn.completion.clone()
            });
        if let Some(completion) = completion {
            completion.await.map_err(AppError::Internal)?;
        }
        if !self.shared.state.is_transport_healthy() {
            let cause = self.shared.pending_settlement.lock().unwrap_or_else(|e|e.into_inner())
                .as_ref().and_then(|pending| pending.failure.clone())
                .unwrap_or_else(|| "Engine cleanup or event recording failed".into());
            return Err(AppError::Conflict(cause));
        }
        Ok(())
    }

    fn kill(&self, _reason: Option<AgentKillReason>) -> Result<(), AppError> {
        self.shared.closed.cancel();
        Ok(())
    }
}

fn break_transport(shared: &SharedRuntime, turn: AgentRuntimeTurn, message: String) {
    shared.state.mark_transport_broken();
    shared.state.emit_error_data_for_turn(
        turn,
        AgentSendError::stream_broken(message).into_stream_error(),
    );
}

#[async_trait]
impl OfficialAgentRuntime for HostedAgentRuntime {
    fn supports_steering_context(&self) -> bool {
        self.shared.driver.supports_steering_context()
    }

    async fn steer_with_receipt(&self, delivery: RuntimeSteerDelivery) -> Result<bool, AppError> {
        if (!delivery.files.is_empty() || !delivery.inject_skills.is_empty())
            && !self.supports_steering_context()
        {
            return Err(AppError::BadRequest(
                "The selected engine does not support steering context".into(),
            ));
        }
        if self.shared.closed.is_cancelled() {
            return Ok(false);
        }
        self.shared.driver.queue_steer(delivery).await
    }

    fn kill_and_wait(&self, _reason: Option<AgentKillReason>) -> RuntimeTeardown {
        self.shared.closed.cancel();
        let Ok(executor) = tokio::runtime::Handle::try_current() else {
            return Box::pin(async {
                Err(AppError::Internal(
                    "Engine teardown requires the async host executor".into(),
                ))
            });
        };
        let shared = self.shared.clone();
        // Start the owned cleanup attempt now, not when a particular waiter
        // happens to poll its future. Dropping a waiter never drops cleanup.
        let flight = {
            let mut slot = shared.cleanup.lock().unwrap_or_else(|e| e.into_inner());
            slot.get_or_insert_with(|| {
                let completion = shared.active.lock().unwrap_or_else(|e|e.into_inner())
                    .as_ref().map(|turn|turn.completion.clone());
                let retry_failed_turn = shared.pending_settlement.lock().unwrap_or_else(|e|e.into_inner())
                    .as_ref().is_some_and(|pending|pending.failure.is_some());
                let driver = shared.driver.clone();
                let settlement_owner = shared.clone();
                let task = executor.spawn(async move {
                    let joined = match completion {
                        Some(completion) => completion.await,
                        None => Ok(()),
                    };
                    if let Err(join_error) = joined {
                        // Preserve the existing best-effort resource release
                        // after an abnormal task exit, while retaining failure.
                        let cleanup = AssertUnwindSafe(driver.cleanup_session()).catch_unwind().await;
                        return Err(match cleanup {
                            Ok(Ok(())) => join_error,
                            Ok(Err(error)) => format!("{join_error}; Session cleanup failed: {error}"),
                            Err(_) => format!("{join_error}; Session cleanup panicked"),
                        });
                    }
                    let pending = settlement_owner.pending_settlement.lock().unwrap_or_else(|e|e.into_inner()).clone();
                    if let Some(mut pending) = pending {
                        if !retry_failed_turn {
                            // This flight joined the first cleanup attempt;
                            // it is not an implicit second attempt. Surface its
                            // original condition instead of discarding it.
                            return Err(pending.failure.unwrap_or_else(||
                                "Engine turn cleanup or terminal receipt remains unconfirmed".to_owned()));
                        }
                        // A new explicit teardown flight retries only the retained
                        // cleanup/receipt. The driver is never asked to run_turn again.
                        if !pending.cleanup_proven {
                            AssertUnwindSafe(driver.cleanup_turn(&pending.message)).catch_unwind().await
                                .map_err(|_|"Engine turn cleanup retry panicked".to_owned())?
                                .map_err(|error|error.to_string())?;
                            pending.cleanup_proven = true;
                            if pending.cancellation.is_cancelled() { pending.outcome.terminal = EngineTurnTerminal::Cancelled; }
                            *settlement_owner.pending_settlement.lock().unwrap_or_else(|e|e.into_inner()) = Some(pending.clone());
                        }
                        AssertUnwindSafe(driver.record_terminal(&pending.message,&pending.outcome)).catch_unwind().await
                            .map_err(|_|"Engine terminal retry panicked".to_owned())?
                            .map_err(|error|error.to_string())?;
                        *settlement_owner.pending_settlement.lock().unwrap_or_else(|e|e.into_inner()) = None;
                    }
                    AssertUnwindSafe(driver.cleanup_session())
                        .catch_unwind()
                        .await
                        .map_err(|_| "Engine Session cleanup panicked".to_owned())?
                        .map_err(|error| error.to_string())?;
                    Ok(())
                });
                Arc::new(CleanupFlight {
                    completion: async move { task.await.map_err(|error| error.to_string())? }
                        .boxed()
                        .shared(),
                })
            })
            .clone()
        };
        Box::pin(async move {
            let result = flight.completion.clone().await;
            if result.is_err() {
                let mut slot = shared.cleanup.lock().unwrap_or_else(|e| e.into_inner());
                if slot
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &flight))
                {
                    *slot = None;
                }
            }
            result.map_err(AppError::Conflict)
        })
    }
}

pub type EngineDriverFactory = Arc<
    dyn Fn(
            AgentRuntimeBuildOptions,
            RuntimeBuildBinding,
        ) -> BoxFuture<'static, Result<Arc<dyn EngineSessionDriver>, AppError>>
        + Send
        + Sync,
>;

/// Register this factory with the existing catalog and an explicit admission
/// policy during application assembly. No packaged upload/install API exists.
pub fn hosted_engine_factory(factory: EngineDriverFactory) -> OfficialRuntimeFactory {
    Arc::new(move |options, binding| {
        let factory = factory.clone();
        Box::pin(async move {
            let runtime_options = options.clone();
            let driver = factory(options, binding).await?;
            Ok(Arc::new(HostedAgentRuntime::new(&runtime_options, driver)?)
                as Arc<dyn OfficialAgentRuntime>)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct GatewayNoticeDriver {
        fail_cleanup: bool,
        fail_record: bool,
        cleanup_entered: tokio::sync::Notify,
        cleanup_release: tokio::sync::Notify,
        receipt: Mutex<Option<EngineTurnOutcome>>,
    }

    #[async_trait]
    impl EngineSessionDriver for GatewayNoticeDriver {
        async fn run_turn(&self, _: &SendMessageData, _: CancellationToken, output: EngineTurnOutput)
            -> Result<EngineTurnOutcome, AppError> {
            assert!(output.record_model_gateway_failure(
                nomifun_chat_model_broker::ChatModelErrorCode::ProviderUnavailable,
                nomifun_net::provider_gateway_error::GatewayBusinessError::InsufficientBalance.action_message(),
            ));
            Ok(EngineTurnOutcome { model_steps: 1,
                terminal: EngineTurnTerminal::Paused { reason: "EXECUTION_MODEL_PROVIDER_UNAVAILABLE".into() } })
        }

        async fn cleanup_turn(&self, _: &SendMessageData) -> Result<(), AppError> {
            self.cleanup_entered.notify_one();
            self.cleanup_release.notified().await;
            if self.fail_cleanup { Err(AppError::Internal("controlled cleanup failure".into())) } else { Ok(()) }
        }

        async fn record_terminal(&self, _: &SendMessageData, outcome: &EngineTurnOutcome) -> Result<(), AppError> {
            if self.fail_record { return Err(AppError::Internal("controlled record failure".into())); }
            *self.receipt.lock().unwrap() = Some(outcome.clone());
            Ok(())
        }

        async fn cleanup_session(&self) -> Result<(), AppError> { Ok(()) }
    }

    fn gateway_notice_runtime(fail_cleanup: bool, fail_record: bool) -> (HostedAgentRuntime, Arc<GatewayNoticeDriver>) {
        let driver = Arc::new(GatewayNoticeDriver { fail_cleanup, fail_record,
            cleanup_entered: tokio::sync::Notify::new(), cleanup_release: tokio::sync::Notify::new(),
            receipt: Mutex::new(None) });
        let options = AgentRuntimeBuildOptions {
            user_id: "0190f5fe-7c00-7a00-8000-000000000001".into(),
            agent_type: AgentType::Nomi, workspace: "fixture".into(), model: None,
            conversation_id: "0190f5fe-7c00-7a00-8000-000000000002".into(),
            delegation_policy: Default::default(), extra: serde_json::json!({}),
            conversation_created_at: None, workspace_binding_lease: None,
        };
        (HostedAgentRuntime::new(&options, driver.clone()).unwrap(), driver)
    }

    fn gateway_notice_message() -> SendMessageData {
        SendMessageData { content: "hello".into(), msg_id: "message".into(), source_message_id: Some("root".into()),
            files: Vec::new(), inject_skills: Vec::new(), origin: None }
    }

    async fn notice_event(events: &mut broadcast::Receiver<AgentStreamEvent>) -> AgentStreamEvent {
        tokio::time::timeout(std::time::Duration::from_secs(2), events.recv()).await.unwrap().unwrap()
    }

    #[tokio::test]
    async fn gateway_notice_follows_cleanup_and_unchanged_pause_receipt_before_finish() {
        let (runtime, driver) = gateway_notice_runtime(false, false);
        let mut events = runtime.subscribe();
        runtime.send_message(gateway_notice_message()).await.unwrap();
        driver.cleanup_entered.notified().await;
        assert!(matches!(events.try_recv(), Err(broadcast::error::TryRecvError::Empty)), "no notice before cleanup proof");
        assert!(driver.receipt.lock().unwrap().is_none());
        driver.cleanup_release.notify_one();
        let notice = notice_event(&mut events).await;
        let receipt = driver.receipt.lock().unwrap().clone().expect("receipt precedes notice");
        assert_eq!(serde_json::to_value(receipt).unwrap(), serde_json::json!({"model_steps":1,
            "terminal":{"status":"paused","reason":"EXECUTION_MODEL_PROVIDER_UNAVAILABLE"}}));
        let AgentStreamEvent::System(notice) = notice else { panic!("nonterminal System notice expected"); };
        assert_eq!(notice["kind"], "model_gateway_account_action");
        assert_eq!(notice["error"]["code"], "USER_LLM_PROVIDER_BILLING_REQUIRED");
        assert_eq!(notice["error"]["message"], nomifun_net::provider_gateway_error::GatewayBusinessError::InsufficientBalance.action_message());
        assert!(matches!(notice_event(&mut events).await, AgentStreamEvent::Finish(data) if data.stop_reason == Some(TurnStopReason::Paused)));
    }

    #[tokio::test]
    async fn gateway_notice_is_suppressed_when_cleanup_receipt_or_cancel_prevents_pause_delivery() {
        for (fail_cleanup, fail_record, cancel) in [(true, false, false), (false, true, false), (false, false, true)] {
            let (runtime, driver) = gateway_notice_runtime(fail_cleanup, fail_record);
            let mut events = runtime.subscribe();
            runtime.send_message(gateway_notice_message()).await.unwrap();
            driver.cleanup_entered.notified().await;
            if cancel { runtime.shared.active.lock().unwrap().as_ref().unwrap().cancellation.cancel(); }
            driver.cleanup_release.notify_one();
            let event = notice_event(&mut events).await;
            if cancel {
                assert!(matches!(event, AgentStreamEvent::Finish(data) if data.stop_reason == Some(TurnStopReason::Cancelled)));
                assert!(matches!(driver.receipt.lock().unwrap().as_ref().unwrap().terminal, EngineTurnTerminal::Cancelled));
            } else {
                assert!(matches!(event, AgentStreamEvent::Error(_)), "settlement failure wins over account notice");
                assert!(driver.receipt.lock().unwrap().is_none());
            }
            assert!(matches!(events.try_recv(), Err(broadcast::error::TryRecvError::Empty)), "no delayed gateway notice");
        }
    }

    #[test]
    fn gateway_notice_is_current_turn_only_and_cannot_be_queued_after_close() {
        let state = AgentRuntimeState::new("fixture", "fixture", 8);
        let mut events = state.subscribe();
        let first = EngineTurnOutput::new(state.clone(), state.reset_for_new_turn(ConversationStatus::Running));
        let code = nomifun_chat_model_broker::ChatModelErrorCode::ProviderUnavailable;
        let action = nomifun_net::provider_gateway_error::GatewayBusinessError::InsufficientBalance.action_message();
        assert!(first.record_model_gateway_failure(code, action));
        first.close();
        assert!(!first.record_model_gateway_failure(code, action));
        state.reset_for_new_turn(ConversationStatus::Running);
        assert!(!first.publish_settled_model_gateway_notice());
        assert!(matches!(events.try_recv(), Err(broadcast::error::TryRecvError::Empty)));
    }

    #[test]
    fn model_progress_survives_close_without_leaking_to_a_later_turn() {
        let state = AgentRuntimeState::new("0190f5fe-7c00-7a00-8000-000000000002", "fixture", 16);
        let first = EngineTurnOutput::new(state.clone(), state.reset_for_new_turn(ConversationStatus::Running));
        let late_writer = first.clone();
        assert!(first.record_model_steps(2));
        assert!(first.record_model_steps(1));
        assert_eq!(first.close(), 2);
        let next = EngineTurnOutput::new(state.clone(), state.reset_for_new_turn(ConversationStatus::Running));
        assert!(!late_writer.record_model_steps(9));
        assert_eq!(first.close(), 2);
        assert_eq!(next.close(), 0, "a cancelled old turn must not charge the next turn");
    }
}
