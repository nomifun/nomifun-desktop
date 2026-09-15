//! Trusted, open runtime composition underneath the existing Session owner.
use std::sync::{Arc, Mutex, OnceLock};

use nomifun_ai_agent::runtime_registry::AgentRuntimeFactory;
use nomifun_ai_agent::{
    AgentRuntimeHandle, RuntimeEngineAdmission, RuntimeEngineCatalog, RuntimeEngineDescriptor,
    RuntimeEngineFactory, RuntimeEngineSupport,
};
use nomifun_api_types::{RUNTIME_ENGINE_BINDING_KEY, RuntimeEngineBinding, RuntimeEngineSelector};
use nomifun_common::AppError;
use sha2::{Digest, Sha256};

/// A compiled-in engine receives real, immutable Session facts before it
/// constructs its driver. The Session host also resolves accepted turn
/// receipts; model/effect/history adapters remain explicitly host-composed.
pub type SessionEngineDriverFactory = Arc<
    dyn Fn(
        nomifun_ai_agent::types::AgentRuntimeBuildOptions,
        super::engine_session_host::AdmittedEngineSession,
        Arc<super::engine_session_host::EngineSessionHost>,
    ) -> futures_util::future::BoxFuture<'static, Result<Arc<dyn nomifun_ai_agent::engine_sdk::EngineSessionDriver>, AppError>>
    + Send + Sync
>;

/// Only engines compiled into this application may be registered, before router
/// assembly. No post-package mounting, executable uploads or hot registration.
#[derive(Default)]
pub struct RuntimeEngineHost {
    catalog: OnceLock<Arc<RuntimeEngineCatalog>>,
    extensions: Mutex<RuntimeEngineCatalog>,
    nomi_factory: OnceLock<AgentRuntimeFactory>,
    session_host: OnceLock<Arc<super::engine_session_host::EngineSessionHost>>,
    restart_recovery: Mutex<nomifun_conversation::terminal_proof::RegisteredEngineRecoveryMap>,
}

impl RuntimeEngineHost {
    /// Compile-time convenience over register_hosted that performs the same
    /// durable Session admission as official Coding, before calling the
    /// extension's driver constructor. No runtime upload/installation route.
    pub fn register_session_hosted(
        self: &Arc<Self>, descriptor: RuntimeEngineDescriptor,
        factory: SessionEngineDriverFactory, admission: Arc<dyn RuntimeEngineAdmission>,
    ) -> Result<(), AppError> {
        let engines = Arc::downgrade(self);
        self.register_hosted(descriptor, Arc::new(move |options, binding| {
            let engines = engines.clone();
            let factory = factory.clone();
            Box::pin(async move {
                let host = engines.upgrade().ok_or_else(|| AppError::Conflict("Runtime host has shut down".into()))?.session_host()?;
                let session = host.resolve(&options, &binding).await?;
                factory(options, session, host).await
            })
        }), admission)
    }

    pub(crate) fn install_session_host(&self, host: Arc<super::engine_session_host::EngineSessionHost>) -> Result<(), AppError> {
        let _registration = self.extensions.lock().map_err(|_| AppError::Internal("Runtime registration lock poisoned".into()))?;
        if self.catalog.get().is_some() {
            return Err(AppError::Conflict("Session host assembly is closed".into()));
        }
        self.session_host.set(host).map_err(|_| AppError::Conflict("Engine Session host already installed".into()))
    }

    /// Actual product Session facts; no Coding profile or event codec required.
    /// Call from a compiled-in driver factory after application assembly.
    pub fn session_host(&self) -> Result<Arc<super::engine_session_host::EngineSessionHost>, AppError> {
        self.session_host.get().cloned().ok_or_else(|| AppError::Conflict("Engine Session host is not assembled".into()))
    }

    /// Source-integrated engines may share lifecycle/task ownership without
    /// implementing AgentRuntimeControl. The driver factory must still obtain
    /// exact, admitted platform ports; this helper grants no model/tool access.
    pub fn register_hosted(
        &self,
        descriptor: RuntimeEngineDescriptor,
        factory: nomifun_ai_agent::engine_sdk::EngineDriverFactory,
        admission: Arc<dyn RuntimeEngineAdmission>,
    ) -> Result<(), AppError> {
        self.register(descriptor, nomifun_ai_agent::engine_sdk::hosted_engine_factory(factory), admission)
    }

    /// Optional, compile-time recovery extension for an exact engine build.
    /// Register during assembly, never from a packaged runtime upload.
    pub fn register_restart_recovery(&self, descriptor: &RuntimeEngineDescriptor,
        recovery: Arc<dyn nomifun_conversation::terminal_proof::RegisteredEngineRestartRecovery>) -> Result<(), AppError> {
        descriptor.validate()?;
        let _registration = self.extensions.lock().map_err(|_| AppError::Internal("Runtime registration lock poisoned".into()))?;
        if self.catalog.get().is_some() { return Err(AppError::Conflict("Runtime recovery registration is closed".into())); }
        let key = (descriptor.family_id.clone(), descriptor.build_id.clone(), descriptor.build_digest.clone());
        let mut hooks = self.restart_recovery.lock().map_err(|_| AppError::Internal("Runtime recovery lock poisoned".into()))?;
        if hooks.contains_key(&key) { return Err(AppError::Conflict("Runtime recovery already registered".into())); }
        hooks.insert(key, recovery);
        Ok(())
    }

    pub(crate) fn restart_recovery_hooks(&self) -> Result<nomifun_conversation::terminal_proof::RegisteredEngineRecoveryMap, AppError> {
        self.restart_recovery.lock().map(|hooks| hooks.clone()).map_err(|_| AppError::Internal("Runtime recovery lock poisoned".into()))
    }

    pub fn register(
        &self,
        descriptor: RuntimeEngineDescriptor,
        factory: RuntimeEngineFactory,
        admission: Arc<dyn RuntimeEngineAdmission>,
    ) -> Result<(), AppError> {
        descriptor.validate()?;
        let mut extensions = self
            .extensions
            .lock()
            .map_err(|_| AppError::Internal("Runtime registration lock poisoned".into()))?;
        if self.catalog.get().is_some() {
            return Err(AppError::Conflict(
                "Runtime registration is closed after host assembly".into(),
            ));
        }
        extensions.register(descriptor, factory, admission)
    }

    /// Declare a packaged alias for an already registered extension build.
    /// Repointing requires changing the composition source and repackaging;
    /// existing Session/Fork bindings never resolve the alias again.
    pub fn register_channel(
        &self,
        family_id: &str,
        channel: &str,
        build_id: &str,
    ) -> Result<(), AppError> {
        let mut extensions = self
            .extensions
            .lock()
            .map_err(|_| AppError::Internal("Runtime registration lock poisoned".into()))?;
        if self.catalog.get().is_some() {
            return Err(AppError::Conflict(
                "Runtime channel registration is closed after host assembly".into(),
            ));
        }
        // These aliases belong to the two official defaults. Source-integrated
        // alternate builds may declare other channels, not replace defaults.
        if channel == "stable" && matches!(family_id, "nomifun.nomi" | "nomifun.coding") {
            return Err(AppError::Conflict(
                "Official stable engine channels are reserved".into(),
            ));
        }
        extensions.register_channel(family_id, channel, build_id)
    }

    pub(crate) fn dispatch(self: &Arc<Self>, nomi: AgentRuntimeFactory) -> AgentRuntimeFactory {
        self.nomi_factory
            .set(nomi.clone())
            .unwrap_or_else(|_| panic!("runtime factory installed twice"));
        let host = self.clone();
        Arc::new(move |options| {
            let host = host.clone();
            let legacy = nomi.clone();
            Box::pin(async move {
                match binding_from_extra(&options.extra)? {
                    Some(binding) => host.catalog()?.open(&binding, options).await,
                    // Pre-existing conversations have no engine binding. This
                    // is the explicit legacy Nomi path, never a fallback for
                    // an unavailable or malformed bound implementation.
                    None => legacy(options).await,
                }
            })
        })
    }

    pub(crate) fn install(&self, coding: RuntimeEngineFactory) -> Result<(), AppError> {
        let extensions = self
            .extensions
            .lock()
            .map_err(|_| AppError::Internal("Runtime registration lock poisoned".into()))?;
        if self.catalog.get().is_some() {
            return Err(AppError::Conflict(
                "Runtime catalog already installed".into(),
            ));
        }
        // Stage the full catalog before publication. A failed official build or
        // alias registration leaves both the staged extensions and OnceLock
        // unchanged; no partially installed catalog is observable.
        let mut catalog = extensions.clone();
        let nomi = self
            .nomi_factory
            .get()
            .ok_or_else(|| AppError::Internal("Nomi factory not installed".into()))?
            .clone();
        let descriptor = nomi_descriptor();
        let build_id = descriptor.build_id.clone();
        catalog.register(
            descriptor,
            Arc::new(move |options, _| {
                let nomi = nomi.clone();
                Box::pin(async move {
                    #[allow(unreachable_patterns)]
                    match nomi(options).await? {
                        AgentRuntimeHandle::Registered(runtime) => Ok(runtime),
                        AgentRuntimeHandle::Nomi(runtime) => {
                            Ok(runtime as Arc<dyn nomifun_ai_agent::RegisteredAgentRuntime>)
                        }
                        _ => Err(AppError::Internal(
                            "Built-in Nomi factory returned a test-only runtime".into(),
                        )),
                    }
                })
            }),
            Arc::new(NomiAdmission),
        )?;
        catalog.register_channel("nomifun.nomi", "stable", &build_id)?;
        let descriptor = super::coding_runtime_host::descriptor();
        let build_id = descriptor.build_id.clone();
        catalog.register(descriptor, coding, Arc::new(super::coding_runtime_host::support()))?;
        catalog.register_channel("nomifun.coding", "stable", &build_id)?;
        self.catalog
            .set(Arc::new(catalog))
            .map_err(|_| AppError::Conflict("Runtime catalog already installed".into()))
    }

    pub fn catalog(&self) -> Result<&Arc<RuntimeEngineCatalog>, AppError> {
        self.catalog
            .get()
            .ok_or_else(|| AppError::Conflict("Runtime host is not assembled".into()))
    }

    pub(crate) fn default_binding(&self) -> Result<RuntimeEngineBinding, AppError> {
        self.catalog()?.resolve(
            &RuntimeEngineSelector::Channel {
                family_id: "nomifun.nomi".into(),
                channel: "stable".into(),
            },
            "default",
        )
    }

    pub(crate) fn agent_binding(
        &self,
        payload: &nomifun_agent_contracts::AgentPresetRevisionPayload,
    ) -> Result<RuntimeEngineBinding, AppError> {
        match &payload.runtime_engine {
            Some(selection) => {
                use nomifun_agent_contracts::AgentRuntimeEngineSelector;
                let selector = match &selection.selector {
                    AgentRuntimeEngineSelector::Exact { family_id, build_id, build_digest } =>
                        RuntimeEngineSelector::Exact { family_id: family_id.clone(), build_id: build_id.clone(), build_digest: build_digest.clone() },
                    AgentRuntimeEngineSelector::Channel { family_id, channel } =>
                        RuntimeEngineSelector::Channel { family_id: family_id.clone(), channel: channel.clone() },
                };
                self.catalog()?.resolve(&selector, &selection.profile)
            }
            None => self.default_binding(),
        }
    }

    pub(crate) fn validate_agent(
        &self,
        payload: &nomifun_agent_contracts::AgentPresetRevisionPayload,
        snapshot: &nomifun_agent_contracts::ResolvedSnapshotEnvelope,
    ) -> Result<RuntimeEngineBinding, AppError> {
        let binding = self.agent_binding(payload)?;
        self.catalog()?.validate_snapshot(&binding, snapshot)?;
        Ok(binding)
    }
}

// Engine support must match its real tool owner. Native Nomi MCP stays
// available as a separate lane. Canonical resources and frozen MCP tools use retained dispatch,
// durable owner receipts, mandatory recovery context and source-replay gates.
struct NomiAdmission;
pub(crate) fn validate_nomi_snapshot(snapshot: &nomifun_agent_contracts::ResolvedSnapshotEnvelope) -> Result<(), AppError> {
    use nomifun_agent_contracts::{ContributionSourceKind, PluginSourceKind};
    nomifun_ai_agent::tool_middleware::validate_selection(&snapshot.content)
        .map_err(|e| AppError::Conflict(e.to_string()))?;
    let selected = snapshot.content.enabled_capabilities.iter().collect::<Vec<_>>();
    let resources = selected.iter().any(|entry| entry.capability.id.as_ref() == "mcp.resource");
    if resources && selected.iter().any(|entry| entry.capability.id.as_ref() == "mcp.tool_proxy") {
        return Err(AppError::Conflict("MCP resources use the platform owner; select frozen MCP tools instead of the native all-tools proxy".into()));
    }
    if resources && selected.iter().any(|entry| entry.capability.id.as_ref() == "mcp.resource"
        && (entry.contribution_lock.source_kind != ContributionSourceKind::PlatformBuiltin
            || entry.resolved_source.source_kind != PluginSourceKind::Bundled)) {
        return Err(AppError::Conflict("MCP resources require their bundled platform owner".into()));
    }
    if !selected.iter().any(|capability| super::nomi_core_mcp_catalog::is_product_tool(capability.capability.id.as_ref())) {
        return Ok(());
    }
    for capability in &selected {
        let id = capability.capability.id.as_ref();
        if id == "mcp.tool_proxy" || (!resources && matches!(id, "mcp.connect" | "mcp.oauth")) {
            return Err(AppError::Conflict("Choose either frozen per-tool MCP grants or native MCP capabilities, not both".into()));
        }
        if super::nomi_core_mcp_catalog::is_product_tool(id)
            && (capability.contribution_lock.source_kind != ContributionSourceKind::McpBinding
                || capability.resolved_source.source_kind != PluginSourceKind::Bundled
                || !snapshot.content.mcp_tool_locks.iter().any(|lock| lock.capability_id == capability.capability.id
                    && lock.canonical_tool_key.as_ref() == id && lock.materialization_revision == 1))
        {
            return Err(AppError::Conflict("Nomi MCP tool requires exact bundled Snapshot mapping".into()));
        }
        if capability.contribution_lock.source_kind == ContributionSourceKind::McpBinding
            && !super::nomi_core_mcp_catalog::is_product_tool(id) {
            return Err(AppError::Conflict("Unsupported MCP mapping mixed with the product per-tool lane".into()));
        }
    }
    let servers = snapshot.content.mcp_tool_locks.iter().map(|lock| &lock.server_id).collect::<std::collections::BTreeSet<_>>();
    if servers.is_empty() || servers.len() > super::nomi_core_mcp_catalog::MAX_SESSION_SERVERS || snapshot.content.mcp_tool_locks.iter().any(|lock|
        !super::nomi_core_mcp_catalog::is_product_tool(lock.capability_id.as_ref())
        || lock.canonical_tool_key.as_ref() != lock.capability_id.as_ref() || lock.materialization_revision != 1
        || !selected.iter().any(|capability| capability.capability.id == lock.capability_id)) {
        return Err(AppError::Conflict("Nomi per-tool MCP requires bounded exact servers and selected tool locks".into()));
    }
    Ok(())
}
impl RuntimeEngineAdmission for NomiAdmission {
    fn supports_tool_hooks(&self, _binding: &RuntimeEngineBinding) -> bool { true }
    fn uses_nomi_session(&self, _binding: &RuntimeEngineBinding) -> bool { true }

    fn validate_snapshot(&self, binding: &RuntimeEngineBinding, snapshot: &nomifun_agent_contracts::ResolvedSnapshotEnvelope) -> Result<(), AppError> {
        validate_nomi_snapshot(snapshot)?;
        RuntimeEngineSupport::platform().validate_snapshot(binding, snapshot)
    }
    fn validate_session_extra(&self, binding: &RuntimeEngineBinding, extra: &serde_json::Value) -> Result<(), AppError> {
        RuntimeEngineSupport::platform().validate_session_extra(binding, extra)
    }
}

pub(crate) fn nomi_descriptor() -> RuntimeEngineDescriptor {
    RuntimeEngineDescriptor {
        family_id: "nomifun.nomi".into(),
        build_id: format!("{}-host64", env!("CARGO_PKG_VERSION")),
        build_digest: format!(
            "{:x}",
            Sha256::digest(
                concat!(
                    include_str!("../../../../../Cargo.lock"),
                    include_str!("../../Cargo.toml"),
                    include_str!("../../../nomifun-public/Cargo.toml"),
                    include_str!("../../../nomifun-agent-control-plane/src/kernel_catalog.rs"),
                    include_str!("../../../nomifun-agent-contracts/src/engine_features.rs"),
                    include_str!("../../../nomifun-agent-contracts/src/runtime.rs"),
                    include_str!("../../../nomifun-agent-contracts/contracts/engine/platform-feature-inventory.payload.json"),
                    include_str!("agent_wave1_host.rs"),
                    include_str!("agent_wave1_companion_host.rs"),
                    include_str!("agent_wave1_memory_receipts.rs"),
                    include_str!("nomi_core_builtins.rs"),
                    include_str!("remote_runtime.rs"),
                    include_str!("../../../nomifun-public/src/canonical.rs"),
                    include_str!("../../../nomifun-ai-agent/src/factory/nomi.rs"),
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
                    include_str!("../../../nomifun-ai-agent/src/model_attachments.rs"),
                    include_str!("../../../nomifun-ai-agent/src/nomi_skills.rs"),
                    include_str!("../../../nomifun-ai-agent/src/nomi_resources.rs"),
                    include_str!("../../../nomifun-engine-core/src/context_resource.rs"),
                    include_str!("engine_skills.rs"),
                    include_str!("../../../nomifun-agent-kernel/src/compiler.rs"),
                    include_str!("../../../nomifun-agent-kernel/src/session_capabilities.rs"),
                    include_str!("../../../nomifun-ai-agent/src/runtime_admission.rs"),
                    include_str!("../../../nomifun-agent-contracts/src/tool_middleware.rs"),
                    include_str!("../../../nomifun-ai-agent/src/tool_middleware.rs"),
                    include_str!("../../../nomifun-ai-agent/src/model_middleware.rs"),
                    include_str!("nomi_core_tool_discovery.rs"),
                    include_str!("../../../../agent/nomi-agent/src/tool_middleware.rs"),
                    include_str!("../../../../agent/nomi-agent/src/tool_execution.rs"),
                    include_str!("../../../../agent/nomi-agent/src/engine/mod.rs"),
                    include_str!("../../../../agent/nomi-tools/src/lib.rs"),
                    include_str!("../../../../agent/nomi-tools/src/read.rs"),
                    include_str!("../../../../agent/nomi-tools/src/write.rs"),
                    include_str!("../../../../agent/nomi-tools/src/edit.rs"),
                    include_str!("../../../../agent/nomi-tools/src/apply_patch.rs"),
                    include_str!("../../../../agent/nomi-tools/src/bash.rs"),
                    include_str!("../../../../agent/nomi-tools/src/exec_command.rs"),
                    include_str!("../../../../agent/nomi-agent/src/lazy_mcp.rs"),
                    include_str!("../../../../agent/nomi-mcp/src/manager.rs"),
                    include_str!("../../../nomifun-agent-domain-wave2/src/lib.rs"),
                    include_str!("../../../nomifun-agent-domain-wave2/src/workspace_schema.rs"),
                    include_str!("../../../nomifun-agent-domain-wave2/src/process_schema.rs"),
                    include_str!("agent_wave2_host.rs"),
                    include_str!("agent_wave2_vcs_push.rs"),
                    include_str!("engine_git_lifecycle.rs"),
                    include_str!("nomi_core_wave2.rs"),
                    include_str!("engine_process_host.rs"),
                    include_str!("engine_process_recovery.rs"),
                    include_str!("../../../nomifun-engine-core/src/process.rs"),
                    include_str!("../../../nomifun-engine-core/src/lib.rs"),
                    include_str!("workspace_file_read.rs"),
                    include_str!("../../../nomifun-file/src/agent_text_read.rs"),
                    include_str!("../../../nomifun-file/src/agent_instruction_scope.rs"),
                    include_str!("../../../nomifun-file/src/agent_patch_lines.rs"),
                    include_str!("../../../nomifun-file/src/agent_patch_source.rs"),
                    include_str!("../../../nomifun-file/src/agent_patch_outcome.rs"),
                    include_str!("../../../nomifun-file/src/service.rs"),
                    include_str!("../../../nomifun-file/src/agent_text_search.rs"),
                    include_str!("../../../nomifun-file/src/resource.rs"),
                    include_str!("../../../nomifun-file/src/path_safety.rs"),
                    include_str!("../../../nomifun-ai-agent/src/runtime_extension.rs"),
                    include_str!("../../../nomifun-ai-agent/src/runtime_catalog.rs"),
                    include_str!("../desktop.rs"),
                    include_str!("../services.rs"),
                    include_str!("../bootstrap/nomi_core.rs"),
                    include_str!("../bootstrap/composition_cleanup.rs"),
                    include_str!("routes.rs"),
                    include_str!("../../../nomifun-ai-agent/src/runtime_registry.rs"),
                    include_str!("../../../nomifun-ai-agent/src/runtime_registry_shutdown.rs"),
                    include_str!("../../../nomifun-ai-agent/src/runtime_registry_acquisition.rs"),
                    include_str!("../../../nomifun-ai-agent/src/engine_tasks.rs"),
                    include_str!("../../../nomifun-ai-agent/src/engine_effect_scope.rs"),
                    include_str!("../../../nomifun-ai-agent/src/plugin_tools.rs"),
                    include_str!("../../../nomifun-ai-agent/src/manager/nomi/agent.rs"),
                    include_str!("nomi_core_session.rs"),
                    include_str!("nomi_core_agent_projection.rs"),
                    include_str!("../../../nomifun-api-types/src/agent_platform.rs"),
                    include_str!("../../../nomifun-api-types/src/execution_constraints.rs"),
                    include_str!("../../../nomifun-agent-execution/src/attempt_runner.rs"),
                    include_str!("nomi_core_resource_bindings.rs"),
                    include_str!("runtime_engines.rs"),
                    include_str!("nomi_core_mcp_catalog.rs"),
                    include_str!("nomi_core_mcp.rs"),
                    include_str!("agent_wave2_mcp.rs"),
                    include_str!("nomi_core_mcp_resources.rs"),
                    include_str!("engine_mcp_resources.rs"),
                    include_str!("engine_mcp_media.rs"),
                    include_str!("engine_session_host.rs"),
                    include_str!("engine_history.rs"),
                    include_str!("mcp_effect_receipts.rs"),
                    include_str!("hosted_effect_receipts.rs"),
                    include_str!("engine_plugin_product_tools.rs"),
                    include_str!("nomi_core_robot.rs"),
                    include_str!("../../../nomifun-robot/src/tool_registry.rs"),
                    include_str!("../../../nomifun-robot/src/vision.rs"),
                    include_str!("../../../nomifun-ai-agent/src/plugin_tool_error_projection.rs"),
                    include_str!("../../../nomifun-plugin-platform/src/runtime/m1_application.rs"),
                    include_str!("../../../nomifun-db/migrations/102_conversation_hosted_effects.sql"),
                    include_str!("../../../nomifun-db/migrations/103_conversation_git_effects.sql"),
                    include_str!("boot_terminal_proof.rs"),
                    include_str!("../../../nomifun-db/migrations/100_conversation_mcp_effects.sql"),
                    include_str!("../../../nomifun-db/migrations/101_mcp_effect_observations.sql"),
                    include_str!("../../../nomifun-conversation/src/service.rs"),
                    include_str!("../../../nomifun-db/src/conversation_context.rs"),
                    include_str!("../../../nomifun-db/src/repository/conversation.rs"),
                    include_str!("../../../nomifun-db/src/repository/sqlite_conversation.rs"),
                    include_str!("../../../nomifun-ai-agent/src/runtime_handle.rs"),
                    include_str!("../../../nomifun-ai-agent/src/nomi_session_persistence.rs"),
                    include_str!("plugin_platform.rs"),
                    include_str!("state.rs"),
                    include_str!("../../../nomifun-mcp/src/service.rs"),
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
                    include_str!("../../../nomifun-mcp/src/routes.rs"),
                    include_str!("../../../nomifun-db/src/repository/sqlite_mcp_server.rs")
                )
                .as_bytes()
            )
        ),
        display_name: "Nomi".into(),
        host_contract_version: nomifun_api_types::RUNTIME_HOST_CONTRACT_VERSION,
        supported_profiles: vec!["default".into()],
    }
}

pub(crate) fn binding_from_extra(
    extra: &serde_json::Value,
) -> Result<Option<RuntimeEngineBinding>, AppError> {
    extra
        .get(RUNTIME_ENGINE_BINDING_KEY)
        .map(|value| {
            let binding: RuntimeEngineBinding =
                serde_json::from_value(value.clone()).map_err(|error| {
                    AppError::Conflict(format!("Invalid persisted runtime binding: {error}"))
                })?;
            binding.validate()?;
            Ok(binding)
        })
        .transpose()
}
