//! Coding implementation of the same open runtime contract used by Nomi.
//!
//! This adapter supplies Coding strategy/event projection to the shared engine
//! lifecycle SDK. Production composition supplies admitted Session/Broker/Kernel
//! ports; neither adapter nor SDK creates a second persistence/authority owner.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use nomifun_coding_engine::{
    CodingEngine, CodingEngineBuild, CodingEngineError, CodingEngineEvent, CodingEventSink,
    CodingModelPort, CodingToolInvoker, CodingTurnRequest, CodingTurnTerminal, EngineBinding,
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
    AgentRuntimeControl, RegisteredAgentRuntime, RuntimeEngineDescriptor, RuntimeTeardown,
};

/// Per-Session, host-admitted ports. Implementations must validate the durable
/// root message, principal, snapshot, route and tool plan against the existing
/// Session owner. They must not infer authority from user/model JSON.
#[async_trait]
pub trait CodingRuntimeHost: Send + Sync {
    async fn queue_steer(&self, _delivery: crate::RuntimeSteerDelivery) -> Result<bool, AppError> {
        Err(AppError::BadRequest("Coding host does not support receipt-bound steering".into()))
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
    ) -> Result<CodingTurnRequest, AppError>;

    /// Record engine semantics in the existing owner's event/history chain
    /// before broadcasting a UI projection. No separate engine rollout store.
    async fn record_event(
        &self,
        message: &SendMessageData,
        event: &CodingEngineEvent,
    ) -> Result<(), AppError>;

    /// Hosts accepting concurrent steering must override this and atomically
    /// check the inbox and persist admission. False must record no ToolStarted.
    async fn admit_tool(
        &self,
        message: &SendMessageData,
        event: &CodingEngineEvent,
    ) -> Result<bool, AppError> {
        if !matches!(event, CodingEngineEvent::ToolStarted { .. }) {
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

pub struct CodingAgentRuntime {
    runtime: HostedAgentRuntime,
}

struct CodingSessionDriver {
    owner_id: String,
    engine: Arc<CodingEngine>,
    binding: EngineBinding,
    model: Arc<dyn CodingModelPort>,
    tools: Arc<dyn CodingToolInvoker>,
    host: Arc<dyn CodingRuntimeHost>,
}

impl CodingAgentRuntime {
    /// The Coding loop plugs into the same lifecycle SDK as source-integrated
    /// engines; only its execution/context policy and semantic codec differ.
    pub fn new(
        options: &AgentRuntimeBuildOptions,
        engine: Arc<CodingEngine>,
        binding: EngineBinding,
        model: Arc<dyn CodingModelPort>,
        tools: Arc<dyn CodingToolInvoker>,
        host: Arc<dyn CodingRuntimeHost>,
    ) -> Result<Self, AppError> {
        if binding.agent_session_id().as_ref() != options.conversation_id {
            return Err(AppError::Conflict("Coding runtime Session binding mismatch".into()));
        }
        engine.open_session(binding.clone(), model.clone(), tools.clone(), None)
            .map_err(contract_error)?;
        let driver = Arc::new(CodingSessionDriver {
            owner_id: options.user_id.clone(), engine, binding, model, tools, host,
        });
        Ok(Self { runtime: HostedAgentRuntime::new(options, driver)? })
    }
}

struct TurnProjection {
    host: Arc<dyn CodingRuntimeHost>,
    message: SendMessageData,
    output: EngineTurnOutput,
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
        self.host
            .record_event(&self.message, &event)
            .await
            .map_err(|error| CodingEngineError::InvalidContract(error.to_string()))?;
        self.project(event)
    }

    async fn admit_tool(&self, event: CodingEngineEvent) -> Result<bool, CodingEngineError> {
        let admitted = self.host.admit_tool(&self.message, &event).await
            .map_err(|error| CodingEngineError::InvalidContract(error.to_string()))?;
        if admitted { self.project(event)?; }
        Ok(admitted)
    }
}

impl TurnProjection {
    fn project(&self, event: CodingEngineEvent) -> Result<(), CodingEngineError> {
        let projected = match event {
            CodingEngineEvent::TurnStarted { .. } => Some(EngineProgress::Started),
            CodingEngineEvent::OutputTextDelta { text, .. } => {
                Some(EngineProgress::Text(TextEventData { content: text }))
            }
            CodingEngineEvent::ReasoningDelta { text, .. } => {
                Some(EngineProgress::Thinking(ThinkingEventData {
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
            CodingEngineEvent::ModelOutputTruncated { discarded_tool_call_ids, .. } => {
                let mut calls = self.calls.lock().unwrap_or_else(|e| e.into_inner());
                for id in discarded_tool_call_ids { calls.remove(id.as_ref()); }
                None
            }
            CodingEngineEvent::ToolStarted { call_id, .. } => self
                .calls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(call_id.as_ref())
                .cloned()
                .map(EngineProgress::ToolCall),
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
                let instruction_read = call.call_id.starts_with("coding-instructions:")
                    || (call.name == "read_file" && call.args.get("path").and_then(|v| v.as_str())
                        .is_some_and(|path| path.rsplit(['/', '\\']).next().is_some_and(|name|
                            name.eq_ignore_ascii_case("AGENTS.md") || name.eq_ignore_ascii_case("AGENTS.override.md"))));
                call.output = Some(if instruction_read {
                    "Repository instruction body is turn-local and omitted from the stored tool display. Re-read the current file when needed.".into()
                } else { result.output_text() });
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
impl EngineSessionDriver for CodingSessionDriver {
    async fn run_turn(
        &self,
        message: &SendMessageData,
        cancellation: CancellationToken,
        output: EngineTurnOutput,
    ) -> Result<EngineTurnOutcome, AppError> {
        let projection = Arc::new(TurnProjection {
            host: self.host.clone(), message: message.clone(), output,
            calls: Mutex::new(BTreeMap::new()), terminal: Mutex::new(None),
        });
        let session = self.engine.open_session(self.binding.clone(), self.model.clone(),
            self.tools.clone(), Some(projection.clone())).map_err(contract_error)?;
        let request = self.host.prepare_turn(message, cancellation.clone()).await?;
        if request.principal.principal_kind != "user" || request.principal.principal_id != self.owner_id {
            return Err(AppError::Conflict("Coding turn principal differs from its Session owner".into()));
        }
        let execution = session.run_turn_cancellable(request, cancellation).await;
        let pending = projection.terminal.lock().unwrap_or_else(|e| e.into_inner()).take();
        match execution {
            Ok(result) => {
                let outcome = EngineTurnOutcome {
                    model_steps: result.model_steps,
                    terminal: match result.terminal {
                        CodingTurnTerminal::Completed { finish_reason } => EngineTurnTerminal::Completed { finish_reason },
                        CodingTurnTerminal::Cancelled => EngineTurnTerminal::Cancelled,
                        CodingTurnTerminal::Failed { message } => EngineTurnTerminal::Failed { message },
                    },
                };
                if pending.as_ref() != Some(&coding_terminal(&outcome)) {
                    return Err(AppError::Conflict("Coding result and terminal event disagree or terminal is absent".into()));
                }
                Ok(outcome)
            }
            Err(CodingEngineError::Cancelled) => Ok(EngineTurnOutcome::cancelled(
                match pending { Some(CodingEngineEvent::TurnCancelled { model_steps }) => model_steps, _ => 0 })),
            Err(CodingEngineError::TurnFailed(message)) => {
                let model_steps = match pending {
                    Some(CodingEngineEvent::TurnFailed { model_steps, message: recorded }) if recorded == message => model_steps,
                    _ => return Err(AppError::Conflict("Coding failure has no matching terminal record".into())),
                };
                Ok(EngineTurnOutcome { model_steps, terminal: EngineTurnTerminal::Failed { message } })
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
    async fn record_terminal(&self, message: &SendMessageData, outcome: &EngineTurnOutcome) -> Result<(), AppError> {
        self.host.record_event(message, &coding_terminal(outcome)).await
    }
    async fn cleanup_session(&self) -> Result<(), AppError> { self.host.cleanup_session().await }
}

fn coding_terminal(outcome: &EngineTurnOutcome) -> CodingEngineEvent {
    match &outcome.terminal {
        EngineTurnTerminal::Completed { finish_reason } => CodingEngineEvent::TurnCompleted {
            model_steps: outcome.model_steps, finish_reason: finish_reason.clone(),
        },
        EngineTurnTerminal::Cancelled => CodingEngineEvent::TurnCancelled { model_steps: outcome.model_steps },
        EngineTurnTerminal::Failed { message } => CodingEngineEvent::TurnFailed {
            model_steps: outcome.model_steps, message: message.clone(),
        },
    }
}

#[async_trait]
impl AgentRuntimeControl for CodingAgentRuntime {
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
impl RegisteredAgentRuntime for CodingAgentRuntime {
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
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
        fn capability_activation_snapshot(
            &self,
        ) -> Result<Option<crate::AgentCapabilityActivationSnapshot>, AppError> {
            Ok(None)
        }

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
        let state = AgentRuntimeState::new(SESSION, "projection-workspace", 32);
        let turn = state.reset_for_new_turn(ConversationStatus::Running);
        let projection = TurnProjection {
            host: host.clone(),
            message: message(),
            output: EngineTurnOutput::new(state.clone(), turn),
            calls: Mutex::new(BTreeMap::new()),
            terminal: Mutex::new(None),
        };
        let mut events = state.subscribe();
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
                Arc::new(crate::RuntimeEngineSupport::enabled_only([])),
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
