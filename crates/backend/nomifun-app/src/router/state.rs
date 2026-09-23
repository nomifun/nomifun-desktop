//! Module-level router states + their builders.
//!
//! `ModuleStates` is the bundle returned by `build_module_states`; each
//! `build_*_state` constructs one `*RouterState` from `AppServices`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use axum::http::StatusCode;
use nomifun_ai_agent::{
    AgentRouterState, AgentRuntimeSessions, AgentService,
    NomiPlatformBuiltinContextAdmission,
    NomiPlatformBuiltinLifecycleAdmission,
    NomiPlatformBuiltinToolAdmission,
};
use nomifun_agent_contracts::{
    CapabilityConsumer, CanonicalErrorCode, platform_feature_inventory_payload,
    PluginSourceKind, RuntimeProfileKind, RuntimeTarget, VersionString,
    digest_payload,
    agent_store_schema_manifest_payload, official_preset_seed_manifest_payload,
};
use nomifun_agent_control_plane::{
    AgentControlPlane, OfficialTemplateCatalog, PresetRevisionCompiler,
};
use nomifun_agent_kernel::{
    CompilerEnvironment, InMemoryPluginStatePersistence, KernelRegistry,
    MaterializationPolicy,
};
use nomifun_agent_control_plane::{
    KernelCatalogProvider,
};
use nomifun_api_types::{AgentResolvedSnapshot, TerminalExitEvent};
use nomifun_auth::extract_token_from_ws_headers;
use nomifun_channel::ChannelRouterState;
use nomifun_common::{AppError, OnTerminalDelete};
use nomifun_cron::{CronEventEmitter, CronRouterState};
use nomifun_db::{
    IAgentExecutionRepository, IAgentExecutionTemplateRepository,
    IProviderRepository, IRemoteBindingRepository, SqliteAgentExecutionRepository,
    SqliteAgentExecutionTemplateRepository,
    SqliteClientPreferenceRepository,
    SqliteProviderRepository, SqliteRemoteBindingRepository,
    SqliteSettingsRepository,
};
use nomifun_file::{FileRouterState, FileService, FileWatchService, SnapshotService};
use nomifun_knowledge::KnowledgeRouterState;
use nomifun_mcp::{
    ClaudeAdapter, CodeBuddyAdapter, CodexAdapter, GeminiAdapter, McpAgentAdapter, McpConfigService,
    McpConnectionTestService, McpRouterState, McpSyncService, NomiAdapter, NomifunAdapter, OpencodeAdapter,
    QwenAdapter,
};
use nomifun_office::{
    OfficeRouterState, OfficecliWatchManager, ProxyService,
    SnapshotService as OfficeSnapshotService,
};
use nomifun_skill_library::{ExternalPathsManager, SkillRouterState};
use nomifun_agent_execution::{
    AgentExecutionEngine, AgentExecutionEngineConfig,
};
use nomifun_companion::CompanionRouterState;
use nomifun_workshop::WorkshopRouterState;
use nomifun_creation::CreationRouterState;
use nomifun_realtime::WsHandlerState;
use nomifun_requirement::RequirementRouterState;
use nomifun_shell::ShellRouterState;
use nomifun_system::{
    ClientPrefService, ConnectionTestRouterState, ConnectionTestService, ModelFetchService,
    ProviderService, SettingsService, SystemRouterState, VersionCheckService,
};
use nomifun_terminal::TerminalRouterState;
use nomifun_webhook::WebhookRouterState;

use crate::services::{AppServices, BackgroundTaskRegistry};
use super::nomi_core_control_plane::NomiCoreControlPlaneStore;
use super::nomi_core_chat_route::NomiCoreDefaultChatRouteResolver;
use super::nomi_core_session::{
    NomiCoreAgentApiState, NomiCorePluginToolSessionProvider,
    NomiCoreSessionOwner,
};
use super::idmm::IdmmRouterState;
/// All module-level router states bundled into a single struct.
///
/// Reduces parameter bloat on router constructors and makes it easy for
/// tests to override individual modules.
pub struct ModuleStates {
    pub system: SystemRouterState,
    pub ssh_host: nomifun_ssh::SshHostRouterState,
    pub agent: AgentRouterState,

    pub connection_test: ConnectionTestRouterState,
    pub file: FileRouterState,
    pub mcp: McpRouterState,
    pub skill: SkillRouterState,
    pub channel: ChannelRouterState,
    pub cron: CronRouterState,
    pub requirement: RequirementRouterState,
    /// Per-AgentSession intelligent decision supervisor.
    pub(crate) idmm: IdmmRouterState,
    pub knowledge: KnowledgeRouterState,
    pub companion: CompanionRouterState,
    /// 客服独立域 (customer-service domain).
    pub customer_service: nomifun_customer_service::CustomerServiceRouterState,
    /// Creative Studio project, asset, template, and archive domain.
    pub workshop: WorkshopRouterState,
    /// Unified Plugin Library, Draft, Import, Surface and storage state.
    pub plugin: super::plugin::PluginRouterState,
    /// 生成引擎 (creation) media task queue.
    pub creation: CreationRouterState,
    pub webhook: WebhookRouterState,
    /// Persistent Agent collaboration and execution state.
    pub agent_execution: Arc<AgentExecutionEngine>,
    pub terminal: TerminalRouterState,
    pub office: OfficeRouterState,
    pub shell: ShellRouterState,
    /// Canonical Agent Settings/AgentSession/Remote adapter for the current
    /// Nomi-core product.  It shares the Session owner above and persists its
    /// control-plane facts in the Nomi-core database tables.
    pub(crate) nomi_core_agent_api: NomiCoreAgentApiState,
}

fn default_allowed_roots(work_dir: Option<&std::path::Path>) -> Vec<std::path::PathBuf> {
    let mut roots = vec![
        std::env::temp_dir(),
        dirs::home_dir().unwrap_or_else(std::env::temp_dir),
    ];
    // Auto-provisioned per-conversation workspaces live under
    // `{work_dir}/conversations/{uuidv7}/`. On Windows the
    // operator may put `work_dir` on a separate drive (e.g. `X:\Nomi`)
    // that's neither under `temp_dir` nor `home_dir`, which previously
    // caused `/api/fs/list` to 403 every Hermes-mode session
    // (ELECTRON-1BT). Including `work_dir` keeps temp + custom-on-drive
    // workspaces on the allowlist without widening the sandbox to
    // unrelated paths.
    if let Some(wd) = work_dir
        && !wd.as_os_str().is_empty()
        && !roots.iter().any(|r| r == wd)
    {
        roots.push(wd.to_path_buf());
    }
    roots
}

/// Components needed to start the channel message loop.
///
/// Returned alongside `ChannelRouterState` by `build_channel_state`.
/// The caller must spawn the message loop as a background task.
pub struct ChannelMessageLoopComponents {
    pub message_loop: nomifun_channel::message_loop::ChannelMessageLoop,
    pub message_rx: tokio::sync::mpsc::Receiver<nomifun_channel::types::ChannelIncoming>,
    pub manager: Arc<nomifun_channel::manager::ChannelManager>,
    pub pairing_service: Arc<nomifun_channel::pairing::PairingService>,
    pub repository: Arc<dyn nomifun_db::IChannelRepository>,
    pub plugin_factory: Arc<nomifun_channel::manager::PluginFactory>,
    /// Busy-time prompt queue drain (spec D1). The caller spawns
    /// `queue_drain.run(event_bus.subscribe_user())` next to the message loop.
    pub queue_drain: nomifun_channel::queue_drain::QueueDrain,
    /// Shared channel message service (pending-decision store + asset
    /// resolver for the delivery-notify observer's IM relay).
    pub message_service: Arc<nomifun_channel::message_service::ChannelMessageService>,
}

/// Build all default `ModuleStates` from application services.
/// Compatibility entry point; production composition must use the fallible
/// builder so its resource owner can perform startup-failure cleanup.
pub async fn build_module_states(services: &AppServices) -> (ModuleStates, ChannelMessageLoopComponents) {
    try_build_module_states(services).await.unwrap_or_else(|error| {
        panic!("application module-state assembly failed: {error:#}")
    })
}

/// Does not publish a router or consume AppServices on failure. The caller
/// must retain and clean the partially assembled service graph before exit.
pub(crate) async fn try_build_module_states(
    services: &AppServices,
) -> anyhow::Result<(ModuleStates, ChannelMessageLoopComponents)> {
    let boot = Instant::now();
    tracing::info!("startup: module state build started");

    let skill_state = build_skill_state(services).await;
    tracing::info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: skill state built"
    );

    let canonical_session_owner = nomifun_conversation::CanonicalAgentSessionOwner::from_pool(
        services.database.pool().clone(),
    )
    .await
    .map_err(|error| anyhow::anyhow!("canonical AgentSession owner assembly failed: {error:#}"))?;
    let conversation_owner = Arc::new(NomiCoreSessionOwner::new(
        canonical_session_owner,
        services.agent_runtime_sessions.clone(),
        services.event_bus.clone(),
        services.background_tasks.clone()
            as Arc<dyn nomifun_conversation::BackgroundTaskRegistrar>,
        services.work_dir.join("agent-sessions"),
        services.database.pool().clone(),
        services.creation_service.clone(),
    ));
    let idmm_service = super::idmm::build_idmm_service(
        services.authoritative_user_id.clone(),
        services.database.pool().clone(),
        conversation_owner.clone(),
        services.model_invoke_service.clone(),
        services.work_dir.join("idmm"),
        services.provider_lifecycle.clone(),
    );
    conversation_owner.install_idmm(Arc::downgrade(&idmm_service))?;
    let idmm_state = IdmmRouterState::new(idmm_service.clone(), conversation_owner.clone());
    let javascript_runtime_foundation =
        super::javascript_runtime::build_javascript_runtime_foundation(
            services.database.pool().clone(),
            services.data_dir.clone(),
        )
        .await
        .map_err(|error| anyhow::anyhow!("JavaScript Runtime foundation composition failed: {error:#}"))?;
    let plugin_action_dispatcher = Arc::new(
        super::plugin_ports::RegistryPluginActionDispatcher::default(),
    );
    let plugin_ports = super::plugin_ports::build_plugin_service_ports(
        services.database.pool().clone(),
        services.encryption_key,
        plugin_action_dispatcher.clone(),
        Arc::new(super::plugin_ports::UnavailablePluginDesktopOwner),
    );
    let plugin_surface_host = plugin_ports.host.clone();
    let plugin_service_runtime = Arc::new(nomifun_plugin_platform::PluginServiceRuntime::new(
        javascript_runtime_foundation.authority(),
        services.plugin_artifacts.clone(),
        plugin_ports,
    ));
    let plugin_binding_registry = nomifun_plugin_platform::InMemoryPluginBindingRegistry::new(
        Arc::new(super::plugin_ports::PluginServiceActionRuntime::new(
            plugin_service_runtime.clone(),
        )),
    );
    super::plugin_ports::register_plugin_binding_owners(&plugin_binding_registry)
        .map_err(|error| anyhow::anyhow!("Plugin Binding owner registration failed: {error}"))?;
    plugin_action_dispatcher
        .install(plugin_binding_registry.clone())
        .map_err(anyhow::Error::msg)?;
    let plugin_binding_consumers =
        super::plugin_ports::PluginBindingConsumers::new(
            plugin_binding_registry.clone(),
            plugin_surface_host,
        );
    services
        .plugin_service_runtime
        .set(plugin_service_runtime.clone())
        .map_err(|_| anyhow::anyhow!("Plugin Service runtime was installed more than once"))?;
    let plugin_install = Arc::new(nomifun_plugin_platform::PluginInstallService::new(
        services.plugin_repository.clone(),
        services.plugin_artifacts.clone(),
        services.plugin_data_roots.clone(),
        plugin_service_runtime.clone(),
        Arc::new(plugin_binding_registry),
    ));
    plugin_install
        .recover()
        .await
        .map_err(|error| anyhow::anyhow!("Plugin mutation recovery failed: {error}"))?;
    let plugin_state = super::plugin::PluginRouterState::new(
        services,
        plugin_install,
        plugin_service_runtime,
        plugin_binding_consumers,
    );
    plugin_state
        .recover()
        .await
        .map_err(|error| anyhow::anyhow!("Plugin runtime recovery failed: {error}"))?;
    let (nomi_core_agent_api, nomi_core_wave4_owners, mcp_catalog_publisher) =
        build_nomi_core_agent_api_state(
            services,
            conversation_owner.clone(),
            idmm_service.clone(),
            plugin_state.agent.clone(),
        )
        .await
        .map_err(|error| anyhow::anyhow!("Nomi-core Agent/Plugin platform composition failed: {error:#}"))?;
    #[cfg(feature = "browser-use")]
    if let Some(attached_chrome) = services.attached_chrome.as_ref() {
        crate::browser_workspace_provider::install_attached_session_verifier(
            attached_chrome,
            nomi_core_agent_api.session_owner.canonical().clone(),
        )?;
    }
    let cron = build_cron_state(services, conversation_owner.clone());
    nomi_core_agent_api
        .install_cron_cleanup_owner(cron.cron_service.clone())
        .map_err(|error| anyhow::anyhow!("Nomi-core Cron cleanup owner installation failed: {error}"))?;
    cron.cron_service.with_agent_preset_resolver(Arc::new(
        NomiCoreCronAgentPresetResolver {
            control_plane: nomi_core_agent_api.control_plane.clone(),
        },
    ));

    // Generation-5 Store recovery is owned by NomiCoreSessionOwner. The
    // retired Conversation orphan sweep is intentionally not part of startup.

    // The agent catalog already hydrated at startup (see `lib.rs`).
    // Extension-contributed rows will land in `agent_metadata` in a
    // later step; for now we rely on the builtin + internal seed rows.

    let (channel_state, channel_components) =
        build_channel_state(services, conversation_owner.clone()).await;
    nomi_core_wave4_owners
        .install_channel(
            Arc::clone(&channel_components.manager),
            Arc::clone(&channel_components.pairing_service),
            Arc::clone(&channel_components.repository),
            Arc::clone(&services.customer_service_service),
        )
        .map_err(|error| anyhow::anyhow!("Nomi-core Wave 4 Channel owner installation failed: {error}"))?;
    nomi_core_wave4_owners
        .install_channel_ingress(Arc::clone(&channel_components.message_service))
        .map_err(|error| anyhow::anyhow!("Nomi-core Wave 4 Channel ingress installation failed: {error}"))?;
    tracing::info!(elapsed_ms = boot.elapsed().as_millis(), "startup: channel state built");

    let agent_service = AgentService::new(
        services.agent_registry.clone(),
        services.data_dir.clone(),
        services.model_invoke_service.clone(),
    );
    tracing::info!(elapsed_ms = boot.elapsed().as_millis(), "startup: agent service built");

    tracing::info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: module states bundle started"
    );
    // AgentExecution owns Attempt, retry, recovery and canonical turn receipt
    // before AutoWork is allowed to resume any persisted queue binding.
    let agent_execution = build_agent_execution_engine(
        services,
        conversation_owner.clone(),
    );
    nomi_core_agent_api
        .wave5_owner
        .install_runtime_owners(
            agent_execution.clone(),
            cron.cron_service.clone(),
        )
        .map_err(|error| anyhow::anyhow!("Wave 5 owner installation failed: {error}"))?;
    let requirement_state = build_requirement_state(
        services,
        conversation_owner.clone(),
        agent_execution.clone(),
    );
    nomi_core_agent_api
        .install_requirement_cleanup_owner(
            requirement_state.requirement_service.clone(),
            requirement_state.auto_work_runner.clone(),
        )
        .map_err(|error| {
            anyhow::anyhow!("Nomi-core Requirement cleanup owner installation failed: {error}")
        })?;
    nomi_core_agent_api
        .recover_deleting_agent_sessions()
        .await
        .map_err(|error| anyhow::anyhow!(
            "canonical AgentSession delete recovery failed: {error}"
        ))?;
    cron.cron_service.init().await;
    tracing::info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: canonical delete recovery and cron initialization completed"
    );
    agent_execution.spawn_recovery();
    requirement_state.auto_work_runner.start_sweeper();
    requirement_state.auto_work_runner.resume_persisted_bindings();
    let idmm_shutdown = services.background_shutdown.child_token();
    services.register_background_task(tokio::spawn(idmm_service.run(idmm_shutdown)));
    let companion_state = build_companion_state(
        services,
        channel_components.manager.clone(),
        conversation_owner.clone(),
    )
        .with_knowledge_service(services.knowledge_service.clone());
    let states = ModuleStates {
        system: build_system_state(services),
        ssh_host: build_ssh_host_state(services),
        agent: AgentRouterState {
            agent_registry: services.agent_registry.clone(),
            service: agent_service,
        },
        connection_test: build_connection_test_state(),
        file: build_file_state(services),
        mcp: {
            let mut state = build_mcp_state(services);
            state.config_service = state.config_service.with_catalog_publisher(mcp_catalog_publisher);
            state
        },
        skill: skill_state,
        channel: channel_state,
        cron,
        requirement: requirement_state,
        idmm: idmm_state,
        knowledge: KnowledgeRouterState::new(services.knowledge_service.clone()),
        companion: companion_state,
        customer_service: nomifun_customer_service::CustomerServiceRouterState {
            service: services.customer_service_service.clone(),
            channel_repo: Arc::new(nomifun_db::SqliteChannelRepository::new(
                services.database.pool().clone(),
            )),
        },
        workshop: build_workshop_state(services),
        plugin: plugin_state,
        creation: build_creation_state(services),
        webhook: build_webhook_state(services),
        // REST routes, model tools and AutoWork share this one engine and its
        // canonical Attempt/receipt/recovery state machine.
        agent_execution,
        terminal: build_terminal_state(services),
        office: build_office_state(services),
        shell: build_shell_state(services),
        nomi_core_agent_api,
    };

    tracing::info!(
        elapsed_ms = boot.elapsed().as_millis(),
        "startup: module state build completed"
    );

    Ok((states, channel_components))
}

/// Build the persistent Agent Settings control plane used by the current
/// Nomi-core HTTP surface.
///
/// This registry is intentionally separate from the Fresh-v4 `AgentPlatform`
/// runtime. It materializes the canonical inventory with real Nomi-owned wave
/// registrations for Preview/Save and binds their exact Tool/Context/lifecycle
/// projections into `NomiCoreSessionOwner` and the existing Conversation/Nomi
/// engine.
async fn build_nomi_core_agent_api_state(
    services: &AppServices,
    conversation_owner: Arc<NomiCoreSessionOwner>,
    idmm: Arc<nomifun_idmm::IdmmService>,
    agent_plugins: nomifun_plugin_platform::AgentPluginBindings,
) -> anyhow::Result<(
    NomiCoreAgentApiState,
    Arc<super::nomi_core_wave4::NomiCoreWave4Owners>,
    Arc<dyn nomifun_mcp::service::McpCatalogPublisher>,
)> {
    const CONTRACT_VERSION: &str = "1.0.0";
    let builtin_plan = super::nomi_core_builtins::build(
        services,
        conversation_owner.canonical().store().clone(),
    )
    .await?;
    let wave4_owners = Arc::clone(&builtin_plan.wave4_owners);
    let wave5_owner = Arc::clone(&builtin_plan.wave5_owner);
    let robot_owner = builtin_plan.robot_owner.clone();
    #[cfg(feature = "browser-use")]
    let browser_owner = Arc::clone(&builtin_plan.browser_owner);
    wave4_owners
        .reclaim_orphaned_receipts()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let registrations = builtin_plan.registrations;
    let feature_inventory = platform_feature_inventory_payload()
        .map_err(|error| anyhow::anyhow!(error.message))?;
    let feature_digest = digest_payload(&feature_inventory)?;

    let mut policy = MaterializationPolicy::stable(CONTRACT_VERSION);
    policy.allowed_sources.insert(PluginSourceKind::ManagedLocal);
    policy.available_runtime_features =
        feature_inventory.runtime_features.clone();
    // Kernel PluginState is an in-process capability-state cache. Durable
    // owner data belongs to the Plugin repository/KV and Plugin data roots;
    // this NomiCore composition does not own the Fresh-v4 `plugin_states`
    // table and must not open it from the legacy application database.
    let state_persistence = Arc::new(InMemoryPluginStatePersistence::new());
    let kernel = Arc::new(KernelRegistry::new(
        policy,
        state_persistence,
    )?);
    let materialized = kernel
        .replace_all(registrations.clone())?;
    let approved_platform_builtin_capability_ids = builtin_plan
        .tool_capability_ids
        .union(&builtin_plan.context_capability_ids)
        .cloned()
        .chain(builtin_plan.lifecycle_capability_ids.iter().cloned())
        .chain(
            builtin_plan
                .host_dynamic_tool_capability_ids
                .iter()
                .cloned(),
        )
        .collect::<BTreeSet<_>>();
    let native_capability_ids = materialized
        .capabilities
        .values()
        .filter(|capability| {
            capability
                .manifest
                .supports_consumer(CapabilityConsumer::Agent)
                && super::agent_binding_projection::native_capability_available(&capability.manifest)
                && !approved_platform_builtin_capability_ids
                    .contains(&capability.manifest.id)
        })
        .map(|capability| capability.manifest.id.clone())
        .collect::<BTreeSet<_>>();
    let platform_builtin_tool_admission = Arc::new(
        NomiPlatformBuiltinToolAdmission::from_registry(
            &materialized,
            builtin_plan.tool_capability_ids.clone(),
            native_capability_ids.clone(),
            Arc::clone(&builtin_plan.schema_resolver),
        )?,
    );
    let platform_builtin_context_admission = Arc::new(
        NomiPlatformBuiltinContextAdmission::from_registry(
            &materialized,
            builtin_plan.context_capability_ids.clone(),
            native_capability_ids.clone(),
        )?,
    );
    let platform_builtin_lifecycle_admission = Arc::new(
        NomiPlatformBuiltinLifecycleAdmission::from_registry(
            &materialized,
            builtin_plan.lifecycle_capability_ids.clone(),
            native_capability_ids,
            Arc::clone(&builtin_plan.lifecycle_invoker),
        )?,
    );
    let unavailable_capabilities = materialized
        .capabilities
        .values()
        .filter(|capability| {
            capability
                .manifest
                .supports_consumer(CapabilityConsumer::Agent)
        })
        .filter_map(|capability| {
            if approved_platform_builtin_capability_ids
                .contains(&capability.manifest.id)
            {
                return None;
            }
            (!super::agent_binding_projection::native_capability_available(&capability.manifest)).then(|| {
                (
                    capability.manifest.id.clone(),
                    CanonicalErrorCode::from("CAPABILITY_UNAVAILABLE"),
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    let catalog = Arc::new(
        KernelCatalogProvider::new(Arc::clone(&kernel))
            .with_unavailable_capabilities(unavailable_capabilities),
    );
    let module_publisher = Arc::new(NomiCoreModuleCatalogPublisher::new(
        Arc::clone(&kernel),
        registrations,
        Arc::new(nomifun_db::SqliteMcpServerRepository::new(
            services.database.pool().clone(),
        )),
        Arc::clone(&builtin_plan.wave2_owner),
    ));
    #[cfg(feature="browser-use")]
    if let Some(runtime) = services.headless_render.as_ref() {
        services.knowledge_service.set_browser_render_content_port(
            super::knowledge_browser::KnowledgeHeadlessRenderPort::bind(runtime.clone()));
    }
    let mcp_catalog_publisher: Arc<dyn nomifun_mcp::service::McpCatalogPublisher> =
        module_publisher;
    let schema_resolver: Arc<dyn nomifun_ai_agent::NomiPluginToolSchemaResolver> =
        Arc::new(BuiltinToolSchemaAdapter {
            inner: Arc::clone(&builtin_plan.schema_resolver),
        });

    let schema_digest = digest_payload(&agent_store_schema_manifest_payload())?;
    let seed = official_preset_seed_manifest_payload();
    let installation_role_bindings =
        super::nomi_core_tool_discovery::installation_binding(&materialized)?;
    #[cfg(feature = "browser-use")]
    let installation_role_bindings = {
        let mut bindings = installation_role_bindings;
        // Provider ownership and live resource readiness are separate facts.
        // A dev host without packaged CEF can still own the Attached Chrome
        // provider contract; an unavailable concrete Browser resource then
        // narrows the tool surface instead of invalidating the whole Agent.
        let browser_provider_owned = services.browser_resources.is_some()
            || services.attached_chrome.is_some();
        bindings.extend(crate::browser_workspace_provider::installation_binding(
            &materialized,
            browser_provider_owned,
        )?);
        bindings
    };
    #[cfg(feature = "computer-use")]
    let installation_role_bindings = {
        let mut bindings = installation_role_bindings;
        bindings.extend(super::agent_role_host::installation_binding(&materialized)?);
        bindings
    };
    let environment = CompilerEnvironment {
        resolver_version: VersionString::from(CONTRACT_VERSION),
        required_runtime_protocol_version: VersionString::from(CONTRACT_VERSION),
        required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
        runtime_feature_inventory_digest: feature_digest.clone(),
        available_runtime_features: feature_inventory.runtime_features.clone(),
        installation_role_bindings,
        canonical_schema_manifest_digest: schema_digest.clone(),
        target_contribution_manifest_digest: seed.target_first_party_contribution_digest.clone(),
        host_target: nomi_core_runtime_target(),
        host_surface: if cfg!(any(feature = "browser-use", feature = "computer-use")) {
            "desktop".to_owned()
        } else {
            "headless".to_owned()
        },
        availability_evidence_revision: "nomi-core-local-2026-09-04".to_owned(),
    };
    let templates = OfficialTemplateCatalog::load()?;
    let compiler = PresetRevisionCompiler::new()
        .with_canonical_registry(Arc::clone(&kernel), environment.clone())
        .with_consumer_validator(super::nomi_core_tool_discovery::validate_snapshot);
    let store = Arc::new(NomiCoreControlPlaneStore::new(
        services.database.pool().clone(),
    ));
    let control_plane = Arc::new(AgentControlPlane::new(
        store.clone(),
        catalog,
        templates,
        compiler,
    )
    .with_installation_role_binding_store(Arc::new(
        super::nomi_core_role_defaults::NomiCoreRoleBindingStore::new(services.database.pool().clone())
            .with_host_bindings(environment.installation_role_bindings.clone()),
    ))
    .with_default_chat_route_resolver(Arc::new(
        NomiCoreDefaultChatRouteResolver::new(services.database.pool().clone()),
    )));
    let resource_bindings = super::nomi_core_resource_bindings::NomiCoreResourceBindingResolverRegistry::product(services)
        .map_err(|error| {
            anyhow::anyhow!("{}: {}", error.code(), error.message())
        })?;
    let product_agent_resolver = Arc::new(
        super::nomi_core_session::NomiCoreProductAgentResolver::new(
            Arc::clone(&control_plane),
            Arc::clone(&services.authoritative_user_id),
            services.database.pool().clone(),
            Arc::clone(&services.official_runtime),
            resource_bindings.clone(),
        ),
    );
    conversation_owner
        .install_product_agent_resolver(Arc::downgrade(&product_agent_resolver))?;
    services
        .cs_dialogue_engine
        .with_agent_policy_resolver(product_agent_resolver.clone());
    let mcp_server_repository: Arc<dyn nomifun_db::IMcpServerRepository> =
        Arc::new(nomifun_db::SqliteMcpServerRepository::new(
            services.database.pool().clone(),
        ));
    let engine_sessions = Arc::new(super::engine_session_host::EngineSessionHost::new(
        &conversation_owner, Arc::clone(&control_plane), &services.official_runtime, services.database.pool().clone(), services.encryption_key,
        super::engine_kernel_session::EngineKernelAssembly {
            kernel: Arc::clone(&kernel), environment: environment.clone(), wave2: Arc::clone(&builtin_plan.wave2_owner),
            context_admission: Arc::clone(&platform_builtin_context_admission),
            #[cfg(feature = "browser-use")]
            browser: browser_owner,
            hosted_effects: super::hosted_effect_receipts::HostedEffectReceipts::new(
                services.database.pool().clone(),
            ),
            robot: robot_owner.clone(),
            agent_plugins,
        },
    ));
    services
        .official_runtime
        .install(super::unified_runtime_host::factory(
            engine_sessions,
            Arc::clone(&schema_resolver),
            Arc::clone(&builtin_plan.schema_resolver),
            builtin_plan.host_dynamic_tool_capability_ids.clone(),
            idmm,
        ))?;
    conversation_owner.install_official_runtime(Arc::clone(&services.official_runtime), Arc::downgrade(&control_plane))?;
    let reconciled_turns = conversation_owner.reconcile_orphaned_active_turns().await?;
    if reconciled_turns > 0 {
        tracing::warn!(
            reconciled_turns,
            "reconciled orphaned AgentSession turns before route publication"
        );
    }
    let plugin_tool_sessions = Arc::new(NomiCorePluginToolSessionProvider::new(
        Arc::clone(&conversation_owner),
        Arc::clone(&control_plane),
        Arc::clone(&kernel),
        environment,
        schema_resolver,
        platform_builtin_tool_admission,
        platform_builtin_context_admission,
        platform_builtin_lifecycle_admission,
        robot_owner,
        Arc::clone(&builtin_plan.wave2_owner),
        services.database.pool().clone(),
    ));
    let remote_repository: Arc<dyn IRemoteBindingRepository> = Arc::new(
        SqliteRemoteBindingRepository::new(services.database.pool().clone()),
    );
    // Pending rendered sources must not run against the default unavailable
    // port before the canonical Kernel/Provider composition is ready.
    services.spawn_knowledge_resume_task();
    Ok((
        NomiCoreAgentApiState::new(
            services.authoritative_user_id.clone(),
            conversation_owner,
            control_plane,
            remote_repository,
            services.nomi_core_remote_runtime.clone(),
            resource_bindings,
            mcp_server_repository,
            Arc::clone(&wave4_owners),
            wave5_owner,
            product_agent_resolver,
            plugin_tool_sessions,
            services.ssh_pool.clone(),
            #[cfg(feature = "browser-use")]
            services.browser_resources.clone(),
            #[cfg(feature = "browser-use")]
            services.attached_chrome.clone(),
        ),
        wave4_owners,
        mcp_catalog_publisher,
    ))
}

struct BuiltinToolSchemaAdapter {
    inner: Arc<dyn nomifun_ai_agent::NomiPlatformBuiltinToolSchemaResolver>,
}

#[async_trait::async_trait]
impl nomifun_ai_agent::NomiPluginToolSchemaResolver for BuiltinToolSchemaAdapter {
    async fn resolve(
        &self,
        capability: &nomifun_agent_contracts::ResolvedCapability,
        reference: &nomifun_agent_contracts::CanonicalSchemaRef,
    ) -> Result<nomifun_agent_contracts::StrictJsonValue, String> {
        self.inner.resolve(capability, reference).await
    }
}

struct NomiCoreModuleCatalogPublisher {
    kernel: Arc<KernelRegistry>,
    base_registrations: Vec<nomifun_agent_kernel::PluginRegistration>,
    repository: Arc<dyn nomifun_db::IMcpServerRepository>,
    host: Arc<super::nomi_core_wave2::NomiCoreWave2Host>,
    publish_lock: tokio::sync::Mutex<()>,
}

impl NomiCoreModuleCatalogPublisher {
    fn new(
        kernel: Arc<KernelRegistry>,
        registrations: Vec<nomifun_agent_kernel::PluginRegistration>,
        repository: Arc<dyn nomifun_db::IMcpServerRepository>,
        host: Arc<super::nomi_core_wave2::NomiCoreWave2Host>,
    ) -> Self {
        let base_registrations = registrations
            .into_iter()
            .filter(|registration| {
                !registration
                    .metadata
                    .manifest
                    .payload
                    .contributions
                    .capabilities
                    .iter()
                    .any(|capability| {
                        super::nomi_core_mcp_catalog::is_product_tool(capability.id.as_ref())
                    })
            })
            .collect();
        Self {
            kernel,
            base_registrations,
            repository,
            host,
            publish_lock: tokio::sync::Mutex::new(()),
        }
    }
}

#[async_trait::async_trait]
impl nomifun_mcp::service::McpCatalogPublisher for NomiCoreModuleCatalogPublisher {
    async fn refresh(&self) -> Result<(), nomifun_mcp::McpError> {
        let _guard = self.publish_lock.lock().await;
        let mut registrations = self.base_registrations.clone();
        registrations.extend(
            super::nomi_core_mcp_catalog::load_registrations(
                self.repository.as_ref(),
                Arc::clone(&self.host),
            )
            .await
            .map_err(|_| nomifun_mcp::McpError::CatalogPublicationPending)?,
        );
        self.kernel
            .replace_all(registrations)
            .map_err(|_| nomifun_mcp::McpError::CatalogPublicationPending)?;
        Ok(())
    }
}

struct NomiCoreCronAgentPresetResolver {
    control_plane: Arc<AgentControlPlane>,
}

#[async_trait::async_trait]
impl nomifun_cron::CronAgentPresetResolver for NomiCoreCronAgentPresetResolver {
    async fn resolve_snapshot(
        &self,
        owner_id: &str,
        preset_id: &str,
    ) -> Result<AgentResolvedSnapshot, AppError> {
        let owner = nomifun_agent_contracts::UserId::from(owner_id.to_owned());
        let editor = self
            .control_plane
            .editor(&owner, preset_id, None)
            .await
            .map_err(control_plane_error_to_app)?;
        let binding = self
            .control_plane
            .resolve_agent_session_binding(&owner, preset_id)
            .await
            .map_err(control_plane_error_to_app)?;
        let (binding, revision, snapshot) = self
            .control_plane
            .saved_binding_artifacts(&owner, &binding)
            .await
            .map_err(control_plane_error_to_app)?;
        let common_owner = nomifun_common::UserId::parse(owner_id.to_owned())
            .map_err(|error| AppError::Forbidden(format!("invalid Cron owner: {error}")))?;
        super::agent_binding_projection::project_saved_artifacts(
            &common_owner,
            binding,
            revision,
            snapshot,
            Some(&editor.preset.display_name),
        )
        .map(|resolved| resolved.projection.snapshot)
    }
}

pub(super) fn control_plane_error_to_app(error: nomifun_agent_control_plane::ControlPlaneError) -> AppError {
    let mut message = format!("{}: {error}", error.code().as_ref());
    // Preserve compiler explanations across the product-domain adapter. The
    // outer preset error alone cannot identify an incompatible capability.
    if let Some(details) = error.details()
        && let Some(diagnostics) = details.get("diagnostics").and_then(serde_json::Value::as_array)
    {
        for diagnostic in diagnostics {
            if let Some(reason) = diagnostic.get("message").and_then(serde_json::Value::as_str) {
                message.push_str("; ");
                message.push_str(reason);
            }
        }
    }
    match error.status() {
        StatusCode::BAD_REQUEST => AppError::BadRequest(message),
        StatusCode::FORBIDDEN => AppError::Forbidden(message),
        StatusCode::NOT_FOUND => AppError::NotFound(message),
        StatusCode::CONFLICT => AppError::Conflict(message),
        StatusCode::UNPROCESSABLE_ENTITY => AppError::UnprocessableEntity(message),
        _ => AppError::Internal(message),
    }
}

fn nomi_core_runtime_target() -> RuntimeTarget {
    let target = if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else {
        "unsupported-local-target"
    };
    RuntimeTarget::from(target)
}

/// Build the default `SystemRouterState` from application services.
pub fn build_system_state(services: &AppServices) -> SystemRouterState {
    let encryption_key = services.encryption_key;
    let pool = services.database.pool().clone();
    let provider_repo = Arc::new(SqliteProviderRepository::new(pool.clone()));
    let provider_model_repo = services.provider_model_repo.clone();
    let capability_repo = Arc::new(
        nomifun_db::SqliteProviderModelCapabilityRepository::new(pool.clone()),
    );
    let connection_repo = Arc::new(
        nomifun_db::SqliteProviderConnectionRepository::new(pool.clone()),
    );

    // Cross-subsystem provider-deletion guard: aggregate every hard binding
    // (companion, public Agent, active Agent Execution) and strip soft
    // failover/model-pool references only after deletion is allowed.
    let execution_repo: Arc<dyn IAgentExecutionRepository> =
        Arc::new(SqliteAgentExecutionRepository::new(pool.clone()));
    let execution_template_repo: Arc<dyn IAgentExecutionTemplateRepository> =
        Arc::new(SqliteAgentExecutionTemplateRepository::new(pool.clone()));
    let deletion_coordinator = Arc::new(crate::provider_deletion::AppProviderDeletionCoordinator {
        provider_lifecycle: services.provider_lifecycle.clone(),
        companion: services.companion_service.clone(),
        customer_service: services.customer_service_service.clone(),
        workshop: services.workshop_service.clone(),
        execution_repo,
        execution_template_repo,
        pool: pool.clone(),
    });

    SystemRouterState {
        settings_service: SettingsService::new(Arc::new(SqliteSettingsRepository::new(pool.clone()))),
        client_pref_service: ClientPrefService::new(Arc::new(SqliteClientPreferenceRepository::new(pool.clone()))),
        provider_service: ProviderService::new(
            provider_repo.clone(),
            provider_model_repo.clone(),
            capability_repo.clone(),
            connection_repo.clone(),
            encryption_key,
        )
        .with_deletion_coordinator(deletion_coordinator.clone()),
        provider_connection_service: nomifun_system::ProviderConnectionService::new(
            connection_repo.clone(),
            provider_repo.clone(),
            capability_repo.clone(),
            encryption_key,
        ),
        model_fetch_service: ModelFetchService::new_dynamic(provider_repo.clone(), encryption_key),
        provider_model_service: nomifun_system::ProviderModelService::new(
            provider_model_repo,
            capability_repo,
            provider_repo,
            connection_repo,
            deletion_coordinator,
        ),
        managed_model_service: Some(services.managed_model_service.clone()),
        version_check_service: VersionCheckService::new_dynamic(env!("CARGO_PKG_VERSION").to_owned()),
        data_dir: services.data_dir.clone(),
        work_dir: services.work_dir.clone(),
        work_dir_is_cli_override: services.work_dir_is_cli_override,
    }
}

impl nomifun_cron::CronBackgroundTaskRegistrar for BackgroundTaskRegistry {
    fn register(&self, task: tokio::task::JoinHandle<()>) {
        BackgroundTaskRegistry::register(self, task);
    }
}

/// Build the SSH host-book router state on the process connection pool. The pool
/// carries its own host book, so the routes edit the exact credentials the next
/// redial will use and the test-connection probe dials through the same gate a
/// session does.
pub fn build_ssh_host_state(services: &AppServices) -> nomifun_ssh::SshHostRouterState {
    nomifun_ssh::SshHostRouterState {
        service: services.ssh_pool.host_service(),
        pool: Some(services.ssh_pool.clone()),
    }
}

/// Build the default `ConnectionTestRouterState`.
pub fn build_connection_test_state() -> ConnectionTestRouterState {
    ConnectionTestRouterState {
        service: ConnectionTestService::new(),
    }
}

/// Build the default `FileRouterState` from application services.
pub fn build_file_state(services: &AppServices) -> FileRouterState {
    let broadcaster = services.event_bus.clone();
    let mut allowed_roots = default_allowed_roots(Some(services.work_dir.as_path()));
    // Requirement attachments live under the data dir; include it so the
    // image-base64 preview works when the data dir sits outside home/temp
    // (custom NOMIFUN_DATA_DIR on another drive).
    if !allowed_roots.iter().any(|r| r == &services.data_dir) {
        allowed_roots.push(services.data_dir.clone());
    }
    let browse_roots = nomifun_file::browse::default_browse_roots();
    let file_service = Arc::new(FileService::new(broadcaster.clone(), allowed_roots.clone()));
    let watch_service = Arc::new(FileWatchService::new(broadcaster).expect("file watch service initialization"));
    let snapshot_service = Arc::new(SnapshotService::new());
    FileRouterState {
        file_service,
        watch_service,
        snapshot_service,
        allowed_roots,
        browse_roots,
    }
}

/// Build the default `McpRouterState` from application services.
pub fn build_mcp_state(services: &AppServices) -> McpRouterState {
    let pool = services.database.pool().clone();
    let repo: Arc<dyn nomifun_db::IMcpServerRepository> = Arc::new(nomifun_db::SqliteMcpServerRepository::new(pool));

    let adapters: Vec<Arc<dyn McpAgentAdapter>> = vec![
        Arc::new(ClaudeAdapter),
        Arc::new(GeminiAdapter),
        Arc::new(QwenAdapter),
        Arc::new(CodexAdapter),
        Arc::new(CodeBuddyAdapter),
        Arc::new(OpencodeAdapter),
        Arc::new(NomiAdapter),
        Arc::new(NomifunAdapter::new(repo.clone())),
    ];

    let oauth_token_repo: Arc<dyn nomifun_db::IOAuthTokenRepository> = Arc::new(
        nomifun_db::SqliteOAuthTokenRepository::new(services.database.pool().clone()),
    );

    McpRouterState {
        config_service: McpConfigService::new(repo.clone()),
        sync_service: McpSyncService::new(adapters),
        connection_test_service: McpConnectionTestService::new_dynamic(),
        oauth_service: nomifun_mcp::McpOAuthService::new_dynamic(oauth_token_repo),
    }
}

/// Adapter exposing companions to channel conversations.
///
/// The channel layer resolves a session's companion via the channel row's own
/// `companion_id` first; this profile supplies the legacy per-platform binding
/// (when present and alive) and the per-companion model lookup. There is **no
/// default-companion fallback** —an unbound channel is hosted by no companion.
/// Channel sessions with no per-platform model fall back to the bound
/// companion's configured model, so its model choice travels with it to remote
/// sessions.
struct CompanionChannelAgentProfile {
    companion_service: Arc<nomifun_companion::CompanionService>,
    channel_settings: Arc<nomifun_channel::channel_settings::ChannelSettingsService>,
}

#[async_trait::async_trait]
impl nomifun_channel::message_service::ChannelAgentProfile for CompanionChannelAgentProfile {
    async fn channel_companion_id(&self, platform: &str) -> Option<String> {
        // Per-companion binding is the ONLY way a channel becomes hosted by a companion:
        // each bot row carries its own `companion_id` (set when enabled from a companion's
        // 远程连接). The legacy per-platform binding still resolves here when present AND
        // alive, but there is **no default-companion fallback** — an unbound channel is
        // hosted by no companion (历史债「渠道与远程连接默认由默认伙伴接待」已废除；连接由
        // 用户为每个伙伴显式配置. A stale legacy binding (deleted companion) degrades to
        // None too, rather than pinning sessions to a ghost.
        if let Some(plugin) = nomifun_channel::types::PluginType::from_str_opt(platform)
            && let Ok(Some(bound)) = self.channel_settings.get_channel_companion_id(plugin).await
            && self.companion_service.get_companion(&bound).await.is_ok()
        {
            return Some(bound);
        }
        None
    }

    async fn companion_model(&self, companion_id: &str) -> Option<nomifun_common::ProviderWithModel> {
        let profile = self.companion_service.get_companion(companion_id).await.ok()?;
        profile.model
    }

    async fn companion_exists(&self, companion_id: &str) -> bool {
        self.companion_service.get_companion(companion_id).await.is_ok()
    }

    async fn companion_name(&self, companion_id: &str) -> Option<String> {
        self.companion_service
            .get_companion(companion_id)
            .await
            .ok()
            .map(|c| c.name)
            .filter(|n| !n.trim().is_empty())
    }

    async fn ensure_companion_session(&self, companion_id: &str) -> Option<String> {
        match self.companion_service.create_companion_thread(companion_id, None).await {
            Ok(thread) => match nomifun_common::ConversationId::try_from(thread.conversation_id.as_str()) {
                Ok(_) => Some(thread.conversation_id),
                Err(error) => {
                    tracing::warn!(
                        companion_id = %companion_id,
                        conversation_id = %thread.conversation_id,
                        %error,
                        "companion session returned an invalid canonical conversation ID"
                    );
                    None
                }
            },
            Err(error) => {
                tracing::warn!(companion_id = %companion_id, %error, "ensure_companion_session failed (likely no model configured)");
                None
            }
        }
    }
}

/// 客服域接缝适配器: exposes the customer-service binding lookup and the
/// stateless dialogue engine to the channel layer through the [`CsRouting`]
/// trait. `Ok("")` from `handle_visitor_message` means "merged into another
/// in-flight batch — send nothing" (the engine's `Ok(None)`).
struct AppCsRouting {
    service: Arc<nomifun_customer_service::CustomerServiceService>,
    engine: Arc<nomifun_customer_service::CsDialogueEngine>,
}

#[async_trait::async_trait]
impl nomifun_channel::message_service::CsRouting for AppCsRouting {
    async fn binding_for(&self, channel_plugin_id: &str) -> Option<String> {
        match self.service.binding_for_plugin(channel_plugin_id).await {
            Ok(binding) => binding,
            Err(error) => {
                tracing::warn!(%error, channel_plugin_id, "customer-service binding lookup failed");
                None
            }
        }
    }

    async fn handle_visitor_message(
        &self,
        cs_agent_id: &str,
        channel_plugin_id: &str,
        channel_user_id: &str,
        chat_id: &str,
        text: &str,
    ) -> Result<String, String> {
        self.engine
            .handle_visitor_message(cs_agent_id, channel_plugin_id, channel_user_id, chat_id, text)
            .await
            .map(|reply| reply.unwrap_or_default())
    }
}

/// Build the default `ChannelRouterState` and message-loop components.
pub async fn build_channel_state(
    services: &AppServices,
    conversation_owner: Arc<NomiCoreSessionOwner>,
) -> (ChannelRouterState, ChannelMessageLoopComponents) {
    let pool = services.database.pool().clone();
    let repo: Arc<dyn nomifun_db::IChannelRepository> = Arc::new(nomifun_db::SqliteChannelRepository::new(pool));
    let encryption_key = services.encryption_key;

    let (message_tx, message_rx) = tokio::sync::mpsc::channel(256);

    // Channel configuration and pairing are personal control-plane state. Bind
    // their realtime audience to the authoritative primary WebUI user before
    // constructing any producer; never reconstruct or guess it from payloads.
    let owner_user_id = services.authoritative_user_id.to_string();

    let manager = Arc::new(nomifun_channel::manager::ChannelManager::new(
        repo.clone(),
        services.event_bus.clone(),
        owner_user_id.clone(),
        encryption_key,
        message_tx,
    ));
    let group_policy_fence = manager.group_policy_fence();

    let pairing_service = Arc::new(
        nomifun_channel::pairing::PairingService::new(
            repo.clone(),
            services.event_bus.clone(),
            owner_user_id.clone(),
        )
        .with_group_policy_fence(Arc::clone(&group_policy_fence)),
    );

    // Expired pairing codes are purged only by this background sweep —the
    // timer existed but had no caller, so stale codes lingered in the DB
    // indefinitely. Deliberately detached (handle dropped): like the channel
    // message loop and plugin restore tasks, it runs for the process lifetime.
    let _pairing_cleanup = nomifun_channel::pairing::PairingService::start_cleanup_timer(repo.clone());

    let session_manager = Arc::new(nomifun_channel::session::SessionManager::new(repo.clone()));

    let plugin_factory: Arc<nomifun_channel::manager::PluginFactory> =
        Arc::new(Box::new(nomifun_channel::plugins::create_plugin));

    // Build channel settings service for per-plugin channel configuration.
    let pref_pool = services.database.pool().clone();
    let pref_repo: Arc<dyn nomifun_db::IClientPreferenceRepository> =
        Arc::new(SqliteClientPreferenceRepository::new(pref_pool));
    let channel_settings = Arc::new(nomifun_channel::channel_settings::ChannelSettingsService::new(
        pref_repo,
    ));

    // Build message-loop dependencies. Channel routing and the ActionExecutor
    // share the same settings service, so their ordinary product defaults
    // cannot drift apart.
    // 客服域接缝: one adapter instance shared by the message service (turn
    // routing) and the action executor (stranger auto-serve gate).
    let cs_routing: Arc<dyn nomifun_channel::message_service::CsRouting> =
        Arc::new(AppCsRouting {
            service: services.customer_service_service.clone(),
            engine: services.cs_dialogue_engine.clone(),
        });
    let action_executor = Arc::new(
        nomifun_channel::action::ActionExecutor::new(
            Arc::clone(&pairing_service),
            Arc::clone(&session_manager),
            Arc::clone(&channel_settings),
        )
        // Opt-in IM → requirement pipeline: the creator is always wired, but the
        // per-platform `routeToRequirement` setting (default off) gates it, so
        // behaviour is unchanged until a channel enables it.
        .with_requirement_creator(Some(
            nomifun_requirement::RequirementServiceSink::creator_arc(
                services.requirement_service.clone(),
            ),
        ))
        // 客服自动接待: a stranger on a cs-bound bot bypasses the pairing gate
        // (the one-shot session's read-only tool whitelist is the boundary).
        .with_cs_routing(Some(Arc::clone(&cs_routing))),
    );

    // Channel Agent profile: per-platform companion binding + model resolution
    // and companion-id validation for the binding write route. One instance
    // shared by the message service and the router state.
    let channel_agent_profile: Arc<dyn nomifun_channel::message_service::ChannelAgentProfile> =
        Arc::new(CompanionChannelAgentProfile {
            companion_service: services.companion_service.clone(),
            channel_settings: Arc::clone(&channel_settings),
        });

    let channel_sessions: Arc<dyn nomifun_channel::ChannelSessionPort> = conversation_owner;
    let message_service = Arc::new(
        nomifun_channel::message_service::ChannelMessageService::new(
            channel_sessions,
            Arc::clone(&channel_settings),
            repo.clone(),
            owner_user_id,
        )
        // Per-channel companion binding (with platform fallback) + model
        // resolution falls back to the bound companion when the
        // platform has no config of its own.
        .with_channel_agent_profile(Arc::clone(&channel_agent_profile))
        // 客服域接缝: cs-bound bots route their whole inbound turn to the
        // customer-service domain instead of any Conversation.
        .with_cs_routing(Arc::clone(&cs_routing))
        // Outbound media: resolve bare Workshop asset UUIDv7 values to bytes so
        // channel replies can send AI-generated images/files.
        .with_asset_resolver(Arc::new(crate::channel_asset_resolver::ChannelAssetResolver::new(
            services.workshop_service.clone(),
        ))),
    );

    let message_loop = nomifun_channel::message_loop::ChannelMessageLoop::new(
        action_executor,
        Arc::clone(&message_service),
        Arc::clone(&session_manager),
        manager.clone() as Arc<dyn nomifun_channel::stream_relay::ChannelSender>,
    );

    // Busy-time prompt queue drain (spec D1): delivers queued prompts FIFO on
    // turn completion, consuming the same realtime bus the conversation
    // service broadcasts `turn.completed` through.
    let queue_drain = nomifun_channel::queue_drain::QueueDrain::new(
        repo.clone(),
        Arc::clone(&message_service),
        Arc::clone(&session_manager),
        manager.clone() as Arc<dyn nomifun_channel::stream_relay::ChannelSender>,
    )
    .with_group_policy_fence(group_policy_fence);

    let state = ChannelRouterState {
        manager: Arc::clone(&manager),
        pairing_service: Arc::clone(&pairing_service),
        session_manager,
        repo: Arc::clone(&repo),
        plugin_factory: Arc::clone(&plugin_factory),
        settings_service: channel_settings,
        channel_agent_profile: Some(channel_agent_profile),
    };

    let components = ChannelMessageLoopComponents {
        message_loop,
        message_rx,
        manager,
        pairing_service,
        repository: repo,
        plugin_factory,
        queue_drain,
        message_service,
    };

    (state, components)
}

/// Build the default `TerminalRouterState` from application services.
pub fn build_terminal_state(services: &AppServices) -> TerminalRouterState {
    // Late-wire the knowledge service into the terminal singleton (same
    // application-owned late-binding pattern): terminal
    // create/relaunch then binds + mounts knowledge bases into the session
    // cwd. Interior mutability means every clone of the singleton (cron
    // executor, AutoWork driver) sees the wiring too.
    services
        .terminal_service
        .with_knowledge_service(services.knowledge_service.clone());
    // Key-derivation parity: register the terminal work dir as a managed root
    // so the knowledge service's live cwd resolvers (MCP search/read/write
    // dispatch) map a default-workpath terminal to the SAME `__default__`
    // binding row the terminal itself binds/mounts against. Without this the
    // two sides derive different keys for a cwd under work_dir (historic
    // work_dir vs data_dir divergence).
    services
        .knowledge_service
        .add_managed_root(services.terminal_service.work_dir());
    // Live binding propagation: when a workpath knowledge binding is
    // persisted (session-header KnowledgeControl / gateway tool), re-sync the
    // mounts + README of every live terminal on that workpath immediately.
    // The MCP capability needs no re-issue — terminal dispatch resolves the
    // live binding per call. Weak reference: the knowledge singleton must not
    // keep the terminal singleton alive.
    {
        let terminal_service = Arc::downgrade(&services.terminal_service);
        let background_tasks = services.background_tasks.clone();
        services
            .knowledge_service
            .set_binding_changed_hook(Arc::new(move |kind: &str, key: &str| {
                if kind != nomifun_knowledge::WORKPATH_BINDING_KIND {
                    return;
                }
                let Some(terminal_service) = terminal_service.upgrade() else {
                    return;
                };
                let key = key.to_owned();
                let task = tokio::spawn(async move {
                    terminal_service.resync_workpath_knowledge(&key).await;
                });
                background_tasks.register(task);
            }));
    }
    // Clear the terminal-domain owner of any requirement this terminal owned;
    // the ownership boundary has no FK cascade (spec §9.B). Mirror of the
    // conversation delete hook.
    services
        .terminal_service
        .with_delete_hook(services.requirement_service.clone() as Arc<dyn OnTerminalDelete>);
    let lifecycle_notice = Arc::new(AgentTerminalLifecycleNotice {
        runtimes: services.agent_runtime_sessions.clone(),
    });
    // `terminal.exit` is emitted only after the PTY exit status and final
    // scrollback have been persisted. Observe the internal owner-scoped event
    // so natural child exits (not just REST kill/delete requests) update the
    // owning Agent's trusted resource context.
    spawn_terminal_exit_agent_notice_forwarder(
        services,
        lifecycle_notice.clone(),
    );

    // Reuse the singleton terminal service (owns the live PTY map), so the
    // terminal routes and the AutoWork runner share the same PTYs.
    TerminalRouterState::new(services.terminal_service.clone())
        .with_conversation_notice_sink(lifecycle_notice)
}

/// Build the Requirements queue controller over the single AgentExecution
/// state machine. AutoWork retains only binding, claim and queue policy; it
/// cannot acquire a Runtime lease or deliver a Conversation turn directly.
pub fn build_requirement_state(
    services: &AppServices,
    conversation_owner: Arc<NomiCoreSessionOwner>,
    agent_execution: Arc<AgentExecutionEngine>,
) -> RequirementRouterState {
    let autowork_waker = Arc::new(tokio::sync::Notify::new());
    let session_config: Arc<dyn nomifun_requirement::AutoWorkSessionConfigPort> =
        conversation_owner.clone();
    let requirement_service = Arc::new(
        (*services.requirement_service)
            .clone()
            .with_session_config_port(session_config)
            .with_scheduled_session_lookup(conversation_owner.clone())
            .with_autowork_waker(autowork_waker.clone()),
    );
    let execution: Arc<dyn nomifun_requirement::AutoWorkExecutionPort> = agent_execution;
    let workspace: Arc<dyn nomifun_requirement::AutoWorkWorkspacePort> =
        conversation_owner.clone();
    let deps = Arc::new(nomifun_requirement::AutoWorkRunnerDeps {
        authoritative_user_id: services.authoritative_user_id.clone(),
        service: requirement_service.clone(),
        execution,
        workspace,
        wake: autowork_waker,
    });
    let auto_work_runner = Arc::new(nomifun_requirement::AutoWorkRunner::new(deps));
    services.set_auto_work_runner(auto_work_runner.clone());
    RequirementRouterState {
        requirement_service,
        auto_work_runner,
    }
}

/// Build the `WebhookRouterState` (webhook CRUD + per-tag settings). Constructs
/// fresh repos + a platform-dispatching sender from the pool, matching the per-builder pattern.
/// Shares the same DB tables as the completion notifier wired in `AppServices`.
pub fn build_webhook_state(services: &AppServices) -> WebhookRouterState {
    let pool = services.database.pool().clone();
    let webhook_repo: Arc<dyn nomifun_db::IWebhookRepository> =
        Arc::new(nomifun_db::SqliteWebhookRepository::new(pool.clone()));
    let tag_setting_repo: Arc<dyn nomifun_db::ITagSettingRepository> =
        Arc::new(nomifun_db::SqliteTagSettingRepository::new(pool));
    let sender: Arc<dyn nomifun_webhook::WebhookSender> = Arc::new(nomifun_webhook::DefaultWebhookSender::new());
    let service = nomifun_webhook::WebhookService::new(webhook_repo, tag_setting_repo, sender);
    WebhookRouterState { service }
}

/// Build the Creative Studio router state, reusing the singleton project/asset
/// service and its on-disk asset binaries.
pub fn build_workshop_state(services: &AppServices) -> WorkshopRouterState {
    WorkshopRouterState::new(
        services.workshop_service.clone(),
        Arc::new(crate::services::AgentTemplateDraftRunner {
            model_invoke: services.model_invoke_service.clone(),
            workspace: services.data_dir.clone(),
        }),
    )
}

/// Build the 生成引擎 (creation) router state, reusing the singleton
/// `creation_service`. Creation task/asset reconciliation is completed during
/// `AppServices::from_config`, before this router can accept new generation
/// tasks, so a live write cannot race the boot inventory snapshot.
pub fn build_creation_state(services: &AppServices) -> CreationRouterState {
    CreationRouterState::new(services.creation_service.clone())
}

/// Build the single Agent Execution facade shared by REST, model tools and boot
/// recovery. Planner/router/scheduler/executor remain private engine strategies.
pub fn build_agent_execution_engine(
    services: &AppServices,
    conversation_owner: Arc<NomiCoreSessionOwner>,
) -> Arc<AgentExecutionEngine> {
    let repository: Arc<dyn IAgentExecutionRepository> = Arc::new(
        SqliteAgentExecutionRepository::new(services.database.pool().clone()),
    );
    let template_repository: Arc<dyn IAgentExecutionTemplateRepository> = Arc::new(
        SqliteAgentExecutionTemplateRepository::new(services.database.pool().clone()),
    );
    let provider_repository: Arc<dyn IProviderRepository> = Arc::new(
        SqliteProviderRepository::new(services.database.pool().clone()),
    );
    let provider_model_repository: Arc<dyn nomifun_db::IProviderModelRepository> = Arc::new(
        nomifun_db::SqliteProviderModelRepository::new(services.database.pool().clone()),
    );
    // Transitional only: AgentExecution consumes the public typed Session port
    // and no longer receives the Session owner or Runtime registry as
    // production configuration. The adapter is a pure delegate over the
    // existing owner while the canonical AgentSession implementation replaces
    // the remaining Conversation-backed operations.
    let session: Arc<dyn nomifun_agent_execution::AgentExecutionSessionPort> =
        conversation_owner;
    let engine = Arc::new(AgentExecutionEngine::new(AgentExecutionEngineConfig {
        repository,
        template_repository,
        provider_repository,
        provider_model_repository,
        provider_model_capability_repository: services.provider_model_capability_repo.clone(),
        realtime: services.ws_manager.clone(),
        session,
        model_invoke: services.model_invoke_service.clone(),
        workspace_root: services.work_dir.clone(),
        lifecycle: services.agent_execution_lifecycle.clone(),
    }));
    engine
}

/// Build the `CompanionRouterState` (the "nomi" desktop companion: opt-in event
/// collection, scheduled learning, memories, companion chat). Reuses the
/// singleton `services.companion_service` (constructed in `AppServices::from_config`
/// before the agent factory, which holds its memory sink) and late-wires the
/// companion thread manager with the canonical AgentSession owner.
pub fn build_companion_state(
    services: &AppServices,
    channel_manager: Arc<nomifun_channel::manager::ChannelManager>,
    conversation_owner: Arc<NomiCoreSessionOwner>,
) -> CompanionRouterState {
    // Deleting a companion must also drop its ('companion', id) knowledge-binding row so
    // bindings don't orphan (T3.3). Switching a companion's chat model (single source
    // of truth) clears bound IM sessions. Physical endpoints share the same
    // Companion conversation and require no model propagation or boot repair.
    // Deleting a companion likewise clears its channel bindings. All are
    // best-effort cleanup hooks.
    services.companion_service.set_cleanup_hooks(vec![
        Arc::new(CompanionKnowledgeCleanup {
            knowledge: services.knowledge_service.clone(),
        }),
        Arc::new(CompanionChannelModelSync {
            manager: channel_manager,
        }),
        Arc::new(CompanionRobotCleanup { robot: services.robot.clone() }),
    ]);


    let transcript: Arc<dyn nomifun_companion::evolution::TranscriptSource> =
        Arc::new(super::nomi_core_session::CanonicalAgentTranscriptSource::new(
            conversation_owner.clone(),
            services.authoritative_user_id.clone(),
        ));
    let companion_ports = nomifun_companion::companion_ports_from_typed_host(
        services.authoritative_user_id.clone(),
        conversation_owner.clone(),
        conversation_owner,
        transcript,
    );
    services.companion_service.attach_companion(companion_ports);
    CompanionRouterState::new(services.companion_service.clone())
}

/// Companion-delete cascade hook: drops the deleted companion's knowledge binding via
/// `KnowledgeService::delete_binding("companion", …)`. Failures are logged, never
/// propagated (hook contract —the companion is already gone).
struct CompanionKnowledgeCleanup {
    knowledge: Arc<nomifun_knowledge::KnowledgeService>,
}

#[async_trait::async_trait]
impl nomifun_companion::service::CompanionCleanupHook for CompanionKnowledgeCleanup {
    async fn on_companion_deleted(&self, companion_id: &str) {
        if let Err(e) = self.knowledge.delete_binding("companion", companion_id).await {
            tracing::warn!(companion_id, error = %e, "failed to delete companion knowledge binding");
        }
    }
}

fn spawn_terminal_exit_agent_notice_forwarder(
    services: &AppServices,
    notice_sink: Arc<AgentTerminalLifecycleNotice>,
) {
    let mut events = services.event_bus.subscribe_user();
    let terminal_service = services.terminal_service.clone();
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        tracing::warn!(
            "terminal exit Agent-notice forwarder was not started because no Tokio runtime is active"
        );
        return;
    };
    runtime.spawn(async move {
        loop {
            let envelope = match events.recv().await {
                Ok(envelope) => envelope,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(
                        skipped,
                        "terminal exit Agent-notice forwarder lagged; scoped terminal state remains authoritative"
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            if envelope.event.name != "terminal.exit" {
                continue;
            }
            let exit = match serde_json::from_value::<TerminalExitEvent>(
                envelope.event.data,
            ) {
                Ok(exit) => exit,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "ignored malformed terminal.exit event in Agent-notice forwarder"
                    );
                    continue;
                }
            };
            if let Err(error) = terminal_service
                .authorize_user(&envelope.user_id, exit.terminal_id.as_str())
                .await
            {
                tracing::debug!(
                    terminal_id = %exit.terminal_id,
                    error = %error,
                    "ignored terminal.exit event whose owner no longer authorizes the terminal"
                );
                continue;
            }
            let session = match terminal_service.get(exit.terminal_id.as_str()).await {
                Ok(session) => session,
                Err(error) => {
                    tracing::debug!(
                        terminal_id = %exit.terminal_id,
                        error = %error,
                        "terminal exited but its owner conversation could not be resolved"
                    );
                    continue;
                }
            };
            // A queued exit event can race with relaunch of the same terminal
            // id. Only forward it while the durable row still describes this
            // exact exit; otherwise the event belongs to an obsolete PTY epoch
            // and would incorrectly tell the Agent that the replacement is
            // gone.
            if !terminal_exit_matches_current_state(
                &session.last_status,
                session.exit_code,
                exit.exit_code,
            ) {
                tracing::debug!(
                    terminal_id = %exit.terminal_id,
                    event_exit_code = ?exit.exit_code,
                    current_status = session.last_status,
                    current_exit_code = ?session.exit_code,
                    "ignored stale terminal.exit event after terminal state advanced"
                );
                continue;
            }
            let Some(conversation_id) = session.owner_conversation_id else {
                continue;
            };
            notice_sink.notify_terminal_exit(
                conversation_id.as_str(),
                exit.terminal_id.as_str(),
                exit.exit_code,
            );
        }
    });
}

fn terminal_exit_matches_current_state(
    current_status: &str,
    current_exit_code: Option<i32>,
    event_exit_code: Option<i32>,
) -> bool {
    current_status == "exited" && current_exit_code == event_exit_code
}

/// Feed terminal lifecycle state back into a Nomi runtime at its next model
/// boundary. This is intentionally best-effort: scoped terminal tools remain
/// the durable source of truth, while the trusted system-resource notice keeps
/// a present runtime from relying on stale process state.
struct AgentTerminalLifecycleNotice {
    runtimes: Arc<dyn AgentRuntimeSessions>,
}

impl AgentTerminalLifecycleNotice {
    fn notify(
        &self,
        conversation_id: &str,
        terminal_id: &str,
        lifecycle: String,
    ) {
        let Some(runtime) = self.runtimes.get_runtime(conversation_id) else {
            tracing::debug!(
                conversation_id,
                terminal_id,
                lifecycle,
                "terminal lifecycle changed with no registered Agent runtime"
            );
            return;
        };
        let notice = format!(
            "Terminal {terminal_id} {lifecycle}. Treat any previous running \
             state as stale and call nomi_list_terminals before further \
             terminal actions."
        );
        match runtime.notify_system_resource(notice) {
            Ok(delivery) => {
                tracing::debug!(
                    conversation_id,
                    terminal_id,
                    lifecycle,
                    ?delivery,
                    "queued terminal lifecycle as trusted Agent resource state"
                );
            }
            Err(error) => {
                tracing::info!(
                    conversation_id,
                    terminal_id,
                    lifecycle,
                    agent_type = runtime.agent_type().serde_name(),
                    error = %error,
                    "Agent runtime has no trusted system-resource channel; terminal notice remains best-effort via scoped state"
                );
            }
        }
    }

    fn notify_terminal_exit(
        &self,
        conversation_id: &str,
        terminal_id: &str,
        exit_code: Option<i32>,
    ) {
        let lifecycle = match exit_code {
            Some(code) => format!("transitioned to exited (exit_code={code})"),
            None => "transitioned to exited (exit code unavailable)".to_owned(),
        };
        self.notify(conversation_id, terminal_id, lifecycle);
    }
}

impl nomifun_terminal::TerminalConversationNoticeSink for AgentTerminalLifecycleNotice {
    fn notify_terminal_lifecycle(
        &self,
        conversation_id: &str,
        terminal_id: &str,
        event: &'static str,
    ) {
        self.notify(
            conversation_id,
            terminal_id,
            format!("received lifecycle event `{event}`"),
        );
    }
}

/// Companion model-switch / delete → IM channel session sync. The companion's chat model is
/// the single source of truth; when it changes (or the companion is deleted), the
/// channel sessions bound to that companion are cleared so the next inbound IM message
/// recreates the backing conversation with the current model. Best-effort.
struct CompanionChannelModelSync {
    manager: Arc<nomifun_channel::manager::ChannelManager>,
}

#[async_trait::async_trait]
impl nomifun_companion::service::CompanionCleanupHook for CompanionChannelModelSync {
    async fn on_companion_deleted(&self, companion_id: &str) {
        self.manager.unbind_channels_for_deleted_companion(companion_id).await;
    }
    async fn on_companion_model_changed(
        &self,
        companion_id: &str,
        _model: Option<&nomifun_common::ProviderWithModel>,
    ) {
        self.manager.clear_sessions_for_companion(companion_id).await;
    }
}

/// Deleting a Companion revokes every physical endpoint bound to it.
struct CompanionRobotCleanup {
    robot: Option<Arc<crate::robot_wiring::RobotServices>>,
}

#[async_trait::async_trait]
impl nomifun_companion::service::CompanionCleanupHook for CompanionRobotCleanup {
    async fn on_companion_deleted(&self, companion_id: &str) {
        let Some(robot) = self.robot.as_ref() else { return; };
        for record in robot.registry.list().await {
            if record.companion_id.as_deref() == Some(companion_id) {
                if let Err(error) = robot.registry.patch(&record.robot_id, None, Some(None)).await {
                    tracing::warn!(%error, "could not unbind deleted Companion device");
                }
                robot.tools.detach_if_disconnected(&robot.registry, &record.robot_id).await;
                robot.status.mark_offline_if_disconnected(&robot.registry, &record.robot_id, nomifun_common::now_ms()).await;
            }
        }
    }
}

/// Build the default `CronRouterState` from application services.
pub fn build_cron_state(
    services: &AppServices,
    conversation_owner: Arc<NomiCoreSessionOwner>,
) -> CronRouterState {
    let pool = services.database.pool().clone();
    let cron_repo: Arc<dyn nomifun_db::ICronRepository> = Arc::new(nomifun_db::SqliteCronRepository::new(pool.clone()));

    let busy_guard = Arc::new(nomifun_cron::busy_guard::CronBusyGuard::new());
    let cron_sessions: Arc<dyn nomifun_cron::CronSessionPort> = conversation_owner;
    let executor = Arc::new(nomifun_cron::executor::JobExecutor::new(
        services.authoritative_user_id.clone(),
        cron_sessions,
        busy_guard,
        services.data_dir.clone(),
    ));

    let tick_service_ref: Arc<CronServiceTickRef> = Arc::new(CronServiceTickRef::default());
    let tick_ref = tick_service_ref.clone();
    let background_tasks = services.background_tasks.clone();
    let scheduler = Arc::new(nomifun_cron::scheduler::CronScheduler::new(Arc::new(
        move |
            job_id: String,
            user_id: String,
            schedule_revision: i64,
            planned_at_ms: nomifun_common::TimestampMs,
            generation: u64,
        | {
            let svc = tick_ref.0.lock().unwrap().clone();
            let task = tokio::spawn(async move {
                if let Some(svc) = svc {
                    svc.tick_occurrence_with_generation(
                        &user_id,
                        &job_id,
                        schedule_revision,
                        planned_at_ms,
                        generation,
                    )
                    .await;
                }
            });
            background_tasks.register(task);
        },
    )));

    let emitter = CronEventEmitter::new(services.event_bus.clone());
    let cron_service = Arc::new(nomifun_cron::service::CronService::new(
        services.authoritative_user_id.clone(),
        cron_repo,
        scheduler,
        executor,
        emitter,
        services.data_dir.clone(),
    ));
    cron_service.with_cron_background_task_registrar(
        services.background_tasks.clone()
            as Arc<dyn nomifun_cron::CronBackgroundTaskRegistrar>,
    );
    services.set_cron_service(cron_service.clone());

    tick_service_ref.0.lock().unwrap().replace(cron_service.clone());

    CronRouterState {
        cron_service,
    }
}

/// Build the default `OfficeRouterState` from application services.
pub fn build_office_state(services: &AppServices) -> OfficeRouterState {
    let data_dir = services.data_dir.as_path();
    let allowed_roots = default_allowed_roots(Some(services.work_dir.as_path()));

    let spawner: Arc<dyn nomifun_office::ProcessSpawner> = Arc::new(nomifun_office::DefaultProcessSpawner);
    let watch_manager = Arc::new(OfficecliWatchManager::new(spawner, services.event_bus.clone()));

    let snapshot_service = Arc::new(OfficeSnapshotService::new(data_dir));
    let proxy_service = Arc::new(ProxyService::new(watch_manager.clone()));

    OfficeRouterState {
        watch_manager,
        snapshot_service,
        proxy_service,
        allowed_roots,
    }
}

/// Build the default `ShellRouterState` from application services.
pub fn build_shell_state(services: &AppServices) -> ShellRouterState {
    let pool = services.database.pool().clone();
    let client_pref_repo = Arc::new(SqliteClientPreferenceRepository::new(pool.clone()));
    let client_pref_service = ClientPrefService::new(client_pref_repo);
    let provider_repo = Arc::new(SqliteProviderRepository::new(pool.clone()));
    let provider_model_repo = Arc::new(nomifun_db::SqliteProviderModelRepository::new(pool.clone()));
    let capability_repo = Arc::new(
        nomifun_db::SqliteProviderModelCapabilityRepository::new(pool.clone()),
    );
    let connection_repo = Arc::new(
        nomifun_db::SqliteProviderConnectionRepository::new(pool),
    );

    ShellRouterState {
        shell_service: Arc::new(nomifun_shell::ShellService::new(Arc::new(
            nomifun_shell::DefaultSystemOpener,
        ))),
        stt_service: Arc::new(nomifun_shell::SttService::new(Some(
            services.model_invoke_service.clone(),
        ))),
        client_pref_service,
        provider_service: Some(ProviderService::new(
            provider_repo,
            provider_model_repo,
            capability_repo,
            connection_repo,
            services.encryption_key,
        )),
        // The process-wide invoke singleton (assembled in AppServices next to
        // the creation service) backs `/api/tts` and, via SttService, `/api/stt`.
        model_invoke_service: Some(services.model_invoke_service.clone()),
    }
}

/// Helper to break the circular reference between CronScheduler and CronService.
#[derive(Default)]
struct CronServiceTickRef(std::sync::Mutex<Option<Arc<nomifun_cron::service::CronService>>>);

/// Build the default Skill Library router state.
///
/// Skill discovery and managed paths are owned by `nomifun-skill-library`;
/// there is no legacy Extension registry or Hub in the production composition.
pub async fn build_skill_state(services: &AppServices) -> SkillRouterState {
    let skill_data_dir = services.data_dir.clone();

    let app_resource_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .and_then(|p| p.parent().map(|pp| pp.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let skill_paths = nomifun_skill_library::resolve_skill_paths(&app_resource_dir, &skill_data_dir);

    let ext_paths_mgr = Arc::new(ExternalPathsManager::new(&skill_data_dir).await);

    let skill_tag_repo: Arc<dyn nomifun_db::ISkillTagRepository> =
        Arc::new(nomifun_db::SqliteSkillTagRepository::new(services.database.pool().clone()));
    let builtin_skill_tags = Arc::new(nomifun_skill_library::skill_service::load_builtin_skill_tags());

    SkillRouterState {
        skill_paths,
        external_paths_manager: ext_paths_mgr,
        skill_tag_repo,
        builtin_skill_tags,
    }
}

/// Build the default `WsHandlerState` from application services.
pub fn build_ws_state(services: &AppServices) -> WsHandlerState {
    // Operator escape hatch for deployments behind a reverse proxy that
    // forwards neither the original `Host` nor `X-Forwarded-Host`: the WS
    // handshake additionally accepts these exact browser origins.
    let allowed_origins: Arc<[String]> = std::env::var("NOMIFUN_ALLOWED_ORIGINS")
        .map(|raw| nomifun_realtime::parse_allowed_origins(&raw))
        .unwrap_or_default()
        .into();

    // NoAuth: every upgrade is accepted (dev / `--insecure-no-auth`).
    if services.auth_policy.is_no_auth() {
        let authoritative_user_id = services.authoritative_user_id.to_string();
        return WsHandlerState {
            manager: services.ws_manager.clone(),
            token_authenticator: Arc::new(move |_| Some(authoritative_user_id.clone())),
            token_extractor: Arc::new(|_| Some("local".into())),
            allowed_origins,
        };
    }

    // Required / TrustLocalToken: accept either the per-boot local-trust secret
    // (the desktop webview presents it as a `Sec-WebSocket-Protocol` value,
    // since browsers cannot set custom headers on the WS handshake) or a valid
    // JWT (remote logged-in browser).
    let jwt_service = services.jwt_service.clone();
    let local_secret = services.local_trust_secret.clone();
    let authoritative_user_id = services.authoritative_user_id.to_string();
    let token_authenticator = Arc::new(move |token: &str| {
        if let Some(secret) = local_secret.as_deref()
            && token == secret
        {
            return Some(authoritative_user_id.clone());
        }
        jwt_service.verify(token).ok().map(|claims| claims.user_id.into_string())
    });

    let token_extractor = Arc::new(|headers: &axum::http::HeaderMap| extract_token_from_ws_headers(headers));

    WsHandlerState {
        manager: services.ws_manager.clone(),
        token_authenticator,
        token_extractor,
        allowed_origins,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::AppConfig;

    #[test]
    fn product_agent_errors_keep_the_compiler_reason() {
        let error = nomifun_agent_control_plane::ControlPlaneError::with_details(
            "PRESET_REVISION_SAVE_FAILED", StatusCode::UNPROCESSABLE_ENTITY,
            "template expansion did not pass compiler validation",
            serde_json::json!({"diagnostics": [{"code": "AGENT_RUNTIME_ENGINE_UNAVAILABLE", "message": "MCP owner paths cannot be mixed"}]}),
        );
        let mapped = control_plane_error_to_app(error);
        assert_eq!(mapped.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(mapped.to_string().contains("MCP owner paths cannot be mixed"));
    }

    #[test]
    fn terminal_exit_notice_rejects_stale_relaunch_epochs() {
        assert!(terminal_exit_matches_current_state(
            "exited",
            Some(0),
            Some(0)
        ));
        assert!(
            !terminal_exit_matches_current_state("running", None, Some(0)),
            "an old exit event must not describe a relaunched running PTY"
        );
        assert!(
            !terminal_exit_matches_current_state("exited", Some(1), Some(0)),
            "an exit event from another PTY epoch must not override current state"
        );
    }

    /// The pill must report on the socket the agent is actually using. Two pools
    /// (which is what this codebase had before) means the routes describe links
    /// nobody talks to while the live ones stay invisible — so pin the identity,
    /// not just the behaviour.
    #[tokio::test]
    async fn ssh_pool_is_shared_between_routes_and_the_domain_adapter() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("data");
        let db = nomifun_db::init_database_memory().await.unwrap();
        let config = AppConfig {
            data_dir: data_dir.clone(),
            work_dir: data_dir,
            ..Default::default()
        };
        let services = AppServices::from_config(db, &config).await.unwrap();

        let routed = build_ssh_host_state(&services)
            .pool
            .expect("the ssh host routes must be backed by the process pool");
        assert!(
            routed.is_same_pool(&services.ssh_pool),
            "build_ssh_host_state must reuse services.ssh_pool instead of building its own"
        );

        // The host-facing adapter uses the same pool. A link it
        // opens — including one that failed to dial, which is precisely what the
        // header pill has to show — must be visible through the routes' handle.
        let provider: Arc<dyn nomifun_ai_agent::SshBackendProvider> =
            Arc::new(services.ssh_pool.clone());
        let unsaved_host = nomifun_common::SshHostId::new();
        let dialled = provider
            .connect(
                services.authoritative_user_id.as_ref(),
                "conversation-with-no-saved-host",
                unsaved_host.as_str(),
                ".",
            )
            .await;
        assert!(
            dialled.is_err(),
            "a host that is not in the book cannot be dialled"
        );
        assert_eq!(
            routed.active_link_count(),
            1,
            "the routes must see the link the agent's provider just opened"
        );

        let services_source = include_str!("../services.rs");
        assert_eq!(
            services_source.matches("SshConnectionPool::new(").count(),
            1,
            "exactly one ssh connection pool may exist in the process"
        );
        let state_source = include_str!("state.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert!(
            !state_source.contains("SshConnectionPool::new("),
            "the router must not build a second pool the agent cannot see"
        );

        services.database.close().await;
    }
}
