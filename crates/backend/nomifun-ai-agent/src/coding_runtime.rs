//! Coding implementation of the same open runtime contract used by Nomi.
//!
//! This adapter owns tasks and event projection, not Session persistence or
//! tool authority. The composition root must supply admitted host ports. It is
//! not installed on product routes until those ports are wired to their owner.

use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use nomifun_chat_model_broker::ChatFinishReason;
use nomifun_coding_engine::{
    CodingEngine, CodingEngineBuild, CodingEngineError, CodingEngineEvent, CodingEventSink,
    CodingModelPort, CodingToolInvoker, CodingTurnRequest, CodingTurnTerminal, EngineBinding,
};
use nomifun_common::{AgentKillReason, AgentType, AppError, ConversationStatus, TimestampMs};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::protocol::events::{
    AgentStreamEvent, StartEventData, TextEventData, ThinkingEventData, ToolCallEventData,
    ToolCallStatus, TurnStopReason,
};
use crate::protocol::send_error::AgentSendError;
use crate::runtime_state::{AgentRuntimeState, AgentRuntimeTurn};
use crate::types::{AgentRuntimeBuildOptions, SendMessageData};
use crate::{
    AgentRuntimeControl, RegisteredAgentRuntime, RuntimeEngineDescriptor, RuntimeTeardown,
};

/// Per-Session, host-admitted ports. Implementations must validate the durable
/// root message, principal, snapshot, route and tool plan against the existing
/// Session owner. They must not infer authority from user/model JSON.
#[async_trait]
pub trait CodingRuntimeHost: Send + Sync {
    /// Assemble canonical history/context and claim the exact turn. The token
    /// fences all preparation, model and tool work for this logical turn.
    async fn prepare_turn(
        &self,
        message: &SendMessageData,
        cancellation: CancellationToken,
    ) -> Result<CodingTurnRequest, AppError>;

    /// Record engine semantics in the existing owner's event/history chain
    /// before broadcasting a UI projection. No separate engine rollout store.
    async fn record_event(
        &self,
        message: &SendMessageData,
        event: &CodingEngineEvent,
    ) -> Result<(), AppError>;

    /// Required on every exit, including preparation failure and cancellation.
    /// Success proves turn-scoped tools/processes are quiescent.
    async fn cleanup_turn(&self, message: &SendMessageData) -> Result<(), AppError>;

    /// Required, idempotent proof that all Session-owned resources have exited.
    /// A failure retains the registry's teardown quarantine.
    async fn cleanup_session(&self) -> Result<(), AppError>;
}

pub fn coding_runtime_descriptor(
    build: &CodingEngineBuild,
) -> Result<RuntimeEngineDescriptor, AppError> {
    build.validate().map_err(contract_error)?;
    let descriptor = RuntimeEngineDescriptor {
        family_id: build.family_id.as_ref().to_owned(),
        build_id: build.build_id.as_ref().to_owned(),
        build_digest: build.build_digest.as_ref().to_owned(),
        display_name: build.display_name.clone(),
        host_contract_version: crate::RUNTIME_HOST_CONTRACT_VERSION,
        supported_profiles: vec!["coding".to_owned()],
    };
    descriptor.validate()?;
    Ok(descriptor)
}

fn contract_error(error: CodingEngineError) -> AppError {
    AppError::Conflict(format!("Coding runtime: {error}"))
}

type TurnCompletion = Shared<BoxFuture<'static, Result<(), String>>>;

struct ActiveTurn {
    cancellation: CancellationToken,
    done: Arc<AtomicBool>,
    completion: TurnCompletion,
}

struct CompletionGuard(Arc<AtomicBool>);
impl Drop for CompletionGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

struct SharedRuntime {
    owner_id: String,
    state: AgentRuntimeState,
    closed: CancellationToken,
    active: Mutex<Option<ActiveTurn>>,
    cleanup: tokio::sync::Mutex<bool>,
    host: Arc<dyn CodingRuntimeHost>,
}

pub struct CodingAgentRuntime {
    shared: Arc<SharedRuntime>,
    engine: Arc<CodingEngine>,
    binding: EngineBinding,
    model: Arc<dyn CodingModelPort>,
    tools: Arc<dyn CodingToolInvoker>,
}

impl CodingAgentRuntime {
    /// Called by a registered factory only after it has admitted the exact
    /// generic catalog binding and resolved these Session-specific ports.
    pub fn new(
        options: &AgentRuntimeBuildOptions,
        engine: Arc<CodingEngine>,
        binding: EngineBinding,
        model: Arc<dyn CodingModelPort>,
        tools: Arc<dyn CodingToolInvoker>,
        host: Arc<dyn CodingRuntimeHost>,
    ) -> Result<Self, AppError> {
        nomifun_common::UserId::parse(&options.user_id).map_err(|_| {
            AppError::BadRequest("Coding runtime owner must be canonical".to_owned())
        })?;
        nomifun_common::ConversationId::parse(&options.conversation_id).map_err(|_| {
            AppError::BadRequest("Coding runtime Session must be canonical".to_owned())
        })?;
        if options.workspace.trim().is_empty()
            || binding.agent_session_id().as_ref() != options.conversation_id
        {
            return Err(AppError::Conflict(
                "Coding runtime Session/workspace binding mismatch".to_owned(),
            ));
        }
        // Validate build/profile/snapshot before any tasks or tools are started.
        engine
            .open_session(binding.clone(), model.clone(), tools.clone(), None)
            .map_err(contract_error)?;
        Ok(Self {
            shared: Arc::new(SharedRuntime {
                owner_id: options.user_id.clone(),
                state: AgentRuntimeState::new(
                    options.conversation_id.clone(),
                    options.workspace.clone(),
                    2048,
                ),
                closed: CancellationToken::new(),
                active: Mutex::new(None),
                cleanup: tokio::sync::Mutex::new(false),
                host,
            }),
            engine,
            binding,
            model,
            tools,
        })
    }
}

struct TurnProjection {
    shared: Arc<SharedRuntime>,
    message: SendMessageData,
    turn: AgentRuntimeTurn,
    calls: Mutex<BTreeMap<String, ToolCallEventData>>,
    terminal: Mutex<Option<CodingEngineEvent>>,
}

#[async_trait]
impl CodingEventSink for TurnProjection {
    async fn emit(&self, event: CodingEngineEvent) -> Result<(), CodingEngineError> {
        if matches!(
            event,
            CodingEngineEvent::TurnCompleted { .. }
                | CodingEngineEvent::TurnCancelled { .. }
                | CodingEngineEvent::TurnFailed { .. }
        ) {
            // Publication of a terminal must wait for proven tool/process exit.
            *self.terminal.lock().unwrap_or_else(|e| e.into_inner()) = Some(event);
            return Ok(());
        }
        self.shared
            .host
            .record_event(&self.message, &event)
            .await
            .map_err(|error| CodingEngineError::InvalidContract(error.to_string()))?;
        let projected = match event {
            CodingEngineEvent::TurnStarted { .. } => {
                Some(AgentStreamEvent::Start(StartEventData {
                    session_id: Some(self.shared.state.conversation_id().to_owned()),
                }))
            }
            CodingEngineEvent::OutputTextDelta { text, .. } => {
                Some(AgentStreamEvent::Text(TextEventData { content: text }))
            }
            CodingEngineEvent::ReasoningDelta { text, .. } => {
                Some(AgentStreamEvent::Thinking(ThinkingEventData {
                    content: text,
                    subject: None,
                    duration: None,
                    status: None,
                }))
            }
            CodingEngineEvent::ToolCallCompleted { call, .. } => {
                self.calls.lock().unwrap_or_else(|e| e.into_inner()).insert(
                    call.call_id.as_ref().to_owned(),
                    ToolCallEventData {
                        call_id: call.call_id.as_ref().to_owned(),
                        name: call.name,
                        args: call.arguments.0,
                        status: ToolCallStatus::Running,
                        input: None,
                        output: None,
                        description: None,
                        retry: None,
                        artifacts: Vec::new(),
                    },
                );
                None
            }
            CodingEngineEvent::ToolStarted { call_id, .. } => self
                .calls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(call_id.as_ref())
                .cloned()
                .map(AgentStreamEvent::ToolCall),
            CodingEngineEvent::ToolCompleted { result, .. } => {
                let mut call = self
                    .calls
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(result.call_id.as_ref())
                    .ok_or_else(|| {
                        CodingEngineError::InvalidModelEvent(
                            "tool result has no admitted call".to_owned(),
                        )
                    })?;
                call.status = if result.is_error {
                    ToolCallStatus::Error
                } else {
                    ToolCallStatus::Completed
                };
                call.output = Some(result.output_text());
                Some(AgentStreamEvent::ToolCall(call))
            }
            // Semantics are recorded above even when there is no UI projection.
            _ => None,
        };
        if let Some(event) = projected {
            self.shared.state.bump_activity();
            self.shared.state.emit_for_turn(self.turn, event);
        }
        Ok(())
    }
}

#[async_trait]
impl AgentRuntimeControl for CodingAgentRuntime {
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
    fn subscribe(&self) -> broadcast::Receiver<AgentStreamEvent> {
        self.shared.state.subscribe()
    }

    async fn send_message(&self, message: SendMessageData) -> Result<(), AgentSendError> {
        let mut active = self.shared.active.lock().unwrap_or_else(|e| e.into_inner());
        if !self.is_transport_healthy() {
            return Err(AgentSendError::stream_broken(
                "Coding runtime has been closed or quarantined",
            ));
        }
        if active
            .as_ref()
            .is_some_and(|turn| !turn.done.load(Ordering::Acquire))
        {
            return Err(AgentSendError::from_app_error(AppError::Conflict(
                "Coding runtime turn is already running".to_owned(),
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
        let projection = Arc::new(TurnProjection {
            shared: shared.clone(),
            message: message.clone(),
            turn,
            calls: Mutex::new(BTreeMap::new()),
            terminal: Mutex::new(None),
        });
        let session = self
            .engine
            .open_session(
                self.binding.clone(),
                self.model.clone(),
                self.tools.clone(),
                Some(projection.clone()),
            )
            .map_err(|error| AgentSendError::from_app_error(contract_error(error)))?;
        let task = tokio::spawn(async move {
            let _guard = guard;
            let execution = AssertUnwindSafe(async {
                let request = tokio::select! {
                    biased;
                    _ = task_cancellation.cancelled() => return Err(CodingEngineError::Cancelled),
                    request = shared.host.prepare_turn(&message, task_cancellation.clone()) => {
                        request.map_err(|error| CodingEngineError::InvalidContract(error.to_string()))?
                    }
                };
                if request.principal.principal_kind != "user"
                    || request.principal.principal_id != shared.owner_id
                {
                    return Err(CodingEngineError::InvalidContract("Coding turn principal differs from its Session owner".to_owned()));
                }
                session.run_turn_cancellable(request, task_cancellation.clone()).await
            }).catch_unwind().await.unwrap_or(Err(CodingEngineError::TurnPanicked));
            task_cancellation.cancel();
            let cleanup = AssertUnwindSafe(shared.host.cleanup_turn(&message))
                .catch_unwind()
                .await;
            let cleanup = cleanup.unwrap_or_else(|_| {
                Err(AppError::Internal(
                    "Coding turn cleanup panicked".to_owned(),
                ))
            });
            if let Err(error) = cleanup {
                shared.state.mark_transport_broken();
                shared.state.emit_error_data_for_turn(
                    turn,
                    AgentSendError::stream_broken(error.to_string()).into_stream_error(),
                );
                return;
            }
            let pending_terminal = projection
                .terminal
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            let execution = if pending_terminal.is_none() && execution.is_ok() {
                Err(CodingEngineError::InvalidContract(
                    "Coding turn omitted its terminal event".to_owned(),
                ))
            } else {
                execution
            };
            let was_cancelled = requested_cancellation.is_cancelled();
            let terminal = if was_cancelled {
                CodingEngineEvent::TurnCancelled {
                    model_steps: execution.as_ref().map_or(0, |result| result.model_steps),
                }
            } else {
                pending_terminal.unwrap_or_else(|| match &execution {
                    Err(CodingEngineError::Cancelled) => {
                        CodingEngineEvent::TurnCancelled { model_steps: 0 }
                    }
                    Err(error) => CodingEngineEvent::TurnFailed {
                        model_steps: 0,
                        message: error.to_string(),
                    },
                    Ok(result) => CodingEngineEvent::TurnFailed {
                        model_steps: result.model_steps,
                        message: "Coding turn omitted its terminal event".to_owned(),
                    },
                })
            };
            let record = AssertUnwindSafe(shared.host.record_event(&message, &terminal))
                .catch_unwind()
                .await;
            if !matches!(record, Ok(Ok(()))) {
                shared.state.mark_transport_broken();
                shared.state.emit_error_data_for_turn(
                    turn,
                    AgentSendError::stream_broken(
                        "Coding terminal could not be recorded by its Session owner",
                    )
                    .into_stream_error(),
                );
                return;
            }
            let terminal = if was_cancelled {
                CodingTurnTerminal::Cancelled
            } else {
                match execution {
                    Ok(result) => result.terminal,
                    Err(CodingEngineError::Cancelled) => CodingTurnTerminal::Cancelled,
                    Err(error) => CodingTurnTerminal::Failed {
                        message: error.to_string(),
                    },
                }
            };
            let reason = match terminal {
                CodingTurnTerminal::Completed { finish_reason } => match finish_reason {
                    ChatFinishReason::Completed => TurnStopReason::EndTurn,
                    ChatFinishReason::MaxOutputTokens => TurnStopReason::MaxTokens,
                    ChatFinishReason::Refusal => TurnStopReason::Refusal,
                    ChatFinishReason::Cancelled => TurnStopReason::Cancelled,
                    ChatFinishReason::ToolCalls => TurnStopReason::MaxTurnRequests,
                },
                CodingTurnTerminal::Cancelled => TurnStopReason::Cancelled,
                CodingTurnTerminal::Failed { message } => {
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
        let completion = async move { task.await.map_err(|error| error.to_string()) }
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
        let completion = {
            let active = self.shared.active.lock().unwrap_or_else(|e| e.into_inner());
            active.as_ref().map(|turn| {
                turn.cancellation.cancel();
                turn.completion.clone()
            })
        };
        if let Some(completion) = completion {
            completion.await.map_err(AppError::Internal)?;
        }
        if !self.shared.state.is_transport_healthy() {
            return Err(AppError::Conflict(
                "Coding turn cleanup or event recording failed".to_owned(),
            ));
        }
        Ok(())
    }

    fn kill(&self, _reason: Option<AgentKillReason>) -> Result<(), AppError> {
        self.shared.closed.cancel();
        Ok(())
    }
}

#[async_trait]
impl RegisteredAgentRuntime for CodingAgentRuntime {
    fn kill_and_wait(&self, _reason: Option<AgentKillReason>) -> RuntimeTeardown {
        self.shared.closed.cancel();
        let shared = self.shared.clone();
        Box::pin(async move {
            let mut cleaned = shared.cleanup.lock().await;
            if *cleaned {
                return Ok(());
            }
            let completion = shared
                .active
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .map(|turn| turn.completion.clone());
            let joined = match completion {
                Some(completion) => completion.await,
                None => Ok(()),
            };
            AssertUnwindSafe(shared.host.cleanup_session())
                .catch_unwind()
                .await
                .map_err(|_| AppError::Internal("Coding Session cleanup panicked".to_owned()))??;
            joined.map_err(AppError::Internal)?;
            *cleaned = true;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_registry::AgentRuntimeFactory;
    use crate::{
        AgentRuntimeHandle, AgentRuntimeRegistry, InMemoryAgentRuntimeRegistry,
        RuntimeEngineCatalog,
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
    use nomifun_coding_engine::{
        CodingModelStream, CodingRuntimeProfile, CodingToolInvocation, CodingToolPlan,
        CodingToolResult, EngineBuildId, EngineFamilyId,
    };
    use std::collections::BTreeSet;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    const OWNER: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const SESSION: &str = "0190f5fe-7c00-7a00-8000-000000000002";

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

    fn engine() -> Arc<CodingEngine> {
        Arc::new(
            CodingEngine::new(CodingEngineBuild {
                family_id: EngineFamilyId::from("nomifun.coding"),
                build_id: EngineBuildId::from("test-build"),
                build_digest: DigestHex::from("a".repeat(64)),
                display_name: "Coding test".to_owned(),
                supported_profiles: vec![CodingRuntimeProfile::Coding],
            })
            .unwrap(),
        )
    }

    fn binding() -> EngineBinding {
        engine()
            .bind(
                AgentSessionId::from(SESSION),
                RuntimeBindingId::from("binding"),
                CodingRuntimeProfile::Coding,
                ResolvedSnapshotRef {
                    snapshot_id: ResolvedSnapshotId::from("snapshot"),
                    snapshot_digest: DigestHex::from("b".repeat(64)),
                },
            )
            .unwrap()
    }

    struct Host {
        events: Mutex<Vec<CodingEngineEvent>>,
        cleanup_turns: AtomicUsize,
        cleanup_sessions: AtomicUsize,
        fail_cleanup: AtomicBool,
        pending_preparation: AtomicBool,
        block_cleanup: AtomicBool,
        cleanup_entered: tokio::sync::Notify,
        cleanup_release: tokio::sync::Semaphore,
        entered: tokio::sync::Notify,
    }

    impl Host {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                events: Mutex::new(Vec::new()),
                cleanup_turns: AtomicUsize::new(0),
                cleanup_sessions: AtomicUsize::new(0),
                fail_cleanup: AtomicBool::new(false),
                pending_preparation: AtomicBool::new(false),
                entered: tokio::sync::Notify::new(),
                block_cleanup: AtomicBool::new(false),
                cleanup_entered: tokio::sync::Notify::new(),
                cleanup_release: tokio::sync::Semaphore::new(0),
            })
        }
    }

    #[async_trait]
    impl CodingRuntimeHost for Host {
        async fn prepare_turn(
            &self,
            message: &SendMessageData,
            _cancellation: CancellationToken,
        ) -> Result<CodingTurnRequest, AppError> {
            self.entered.notify_one();
            if self.pending_preparation.load(Ordering::Acquire) {
                return std::future::pending().await;
            }
            let route =
                ChatRouteIdentity::new("preset@1", "agent_chat", ModelRouteId::from("route"), 1);
            Ok(CodingTurnRequest::new(
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
                CodingToolPlan::default(),
                PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: OWNER.to_owned(),
                },
                1,
            ))
        }
        async fn record_event(
            &self,
            _message: &SendMessageData,
            event: &CodingEngineEvent,
        ) -> Result<(), AppError> {
            if matches!(
                event,
                CodingEngineEvent::TurnCompleted { .. }
                    | CodingEngineEvent::TurnCancelled { .. }
                    | CodingEngineEvent::TurnFailed { .. }
            ) {
                assert!(
                    self.cleanup_turns.load(Ordering::Acquire) > 0,
                    "terminal must follow cleanup proof"
                );
            }
            self.events.lock().unwrap().push(event.clone());
            Ok(())
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
    impl CodingModelPort for Model {
        async fn open_stream(
            &self,
            _request: ChatModelRequest,
            _cancellation: CancellationToken,
        ) -> Result<CodingModelStream, ChatModelError> {
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
    impl CodingToolInvoker for NoTools {
        async fn invoke(
            &self,
            _request: CodingToolInvocation,
            _cancel: CancellationToken,
        ) -> Result<CodingToolResult, CodingEngineError> {
            panic!("an empty admitted plan must not invoke tools")
        }
    }

    fn runtime(host: Arc<Host>, model: Arc<Model>) -> CodingAgentRuntime {
        CodingAgentRuntime::new(
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
    async fn tool_projection_retains_call_identity_arguments_and_error_outcome() {
        use nomifun_agent_contracts::{ActionId, CapabilityId, StrictJsonValue};
        use nomifun_chat_model_broker::{ChatToolCall, ToolCallId};

        let host = Host::new();
        let runtime = runtime(host.clone(), model(false, false));
        let turn = runtime
            .shared
            .state
            .reset_for_new_turn(ConversationStatus::Running);
        let projection = TurnProjection {
            shared: runtime.shared.clone(),
            message: message(),
            turn,
            calls: Mutex::new(BTreeMap::new()),
            terminal: Mutex::new(None),
        };
        let mut events = runtime.subscribe();
        let call_id = ToolCallId::from("tool-call");
        projection
            .emit(CodingEngineEvent::ToolCallCompleted {
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
            .emit(CodingEngineEvent::ToolStarted {
                step: 1,
                call_id: call_id.clone(),
                capability_id: CapabilityId::from("fs.read"),
                action_id: ActionId::from("read"),
            })
            .await
            .unwrap();
        let AgentStreamEvent::ToolCall(started) = events.recv().await.unwrap() else {
            panic!("expected tool start");
        };
        assert_eq!(started.status, ToolCallStatus::Running);
        assert_eq!(started.args["path"], "README.md");
        projection
            .emit(CodingEngineEvent::ToolCompleted {
                step: 1,
                result: CodingToolResult::text(call_id, "file unavailable", true),
            })
            .await
            .unwrap();
        let AgentStreamEvent::ToolCall(completed) = events.recv().await.unwrap() else {
            panic!("expected tool result");
        };
        assert_eq!(completed.call_id, started.call_id);
        assert_eq!(completed.name, started.name);
        assert_eq!(completed.args, started.args);
        assert_eq!(completed.status, ToolCallStatus::Error);
        assert_eq!(completed.output.as_deref(), Some("file unavailable"));
        assert_eq!(host.events.lock().unwrap().len(), 3);
        runtime.kill_and_wait(None).await.unwrap();
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
                .filter(|event| matches!(event, CodingEngineEvent::OutputTextDelta { .. }))
                .count(),
            2
        );
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
    async fn registry_retains_coding_quarantine_until_real_owner_cleanup_succeeds() {
        let host = Host::new();
        host.fail_cleanup.store(true, Ordering::Release);
        let calls = Arc::new(AtomicUsize::new(0));
        let host_capture = host.clone();
        let calls_capture = calls.clone();
        let factory: AgentRuntimeFactory = Arc::new(move |_| {
            let host = host_capture.clone();
            calls_capture.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                Ok(AgentRuntimeHandle::Registered(Arc::new(runtime(
                    host,
                    model(false, false),
                ))))
            })
        });
        let registry = InMemoryAgentRuntimeRegistry::new(factory);
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

    #[tokio::test]
    async fn coding_is_constructed_through_the_open_catalog_without_a_handle_variant() {
        let engine = engine();
        let descriptor = coding_runtime_descriptor(engine.build()).unwrap();
        let selector = crate::RuntimeEngineSelector::Exact {
            family_id: descriptor.family_id.clone(),
            build_id: descriptor.build_id.clone(),
            build_digest: descriptor.build_digest.clone(),
        };
        let mut catalog = RuntimeEngineCatalog::default();
        catalog
            .register(
                descriptor,
                Arc::new(move |options, _exact| {
                    let engine = engine.clone();
                    Box::pin(async move {
                        let runtime = CodingAgentRuntime::new(
                            &options,
                            engine,
                            binding(),
                            model(false, false),
                            Arc::new(NoTools),
                            Host::new(),
                        )?;
                        Ok(Arc::new(runtime) as Arc<dyn RegisteredAgentRuntime>)
                    })
                }),
            )
            .unwrap();
        let exact = catalog.resolve(&selector, "coding").unwrap();
        let catalog = Arc::new(catalog);
        let registry = InMemoryAgentRuntimeRegistry::new(catalog.bound_factory(exact).unwrap());
        let handle = registry
            .get_or_create_runtime(SESSION, options())
            .await
            .unwrap();
        assert!(matches!(handle, AgentRuntimeHandle::Registered(_)));
        let mut events = handle.subscribe();
        handle.send_message(message()).await.unwrap();
        assert!(matches!(
            terminal(&mut events).await,
            AgentStreamEvent::Finish(_)
        ));
        registry
            .terminate_and_wait_result(SESSION, None)
            .await
            .unwrap();
    }
}
