//! Unified Nomi runtime on the production Conversation owner, Broker and Kernel.
//! No SessionStore, private transcript, provider client or native tool bypass.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::*;
use nomifun_agent_kernel::SessionCapabilityState;
use nomifun_ai_agent::unified_runtime::{UnifiedAgentRuntime, UnifiedRuntimeHost};
use nomifun_ai_agent::types::{AgentRuntimeBuildOptions, SendMessageData};
use nomifun_ai_agent::{RuntimeBuildDescriptor, OfficialRuntimeFactory};
use nomifun_api_types::RuntimeBuildBinding;
use nomifun_chat_model_broker::*;
use nomifun_agent_runtime::*;
use nomifun_common::AppError;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

#[path = "runtime_capabilities.rs"]
mod capabilities;
#[path = "runtime_steering.rs"]
mod steering;
#[path = "runtime_history_port.rs"]
mod history_port;

fn error(value: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Nomi runtime host: {value}"))
}

pub(crate) fn descriptor() -> RuntimeBuildDescriptor {
    RuntimeBuildDescriptor {
        family_id: nomifun_ai_agent::OFFICIAL_NOMI_RUNTIME_FAMILY_ID.into(),
        build_id: format!("{}-host3-adaptive-loop1", env!("CARGO_PKG_VERSION")),
        build_digest: format!(
            "{:x}",
            Sha256::digest(
                concat!(
                    include_str!("../../../../../Cargo.lock"),
                    include_str!("../../Cargo.toml"),
                    include_str!("../../../nomifun-agent-runtime/src/lib.rs"),
                    include_str!("../../../nomifun-public/Cargo.toml"),
                    include_str!("../../../nomifun-agent-control-plane/src/kernel_catalog.rs"),
                    include_str!("../../../nomifun-agent-contracts/src/engine_features.rs"),
                    include_str!("../../../nomifun-agent-contracts/src/runtime.rs"),
                    include_str!("../../../nomifun-agent-contracts/src/package.rs"),
                    include_str!("../../../nomifun-agent-contracts/contracts/engine/platform-feature-inventory.payload.json"),
                    include_str!("agent_wave1_host.rs"),
                    include_str!("../../../nomifun-agent-domain-wave1/src/lib.rs"),
                    include_str!("agent_wave1_companion_host.rs"),
                    include_str!("agent_wave1_memory_receipts.rs"),
                    include_str!("nomi_core_builtins.rs"),
                    include_str!("remote_runtime.rs"),
                    include_str!("../../../nomifun-public/src/canonical.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/engine.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/adaptive.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/error.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/turn.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/kernel.rs"),
                    include_str!("../../../nomifun-agent-kernel/src/compiler.rs"),
                    include_str!("../../../nomifun-agent-kernel/src/plugin.rs"),
                    include_str!("../../../nomifun-agent-kernel/src/session_capabilities.rs"),
                    include_str!("nomi_core_resource_bindings.rs"),
                    include_str!("nomi_core_session.rs"),
                    include_str!("agent_binding_projection.rs"),
                    include_str!("../../../nomifun-api-types/src/agent_platform.rs"),
                    include_str!("../../../nomifun-api-types/src/execution_constraints.rs"),
                    include_str!("../../../nomifun-agent-execution/src/attempt_runner.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/context.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/context_lifecycle.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/output_limit.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/live_context.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/compaction.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/compaction_source.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/agents_md.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/workspace_context.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/search_context.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/workflow.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/patch_recovery.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/planning.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/requirements.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/task_continuation.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/completion.rs"),
                    include_str!("../../../nomifun-agent-domain-wave2/src/lib.rs"),
                    include_str!("../../../nomifun-agent-domain-wave2/src/process_schema.rs"),
                    include_str!("../../../nomifun-agent-domain-wave2/src/workspace_schema.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/history.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/tool.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/tool_dispatch.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/tool_archive.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/history_port.rs"),
                    include_str!("runtime_history_port.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/standard_tools.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/stream_limits.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/tool_context.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/events.rs"),
                    include_str!("../../../nomifun-ai-agent/src/unified_runtime.rs"),
                    include_str!("../../../nomifun-ai-agent/src/engine_sdk.rs"),
                    include_str!("../../../nomifun-ai-agent/src/engine_tasks.rs"),
                    include_str!("../../../nomifun-engine-core/src/lib.rs"),
                    include_str!("../../../nomifun-engine-core/src/error.rs"),
                    include_str!("../../../nomifun-engine-core/src/tool.rs"),
                    include_str!("../../../nomifun-engine-core/src/kernel.rs"),
                    include_str!("../../../nomifun-engine-core/src/process.rs"),
                    include_str!("../../../nomifun-chat-model-broker/src/engine_port.rs"),
                    include_str!("chat_broker_host.rs"),
                    include_str!("../../../nomifun-model-invoke/src/error.rs"),
                    include_str!("../../../nomifun-model-invoke/src/transport.rs"),
                    include_str!("../../../nomifun-model-invoke/src/chat_executor.rs"),
                    include_str!("../../../nomifun-model-invoke/src/chat_bedrock_headers.rs"),
                    include_str!("../../../nomifun-model-invoke/src/chat_sse.rs"),
                    include_str!("../../../nomifun-model-invoke/src/chat_deadline.rs"),
                    include_str!("../../../nomifun-chat-model-broker/src/adapter.rs"),
                    include_str!("../../../nomifun-chat-model-broker/src/broker.rs"),
                    include_str!("../../../nomifun-chat-model-broker/src/provider_errors.rs"),
                    include_str!("../../../nomifun-chat-model-broker/src/responses_decoder.rs"),
                    include_str!("../../../nomifun-chat-model-broker/src/anthropic_decoder.rs"),
                    include_str!("../../../nomifun-chat-model-broker/src/provider_reasoning.rs"),
                    include_str!("../../../nomifun-chat-model-broker/src/wire_budget.rs"),
                    include_str!("../../../nomifun-chat-model-broker/src/contracts.rs"),
                    include_str!("../../../nomifun-chat-model-broker/src/responses_bridge.rs"),
                    include_str!("../../../nomifun-ai-agent/src/runtime_admission.rs"),
                    include_str!("../../../nomifun-ai-agent/src/plugin_tools.rs"),
                    include_str!("../../../nomifun-ai-agent/src/tool_discovery.rs"),
                    include_str!("../../../nomifun-agent-contracts/src/tool_middleware.rs"),
                    include_str!("unified_runtime_history.rs"),
                    include_str!("runtime_patch_recovery.rs"),
                    include_str!("engine_process_host.rs"),
                    include_str!("engine_process_recovery.rs"),
                    include_str!("runtime_event_buffer.rs"),
                    include_str!("agent_tool_surface.rs"),
                    include_str!("runtime_capabilities.rs"),
                    include_str!("runtime_steering.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/steering.rs"),
                    include_str!("../../../nomifun-ai-agent/src/runtime_instance.rs"),
                    include_str!("../../../nomifun-ai-agent/src/runtime_provider.rs"),
                    include_str!("official_runtime.rs"),
                    include_str!("mod.rs"),
                    include_str!("../desktop.rs"),
                    include_str!("../services.rs"),
                    include_str!("../bootstrap/nomi_core.rs"),
                    include_str!("../bootstrap/composition_cleanup.rs"),
                    include_str!("routes.rs"),
                    include_str!("../../../nomifun-ai-agent/src/runtime_sessions.rs"),
                    include_str!("../../../nomifun-ai-agent/src/runtime_session_shutdown.rs"),
                    include_str!("../../../nomifun-ai-agent/src/runtime_session_acquisition.rs"),
                    include_str!("../../../nomifun-ai-agent/src/runtime_handle.rs"),
                    include_str!("runtime_attachments.rs"),
                    include_str!("runtime_skills.rs"),
                    include_str!("engine_skills.rs"),
                    include_str!("../../../nomifun-engine-core/src/context_resource.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/context_resources.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/remote_resources.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/tool_discovery.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/media_context.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/context_tail.rs"),
                    include_str!("../../../nomifun-agent-runtime/src/compacted_history.rs"),
                    include_str!("../../../nomifun-ai-agent/src/model_attachments.rs"),
                    include_str!("nomi_core_wave2.rs"),
                    include_str!("nomi_core_mcp.rs"),
                    include_str!("nomi_core_mcp_resources.rs"),
                    include_str!("../../../nomifun-ai-agent/src/nomi_resources.rs"),
                    include_str!("mcp_effect_receipts.rs"),
                    include_str!("hosted_effect_receipts.rs"),
                    include_str!("engine_plugin_product_tools.rs"),
                    include_str!("engine_plugin_middleware.rs"),
                    include_str!("engine_tool_discovery.rs"),
                    include_str!("engine_robot_tools.rs"),
                    include_str!("engine_browser_tools.rs"),
                    include_str!("../browser_workspace_provider.rs"),
                    include_str!("../../../nomifun-browser-platform/src/product.rs"),
                    include_str!("../../../nomifun-browser-platform/src/bound_resource.rs"),
                    include_str!("../../../nomifun-browser-platform/src/workspace.rs"),
                    include_str!("../../../nomifun-browser-platform/src/run_guard.rs"),
                    include_str!("../../../nomifun-browser-platform/src/runtime.rs"),
                    include_str!("../../../nomifun-browser-platform/src/downloads.rs"),
                    include_str!("../../../nomifun-browser-platform/src/uploads.rs"),
                    include_str!("../../../nomifun-browser-platform/src/attached_browser.rs"),
                    include_str!("nomi_core_robot.rs"),
                    include_str!("../../../nomifun-robot/src/tool_registry.rs"),
                    include_str!("../../../nomifun-robot/src/vision.rs"),
                    include_str!("../../../nomifun-plugin-platform/src/runtime/m1_application.rs"),
                    include_str!("nomi_core_mcp_catalog.rs"),
                    include_str!("plugin_platform.rs"),
                    include_str!("state.rs"),
                    include_str!("../../../nomifun-mcp/src/service.rs"),
                    include_str!("../../../nomifun-mcp/src/identity.rs"),
                    include_str!("../../../nomifun-mcp/src/routes.rs"),
                    include_str!("../../../nomifun-db/src/repository/sqlite_mcp_server.rs"),
                    include_str!("agent_wave2_mcp.rs"),
                    include_str!("../../../nomifun-mcp/src/owner.rs"),
                    include_str!("../../../nomifun-mcp/src/owner_resources.rs"),
                    include_str!("../../../nomifun-mcp/src/owner_resource_template.rs"),
                    include_str!("../../../nomifun-mcp/src/owner_stream.rs"),
                    include_str!("../../../nomifun-mcp/src/owner_legacy_sse.rs"),
                    include_str!("../../../nomifun-mcp/src/owner_stdio.rs"),
                    include_str!("../../../nomifun-mcp/src/owner_discovery.rs"),
                    include_str!("../../../../shared/nomi-process-runtime/src/command_builder.rs"),
                    include_str!("../../../nomifun-mcp/src/connection_test/mod.rs"),
                    include_str!("../../../nomifun-mcp/src/connection_test/protocol.rs"),
                    include_str!("agent_wave2_host.rs"),
                    include_str!("agent_wave2_vcs_push.rs"),
                    include_str!("engine_git_lifecycle.rs"),
                    include_str!("../../../nomifun-file/src/agent_text_read.rs"),
                    include_str!("../../../nomifun-file/src/agent_instruction_scope.rs"),
                    include_str!("../../../nomifun-file/src/agent_patch_lines.rs"),
                    include_str!("../../../nomifun-file/src/agent_patch_source.rs"),
                    include_str!("../../../nomifun-file/src/agent_patch_outcome.rs"),
                    include_str!("../../../nomifun-file/src/service.rs"),
                    include_str!("../../../nomifun-file/src/artifact_store.rs"),
                    include_str!("../../../nomifun-file/src/vcs_stage.rs"),
                    include_str!("../../../nomifun-file/src/agent_text_search.rs"),
                    include_str!("../../../nomifun-file/src/resource.rs"),
                    include_str!("../../../nomifun-file/src/path_safety.rs"),
                    include_str!("../../../nomifun-file/src/snapshot_service/mod.rs"),
                    include_str!("../../../nomifun-file/src/snapshot_service/helpers.rs"),
                    include_str!("unified_runtime_host.rs"),
                    include_str!("engine_session_host.rs"),
                    include_str!("engine_journal.rs"),
                    include_str!("engine_model_facts.rs"),
                    include_str!("engine_tool_host.rs"),
                    include_str!("engine_kernel_session.rs"),
                    include_str!("../../../nomifun-ai-agent/src/engine_effect_scope.rs"),
                    include_str!("../../../nomifun-agent-session/src/store.rs"),
                    include_str!("engine_mcp_resources.rs"),
                    include_str!("engine_mcp_media.rs"),
                    include_str!("engine_workspace_media.rs"),
                    include_str!("engine_creation_tools.rs"),
                    include_str!("automatic_creation_route.rs"),
                    include_str!("workspace_file_read.rs"),
                    include_str!("engine_history.rs")
                )
                .as_bytes()
            )
        ),
        display_name: "Nomi".into(),
        host_contract_version: nomifun_api_types::RUNTIME_HOST_CONTRACT_VERSION,
        supported_profiles: vec!["default".into()],
    }
}

pub(crate) fn factory(
    session_host: Arc<super::engine_session_host::EngineSessionHost>,
    plugin_schemas: Arc<dyn nomifun_ai_agent::NomiPluginToolSchemaResolver>,
    platform_builtin_schemas: Arc<
        dyn nomifun_ai_agent::NomiPlatformBuiltinToolSchemaResolver,
    >,
    host_dynamic_capability_ids: BTreeSet<CapabilityId>,
    supervision: Arc<dyn nomifun_idmm::IdmmProgressSink>,
) -> OfficialRuntimeFactory {
    let host_dynamic_capability_ids = Arc::new(host_dynamic_capability_ids);
    Arc::new(move |options, binding| {
        let plugin_schemas = plugin_schemas.clone();
        let platform_builtin_schemas = platform_builtin_schemas.clone();
        let host_dynamic_capability_ids = Arc::clone(&host_dynamic_capability_ids);
        let session_host = session_host.clone();
        let supervision = supervision.clone();
        Box::pin(async move {
            let admitted = session_host.resolve(&options, &binding).await?;
            super::agent_tool_surface::validate_session_mcp(admitted.snapshot(), &admitted.agent_binding().typed_resource_bindings, &admitted.session().extra)?;
            let principal = admitted.principal().clone();
            let route = admitted.snapshot()
                .content
                .chat_route_identity
                .clone()
                .ok_or_else(|| error("snapshot has no exact Chat route"))?;
            let session_id = AgentSessionId::from(options.conversation_id.clone());
            let route_image_input = admitted.revision().payload.chat_route_records.get(&route.model_task)
                .is_some_and(|record| std::iter::once(&record.primary).chain(record.failovers.iter())
                    .any(|candidate| candidate.features.contains(&ChatRouteFeature::ImageInput)));
            let resources = session_host.open_kernel_session(&admitted)?;
            let compiled = resources.compiled().clone();
            let active = resources.active_state().clone();
            // Compile the exact selected surface once. This preview does not
            // mutate the Kernel active set or expose inactive tools to a model.
            let preview = active.snapshot().map_err(error)?;
            let registry = resources.registry_snapshot()?;
            let full_plan = super::agent_tool_surface::compile(
                &compiled,
                &preview,
                &registry,
                plugin_schemas.as_ref(),
                platform_builtin_schemas.as_ref(),
                host_dynamic_capability_ids.as_ref(),
            )
            .await?;
            let full_plan = resources.compile_tool_plan(full_plan.model_definitions().into_iter().map(|definition| {
                let binding = full_plan.binding(&definition.name).expect("compiled definition has a binding");
                nomifun_engine_core::EngineToolExposure {
                    definition, capability_id: binding.capability_id.clone(), action_id: binding.action_id.clone(),
                }
            }))?;
            let full_plan = full_plan.merged(&resources.plugin_product_tool_plan().await?).map_err(error)?;
            let full_plan = full_plan.merged(&resources.robot_tool_plan().await?).map_err(error)?;
            if full_plan.len() > 128 { return Err(error("Nomi tool surface exceeds 128 actions")); }
            let skills = session_host.read_selected_skills(&admitted).await?;
            let tools = Arc::new(JoinedTools(resources.install_tools(full_plan.clone(), Arc::new(RuntimeToolObservation))?));
            let build = AgentEngineBuild {
                build_id: binding.build_id.clone().into(),
                build_digest: binding.build_digest.clone().into(),
            };
            let engine = Arc::new(AgentEngine::new(build).map_err(error)?
                .with_context_budget(AgentContextBudget { max_context_bytes: 12 * 1024 * 1024, ..Default::default() }).map_err(error)?);
            let engine_binding = EngineBinding::new(
                session_id,
                RuntimeBindingId::from(format!("conversation-runtime:{}", options.conversation_id)),
                binding.build_id.clone().into(),
                binding.build_digest.clone().into(),
                compiled.snapshot_ref().clone(),
            )
            .map_err(error)?;
            let host = Arc::new_cyclic(|weak| ConversationRuntimeHost {
                session_host,
                options: options.clone(),
                binding: binding.clone(),
                engine_binding: engine_binding.clone(),
                snapshot_ref: compiled.snapshot_ref().clone(),
                route,
                route_image_input,
                full_plan,
                compiled: compiled.clone(),
                capability_port: Arc::new(capabilities::HostPort(weak.clone())),
                input_port: Arc::new(steering::HostPort(weak.clone())),
                capability_transition: tokio::sync::Mutex::new(()),
                activation_failed: false.into(),
                skills,
                principal,
                capability_state: active,
                active: tokio::sync::Mutex::new(None),
                last_terminal_root: std::sync::Mutex::new(None),
                tools: tools.clone(),
                resources: resources.clone(),
                supervision,
            });
            let model = host.session_host.compose_model_port(host.clone())?;
            let model = resources.wrap_model_middleware(model)?;
            let runtime =
                UnifiedAgentRuntime::new(&options, engine, engine_binding, model, tools, host)?;
            Ok(Arc::new(runtime) as Arc<dyn nomifun_ai_agent::OfficialAgentRuntime>)
        })
    })
}

struct ActiveTurn {
    root: String,
    wire_id: String,
    steering: steering::Inbox,
    operation: String,
    epoch: i64,
    journal: super::engine_journal::EngineTurnJournal,
    cleanup_started: bool,
    cleanup_proven: bool,
    cancellation: CancellationToken,
    event_buffer: super::runtime_event_buffer::AgentEventBuffer,
    assistant_text_by_step: BTreeMap<u16, String>,
}

struct ConversationRuntimeHost {
    session_host: Arc<super::engine_session_host::EngineSessionHost>,
    options: AgentRuntimeBuildOptions,
    binding: RuntimeBuildBinding,
    engine_binding: EngineBinding,
    snapshot_ref: ResolvedSnapshotRef,
    route: ChatRouteSelection,
    route_image_input: bool,
    full_plan: AgentToolPlan,
    compiled: Arc<nomifun_agent_kernel::CompiledSnapshot>,
    capability_port: Arc<capabilities::HostPort>,
    input_port: Arc<steering::HostPort>,
    capability_transition: tokio::sync::Mutex<()>,
    activation_failed: std::sync::atomic::AtomicBool,
    skills: super::runtime_skills::SelectedSkills,
    principal: PrincipalRef,
    capability_state: Arc<SessionCapabilityState>,
    active: tokio::sync::Mutex<Option<ActiveTurn>>,
    last_terminal_root: std::sync::Mutex<Option<String>>,
    tools: Arc<JoinedTools>,
    resources: Arc<super::engine_kernel_session::EngineKernelSession>,
    supervision: Arc<dyn nomifun_idmm::IdmmProgressSink>,
}

impl ConversationRuntimeHost {
    fn root<'a>(&self, message: &'a SendMessageData) -> &'a str {
        message
            .source_message_id
            .as_deref()
            .unwrap_or(&message.msg_id)
    }

    fn terminal_already_recorded(&self, root: &str) -> Result<bool, AppError> {
        self.last_terminal_root
            .lock()
            .map(|last| last.as_deref() == Some(root))
            .map_err(|_| error("terminal root state poisoned"))
    }

    /// Acquire the canonical receipt/journal and publish in-memory ownership
    /// before opening any resource or doing further awaited preparation. If a
    /// later preparation step fails or is cancelled, cleanup still has the
    /// exact root needed to settle resources and append a terminal.
    async fn admit_preparation(
        &self,
        message: &SendMessageData,
        cancellation: CancellationToken,
    ) -> Result<super::engine_session_host::EngineTurnReceipt, AppError> {
        let root = self.root(message);
        let admitted = self.session_host.read_turn_receipt(
            &self.options,
            &self.binding,
            &self.snapshot_ref,
            message,
        ).await?;
        let operation = admitted.operation_id().to_owned();
        let epoch = admitted.admission_epoch();
        let journal = self.session_host.open_journal(&admitted, cancellation.clone())?;
        let mut active = self.active.lock().await;
        if active.is_some() {
            return Err(error("previous turn has not reached its recorded terminal"));
        }
        *active = Some(ActiveTurn {
            root: root.into(),
            wire_id: message.msg_id.clone(),
            steering: Default::default(),
            operation,
            epoch,
            journal: journal.clone(),
            cleanup_started: false,
            cleanup_proven: false,
            cancellation,
            event_buffer: Default::default(),
            assistant_text_by_step: BTreeMap::new(),
        });
        drop(active);
        // EngineKernelSession retains its own partial-open state before any
        // owner can fail, so leaving ActiveTurn installed is intentional.
        self.resources.open_turn(&admitted, journal)?;
        Ok(admitted)
    }

    async fn append_record(
        &self,
        root: &str,
        payload: String,
        model_operation: Option<&str>,
        terminal: bool,
    ) -> Result<(), AppError> {
        let mut active = self.active.lock().await;
        let turn = active
            .as_mut()
            .ok_or_else(|| error("event without admitted turn"))?;
        if root != turn.root {
            return Err(error("event root mismatch"));
        }
        self.append_locked_record(turn, payload, model_operation, terminal).await?;
        if terminal {
            *self
                .last_terminal_root
                .lock()
                .map_err(|_| error("terminal root state poisoned"))? = Some(root.to_owned());
            *active = None;
        }
        Ok(())
    }

    async fn append_locked_record(&self, turn: &mut ActiveTurn, payload: String, model_operation: Option<&str>, terminal: bool) -> Result<(), AppError> {
        use super::engine_journal::EngineJournalWrite;
        let kind = if terminal { EngineJournalWrite::Terminal }
            else if turn.cleanup_started { EngineJournalWrite::Cleanup }
            else { EngineJournalWrite::Progress };
        turn.journal.append(payload, model_operation.map(str::to_owned), kind).await
    }
}

#[async_trait]
impl UnifiedRuntimeHost for ConversationRuntimeHost {
    async fn admit_tool(&self, message: &SendMessageData, event: &AgentEngineEvent) -> Result<bool, AppError> {
        let admitted = self.admit_steerable_tool(message, event).await?;
        if admitted {
            let operation = self.active.lock().await.as_ref().map(|turn| turn.operation.clone());
            self.supervision.note_progress(
                &self.options.conversation_id,
                operation.as_deref(),
                nomifun_idmm::IdmmProgressPhase::Tool,
            );
        }
        Ok(admitted)
    }

    async fn queue_steer(&self, delivery: nomifun_ai_agent::RuntimeSteerDelivery) -> Result<bool, AppError> {
        self.accept_steer(delivery).await
    }
    fn capability_activation_snapshot(
        &self,
    ) -> Result<Option<nomifun_ai_agent::AgentCapabilityActivationSnapshot>, AppError> {
        if self.activation_failed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(error("activation state is not yet durably reconciled"));
        }
        let snapshot = self.capability_state.snapshot().map_err(error)?;
        Ok(Some(nomifun_ai_agent::AgentCapabilityActivationSnapshot {
            resolved_snapshot_ref: snapshot.resolved_snapshot_ref,
            generation: snapshot.generation,
            active_capability_ids: snapshot.active.into_iter().map(|id| id.as_ref().to_owned()).collect(),
        }))
    }

    async fn prepare_turn(
        &self,
        message: &SendMessageData,
        cancellation: CancellationToken,
    ) -> Result<AgentTurnRequest, AppError> {
        let root = message
            .source_message_id
            .as_deref()
            .unwrap_or(&message.msg_id);
        let admitted = self.admit_preparation(message, cancellation.clone()).await?;
        let patch_recovery = super::runtime_patch_recovery::load(
            self.session_host.as_ref(),
            &admitted,
            &self.snapshot_ref,
        )
        .await?;
        // Refresh platform facts each turn. Unknown-limit fallback and output
        // reservation are explicit runtime policies, not platform defaults.
        let facts = self.session_host.read_model_facts(admitted.session()).await?;
        let (context, output) = facts.envelope_with_unknown_policy(super::engine_model_facts::EngineModelLimits {
            context_tokens: Some(32_768), output_tokens: Some(4096),
        }).ok_or_else(|| error("model limits cannot support Nomi context policy"))?;
        let model_budget = AgentModelBudget::from_limits(Some(context), Some(output)).map_err(error)?;
        let operation = admitted.operation_id().to_owned();
        let response = admitted.session().session();
        let receipt = admitted.request_payload();
        self.skills.validate_extra(&response.extra)?;
        self.skills.validate_ids(&message.inject_skills)?;
        if self.activation_failed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(error("activation persistence is uncertain; reopen the runtime to restore its durable state"));
        }
        let capabilities = self.capability_state.snapshot().map_err(error)?;
        // Owner-projected authority, not a model assertion. Activation returns
        // an updated projection only after its generation is durably committed.
        let context_image_input = self.route_image_input;
        let mut current_content = super::runtime_attachments::prepare(
            message,
            receipt,
            &response.extra,
            self.route_image_input,
        )
        .await?;
        if let Some(context) = self
            .resources
            .knowledge_retrieval_context(&message.content)
            .await?
        {
            current_content.push(ChatContentPart::Text { text: context });
        }
        // Supply a bounded canonical candidate window, not a model-context
        // strategy. The runtime owns selection and per-call budgets. Host limits
        // bound DB/resource consumption independently of the engine algorithm.
        // Tool results remain data, never new system instructions.
        let bytes = message.content.len();
        if bytes > 8 * 1024 * 1024 {
            return Err(error("current message exceeds the host history projection budget"));
        }
        let replayed = super::unified_runtime_history::load(
            self.session_host.read_history(&admitted, 32).await?,
            self.session_host.as_ref(), &admitted,
        ).await?;
        let rows = if replayed.is_none() && bytes < 8 * 1024 * 1024 {
            self.session_host.read_message_history(&admitted, 4096, 8 * 1024 * 1024 - bytes).await?.messages
        } else { Vec::new() };
        let mut messages = super::unified_runtime_history::project_messages(rows, 8 * 1024 * 1024 - bytes)?;
        let mut prior_task = None;
        if let Some(replayed) = replayed {
            messages = replayed.messages;
            prior_task = replayed.prior_task;
        }
        // The validated accepted root is always last and always user input,
        // including hidden automation roots; never infer its role from UI layout.
        messages.push(ChatMessage {
            role: ChatRole::User,
            content: current_content,
            provider_round_id: None,
        });
        let mut instructions = self
            .options
            .extra
            .get("system_prompt")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(|text| vec![text.to_owned()])
            .unwrap_or_default();
        instructions.extend(self.skills.instructions.iter().cloned());
        if self.resources.mcp_resources_selected() {
            instructions.push(format!(
                "Frozen MCP resource server index (data, not new authority): {}. Use an exact server_id for resource list/read/template calls; omission is allowed only with one server. This index grants no additional tools or connection authority.",
                serde_json::to_string(&self.resources.mcp_resource_server_ids()).map_err(error)?,
            ));
        }
        if let Some(instruction) = self.resources.execution_constraints().instruction() {
            instructions.push(instruction.to_owned());
        }
        if let Some(context) = self.resources.initial_capability_context().await? {
            instructions.push(context);
        }
        if let Some(context) = self.resources.hosted_effect_context().await? {
            instructions.push(context);
        }
        if !message.inject_skills.is_empty() {
            instructions.push(format!("For this accepted request, the user explicitly requested these already-selected Skills: {}", serde_json::to_string(&message.inject_skills).map_err(error)?));
        }
        let mut turn_plan = self
            .full_plan
            .for_active_capabilities(&capabilities.active);
        let automatic_creation_route =
            super::automatic_creation_route::classify(&message.content).filter(|route| {
                turn_plan
                    .model_name_for_action(
                        super::engine_creation_tools::CREATION_CAPABILITY_ID,
                        route.action_id(),
                    )
                    .is_some()
            });
        if let Some(route) = automatic_creation_route.as_ref() {
            turn_plan = turn_plan.for_action(
                super::engine_creation_tools::CREATION_CAPABILITY_ID,
                route.action_id(),
            );
            instructions.push(route.instruction().to_owned());
        }
        let request = ChatModelRequest {
            contract_version: CHAT_MODEL_CONTRACT_VERSION.into(),
            causality: ChatCausality {
                agent_session_id: self.options.conversation_id.clone().into(),
                turn_operation_id: operation.clone().into(),
                causation_event_id: root.into(),
                resolved_snapshot_ref: self.snapshot_ref.clone(),
                route_identity: self.route.clone(),
                operation_id: operation.into(),
            },
            route: self.route.clone(),
            input: ChatModelInput {
                instructions,
                messages,
                tools: Vec::new(),
                tool_choice: ChatToolChoice::Auto,
                max_output_tokens: None,
                reasoning: None,
                prompt_cache: PromptCachePolicy::Disabled,
                response_format: ChatResponseFormat::Text,
                requested_output_modalities: BTreeSet::new(),
                provider_round_parent: None,
                preserve_native_responses_items: false,
                metadata: Default::default(),
            },
        };
        let mut request = AgentTurnRequest::new(
            request,
            turn_plan,
            self.principal.clone(),
            capabilities.generation,
        ).with_model_budget(model_budget).with_context_resources(self.skills.resources.clone())
            .with_context_image_input(context_image_input)
            .with_prior_task(prior_task)
            .with_patch_recovery(patch_recovery)
            .with_input_port(self.input_port.clone());
        if self.resources.mcp_resources_selected() {
            request = request.with_resource_port(self.capability_port.clone());
        }
        if let Some(port) = self.resources.tool_discovery_port()? {
            request = request.with_tool_discovery_port(port);
        }
        if self.compiled.content().enabled_capabilities.iter().any(|item| {
            item.capability.id.as_ref() == nomifun_agent_domain_wave4::ROBOT_MODULE_ID
                && item.action_allowlist.contains(
                    &nomifun_agent_contracts::ActionId::from(
                        nomifun_agent_domain_wave4::ROBOT_VISION_ACTION_ID,
                    ),
                )
        }) {
            request = request.with_live_context_port(self.capability_port.clone());
        }
        request = request.with_history_port(Arc::new(history_port::HistoryPort {
            host: self.session_host.clone(), receipt: admitted, cancellation,
        }));
        Ok(request)
    }

    async fn record_event(
        &self,
        message: &SendMessageData,
        event: &AgentEngineEvent,
    ) -> Result<(), AppError> {
        let terminal_event = matches!(
            event,
            AgentEngineEvent::TurnCompleted { .. }
                | AgentEngineEvent::TurnCancelled { .. }
                | AgentEngineEvent::TurnFailed { .. }
        );
        let root = self.root(message);
        let operation = self
            .active
            .lock()
            .await
            .as_ref()
            .map(|turn| turn.operation.clone());
        let writeback_input = {
            let mut active = self.active.lock().await;
            active.as_mut().and_then(|turn| {
                if let AgentEngineEvent::OutputTextDelta { step, text } = event {
                    turn.assistant_text_by_step
                        .entry(*step)
                        .or_default()
                        .push_str(text);
                }
                // Non-human roots (cron, channel, AutoWork, IDMM) do not
                // become durable owner Knowledge through this desktop-chat
                // policy. Their domains need an explicit write authority.
                (matches!(event, AgentEngineEvent::TurnCompleted { .. })
                    && message.origin.as_deref().is_none_or(str::is_empty))
                .then(|| {
                    let assistant = turn
                        .assistant_text_by_step
                        .iter()
                        .rev()
                        .find_map(|(_, text)| (!text.trim().is_empty()).then(|| text.clone()))
                        .unwrap_or_default();
                    (message.content.clone(), assistant)
                })
            })
        };
        if let Some((user_text, assistant_text)) = writeback_input {
            match self
                .resources
                .finalize_knowledge_writeback(
                    user_text,
                    assistant_text,
                    self.options.model.clone(),
                )
                .await
            {
                Ok(Some(report)) => {
                    tracing::info!(
                        agent_session_id = %self.options.conversation_id,
                        status = ?report.status,
                        candidates = report.candidates,
                        written = report.written.len(),
                        failures = report.failures.len(),
                    "turn-final Knowledge write-back completed"
                    )
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(
                    agent_session_id = %self.options.conversation_id,
                    %error,
                    "turn-final Knowledge write-back could not run"
                ),
            }
        }
        if terminal_event && self.active.lock().await.is_none() {
            // No canonical receipt could be re-resolved during cleanup. No
            // Runtime resource was opened, but the Hosted SDK still requires
            // one explicit terminal acknowledgement instead of quarantining
            // the transport for a missing in-memory ActiveTurn.
            *self
                .last_terminal_root
                .lock()
                .map_err(|_| error("terminal root state poisoned"))? = Some(root.to_owned());
            self.supervision.note_progress(
                &self.options.conversation_id,
                operation.as_deref(),
                nomifun_idmm::IdmmProgressPhase::Terminal,
            );
            return Ok(());
        }
        if matches!(event, AgentEngineEvent::TurnInputScope { .. } | AgentEngineEvent::SteeringInputs { .. } | AgentEngineEvent::SteeringDeferred { .. }) {
            return Err(error("control records must be committed by the platform owner"));
        }
        if matches!(event, AgentEngineEvent::ToolStarted { step, .. } if *step > 0) {
            return Err(error("model ToolStarted must use atomic tool admission"));
        }
        if let AgentEngineEvent::PatchRecoveryUpdated { state } = event {
            state.validate().map_err(error)?;
        }
        let records = {
            let mut active = self.active.lock().await;
            let turn = active.as_mut().ok_or_else(|| error("event without admitted turn"))?;
            if turn.root != message.source_message_id.as_deref().unwrap_or(&message.msg_id) {
                return Err(error("event root differs from admitted turn"));
            }
            if terminal_event && !turn.cleanup_proven {
                return Err(error("terminal requires the durable cleanup witness"));
            }
            turn.event_buffer.project(event)
        };
        for event in &records {
        if let AgentEngineEvent::ContextCompacted { retained_context: Some(items), .. } = event {
            // The host projection can change descriptor lengths. Reject an
            // oversized replacement before persisting it or acknowledging the
            // engine's write-ahead event; never store a checkpoint replay will
            // reject merely because projection expanded an omission notice.
            if serde_json::to_vec(items).map_err(error)?.len() > 2 * 1024 * 1024 {
                return Err(error("projected compaction replacement exceeds replay budget"));
            }
        }
        let model_operation = match event {
            AgentEngineEvent::ModelStepStarted { operation_id, .. }
                | AgentEngineEvent::CompactionStarted { operation_id, .. } => Some(operation_id.as_ref()),
            _ => None,
        };
        let payload = serde_json::to_string(event).map_err(error)?;
        let terminal = matches!(
            event,
            AgentEngineEvent::TurnCompleted { .. }
                | AgentEngineEvent::TurnCancelled { .. }
                | AgentEngineEvent::TurnFailed { .. }
        );
        self.append_record(
            message
                .source_message_id
                .as_deref()
                .unwrap_or(&message.msg_id),
            payload,
            model_operation,
            terminal,
        )
        .await?;
        if matches!(event, AgentEngineEvent::TurnStarted { .. }) {
            self.open_steering().await?;
        }
        if let AgentEngineEvent::ToolCompleted { result, .. } = event {
            self.tools.mark_observed(result.call_id.as_ref())?;
        }
        }
        let phase = match event {
            AgentEngineEvent::ToolStarted { .. }
            | AgentEngineEvent::ToolCompleted { .. } => {
                nomifun_idmm::IdmmProgressPhase::Tool
            }
            AgentEngineEvent::TurnCompleted { .. }
            | AgentEngineEvent::TurnCancelled { .. }
            | AgentEngineEvent::TurnFailed { .. } => nomifun_idmm::IdmmProgressPhase::Terminal,
            AgentEngineEvent::ModelStepStarted { .. }
            | AgentEngineEvent::OutputTextDelta { .. }
            | AgentEngineEvent::ReasoningDelta { .. }
            | AgentEngineEvent::ToolResultsOrdered { .. } => {
                nomifun_idmm::IdmmProgressPhase::Model
            }
            _ => nomifun_idmm::IdmmProgressPhase::Other,
        };
        self.supervision.note_progress(
            &self.options.conversation_id,
            operation.as_deref(),
            phase,
        );
        Ok(())
    }

    async fn cleanup_turn(&self, message: &SendMessageData) -> Result<(), AppError> {
        use super::engine_journal::EngineJournalWrite;

        let root = self.root(message);
        if self.terminal_already_recorded(root)? {
            return Ok(());
        }
        if self.active.lock().await.is_none() {
            // The shared SDK may select cancellation before polling run_turn.
            // Re-resolve the already accepted root so cleanup/terminal still
            // use canonical authority. Failure means no Runtime-owned resource
            // could have been opened, so cleanup remains an idempotent no-op.
            let _ = self
                .admit_preparation(message, CancellationToken::new())
                .await;
        }
        {
            let mut active = self.active.lock().await;
            let Some(turn) = active.as_mut() else {
                return Ok(());
            };
            if turn.root != root {
                return Err(error("cleanup targets a different accepted root"));
            }
            if turn.journal.sequence() == 0 {
                let started = serde_json::to_string(&AgentEngineEvent::TurnStarted {
                    binding: self.engine_binding.clone(),
                    turn_operation_id: turn.operation.clone().into(),
                })
                .map_err(error)?;
                turn.journal
                    .append(started, None, EngineJournalWrite::Cleanup)
                    .await?;
                let input_scope = serde_json::to_string(&AgentEngineEvent::TurnInputScope {
                    wire_turn_id: turn.wire_id.clone(),
                })
                .map_err(error)?;
                turn.journal
                    .append(input_scope, None, EngineJournalWrite::Cleanup)
                    .await?;
            }
        }
        // Always attempt owned-effect cleanup even if inbox journaling fails.
        let steering = nomifun_ai_agent::engine_effect_scope::guard_effect_settlement(|| self.close_steering()).await;
        self.resources.cleanup_turn(root).await?;
        steering?;
        // Persist the last partial response before the cleanup witness, so a
        // crash between cleanup and terminal publication retains its text.
        let mut pending = Vec::new();
        if let Some(turn) = self.active.lock().await.as_mut() {
            turn.cleanup_started = true;
            turn.event_buffer.flush(&mut pending);
        }
        for event in pending {
            self.append_record(message.source_message_id.as_deref().unwrap_or(&message.msg_id),
                serde_json::to_string(&event).map_err(error)?, None, false).await?;
        }
        // Joined tool tasks have already persisted every settlement before this cleanup witness.
        self.tools.discard_closed_observations()?;
        let mut active = self.active.lock().await;
        let turn = active
            .as_mut()
            .ok_or_else(|| error("cleanup lost its active turn authority"))?;
        if turn.cleanup_proven {
            return Ok(());
        }
        let payload = serde_json::json!({"event":"host_cleanup_proven", "binding":self.binding,
            "epoch":turn.epoch, "operation":turn.operation}).to_string();
        self.append_locked_record(turn, payload, None, false).await?;
        turn.cleanup_proven = true;
        Ok(())
    }
    async fn cleanup_session(&self) -> Result<(), AppError> {
        self.resources.cleanup_session().await
    }
}

#[async_trait]
impl ChatCausalityGate for ConversationRuntimeHost {
    async fn authorize(&self, causality: &ChatCausality) -> Result<(), ChatModelError> {
        let reject = |reason: &str| {
            ChatModelError::new(
                ChatModelErrorCode::CausalityRejected,
                reason,
                ChatRetryDirective::Never,
            )
        };
        let active = self.active.lock().await;
        let turn = active
            .as_ref()
            .ok_or_else(|| reject("no active Nomi turn"))?;
        if self.activation_failed.load(std::sync::atomic::Ordering::Acquire)
            || turn.cancellation.is_cancelled()
            || causality.agent_session_id.as_ref() != self.options.conversation_id
            || causality.turn_operation_id.as_ref() != turn.operation
            || causality.causation_event_id.as_ref() != turn.root
            || causality.resolved_snapshot_ref != self.snapshot_ref
            || causality.route_identity != self.route
        {
            return Err(reject(
                "Nomi request differs from its admitted Conversation authority",
            ));
        }
        turn.journal.authorize(causality).await
    }
}

/// The runtime supplies only its instruction-observation policy; tool lifetime,
/// duplicate fencing and durable dispatch/settlement belong to the shared host.
struct JoinedTools(Arc<super::engine_tool_host::EngineToolHost>);
impl std::ops::Deref for JoinedTools {
    type Target = super::engine_tool_host::EngineToolHost;
    fn deref(&self) -> &Self::Target { &self.0 }
}
struct RuntimeToolObservation;
impl super::engine_tool_host::EngineToolObservationPolicy for RuntimeToolObservation {
    fn project(&self, invocation: &AgentToolInvocation, result: &AgentToolResult) -> AgentToolResult {
        if super::runtime_event_buffer::is_instruction_read(&invocation.call) {
            super::runtime_event_buffer::instruction_result(result)
        } else {
            super::engine_tool_host::bounded_engine_tool_result(result)
        }
    }
}
#[async_trait]
impl AgentToolInvoker for JoinedTools {
    async fn invoke(&self, invocation: AgentToolInvocation, cancellation: CancellationToken) -> Result<AgentToolResult, AgentEngineError> {
        nomifun_engine_core::EngineToolInvoker::invoke(self.0.as_ref(), invocation, cancellation).await.map_err(Into::into)
    }
}

#[cfg(test)]
mod build_identity_tests {
    #[test]
    fn runtime_digest_covers_module_roots_and_wave_two_effect_owners() {
        let source = include_str!("unified_runtime_host.rs");
        for required in [
            "../../../nomifun-agent-runtime/src/lib.rs",
            "mod.rs",
            "../../../nomifun-agent-contracts/src/package.rs",
            "../../../nomifun-agent-kernel/src/plugin.rs",
            "../../../nomifun-agent-domain-wave1/src/lib.rs",
            "../../../nomifun-agent-domain-wave2/src/lib.rs",
            "../../../nomifun-ai-agent/src/plugin_tools.rs",
            "../../../nomifun-ai-agent/src/tool_discovery.rs",
            "engine_plugin_middleware.rs",
            "engine_tool_discovery.rs",
            "../../../nomifun-agent-runtime/src/tool_discovery.rs",
            "../../../nomifun-mcp/src/identity.rs",
            "../../../nomifun-file/src/artifact_store.rs",
            "../../../nomifun-file/src/vcs_stage.rs",
            "../../../nomifun-agent-session/src/store.rs",
        ] {
            assert!(
                source.contains(&format!("include_str!(\"{required}\")")),
                "Runtime build digest omitted {required}"
            );
        }
    }
}
