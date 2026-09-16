//! Shared lifecycle for compiled-in execution engines. This module owns no
//! model strategy, tool registry, transcript database or permission authority.
//! A driver composes an engine with admitted platform ports; the runtime owns
//! task lifetime, UI generation fencing and cleanup-before-terminal ordering.

pub use crate::engine_effect_scope::{EngineEffectScope, EngineEffectSettlement};
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
    AgentCapabilityActivationSnapshot, AgentRuntimeControl, RegisteredAgentRuntime,
    RuntimeEngineBinding, RuntimeEngineFactory, RuntimeSteerDelivery, RuntimeTeardown,
};

/// Only nonterminal UI events are available to drivers. Persistence belongs to
/// the admitted host and must precede publication. No raw Finish/Error sender
/// is exposed, so an engine cannot accidentally bypass the cleanup barrier.
pub enum EngineProgress {
    Started,
    Text(TextEventData),
    Thinking(ThinkingEventData),
    ToolCall(ToolCallEventData),
}

#[derive(Clone)]
pub struct EngineTurnOutput {
    state: AgentRuntimeState,
    turn: AgentRuntimeTurn,
    open: Arc<Mutex<bool>>,
}

impl EngineTurnOutput {
    pub(crate) fn new(state: AgentRuntimeState, turn: AgentRuntimeTurn) -> Self {
        Self {
            state,
            turn,
            open: Arc::new(Mutex::new(true)),
        }
    }

    /// False means this turn no longer accepts progress. It never retargets a
    /// late event to the next turn, including while cleanup is still pending.
    pub fn publish(&self, progress: EngineProgress) -> bool {
        let open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        if !*open {
            return false;
        }
        let event = match progress {
            EngineProgress::Started => AgentStreamEvent::Start(StartEventData {
                session_id: Some(self.state.conversation_id().to_owned()),
            }),
            EngineProgress::Text(data) => AgentStreamEvent::Text(data),
            EngineProgress::Thinking(data) => AgentStreamEvent::Thinking(data),
            EngineProgress::ToolCall(data) => AgentStreamEvent::ToolCall(data),
        };
        let published = self.state.emit_for_turn(self.turn, event);
        if published {
            self.state.bump_activity();
        }
        published
    }

    fn close(&self) {
        *self.open.lock().unwrap_or_else(|e| e.into_inner()) = false;
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
/// not confer authority. This is a trusted source extension, not a sandbox.
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
struct SharedRuntime {
    state: AgentRuntimeState,
    closed: CancellationToken,
    active: Mutex<Option<ActiveTurn>>,
    cleanup: Mutex<Option<Arc<CleanupFlight>>>,
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
            output.close();
            task_cancellation.cancel();
            let cleanup = AssertUnwindSafe(shared.driver.cleanup_turn(&message))
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err(AppError::Internal("Engine turn cleanup panicked".into())));
            if let Err(error) = cleanup {
                break_transport(&shared, turn, error.to_string());
                return;
            }
            let mut outcome =
                execution.unwrap_or_else(|error| EngineTurnOutcome::failed(error.to_string()));
            if requested_cancellation.is_cancelled() {
                outcome.terminal = EngineTurnTerminal::Cancelled;
            }
            let record = AssertUnwindSafe(shared.driver.record_terminal(&message, &outcome))
                .catch_unwind()
                .await;
            if !matches!(record, Ok(Ok(()))) {
                break_transport(
                    &shared,
                    turn,
                    "Engine terminal could not be recorded by its Session owner".into(),
                );
                return;
            }
            let reason = match outcome.terminal {
                EngineTurnTerminal::Completed { finish_reason } => match finish_reason {
                    ChatFinishReason::Completed => TurnStopReason::EndTurn,
                    ChatFinishReason::MaxOutputTokens => TurnStopReason::MaxTokens,
                    ChatFinishReason::Refusal => TurnStopReason::Refusal,
                    ChatFinishReason::Cancelled => TurnStopReason::Cancelled,
                    ChatFinishReason::ToolCalls => TurnStopReason::MaxTurnRequests,
                },
                EngineTurnTerminal::Cancelled => TurnStopReason::Cancelled,
                EngineTurnTerminal::Failed { message } => {
                    shared.state.emit_error_data_for_turn(
                        turn,
                        AgentSendError::from_app_error(AppError::Conflict(message))
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
            return Err(AppError::Conflict(
                "Engine cleanup or event recording failed".into(),
            ));
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
impl RegisteredAgentRuntime for HostedAgentRuntime {
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
                let completion = shared
                    .active
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .map(|turn| turn.completion.clone());
                let driver = shared.driver.clone();
                let task = executor.spawn(async move {
                    let joined = match completion {
                        Some(completion) => completion.await,
                        None => Ok(()),
                    };
                    AssertUnwindSafe(driver.cleanup_session())
                        .catch_unwind()
                        .await
                        .map_err(|_| "Engine Session cleanup panicked".to_owned())?
                        .map_err(|error| error.to_string())?;
                    joined
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
            RuntimeEngineBinding,
        ) -> BoxFuture<'static, Result<Arc<dyn EngineSessionDriver>, AppError>>
        + Send
        + Sync,
>;

/// Register this factory with the existing catalog and an explicit admission
/// policy during application assembly. No packaged upload/install API exists.
pub fn hosted_engine_factory(factory: EngineDriverFactory) -> RuntimeEngineFactory {
    Arc::new(move |options, binding| {
        let factory = factory.clone();
        Box::pin(async move {
            let runtime_options = options.clone();
            let driver = factory(options, binding).await?;
            Ok(Arc::new(HostedAgentRuntime::new(&runtime_options, driver)?)
                as Arc<dyn RegisteredAgentRuntime>)
        })
    })
}
