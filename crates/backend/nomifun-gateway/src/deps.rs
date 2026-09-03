//! Legacy compatibility host for the pre-Fresh-v4 Gateway transport.

use std::sync::Arc;

use nomifun_ai_agent::AgentService;
use nomifun_common::{CompanionId, ConversationId, UserId};
use nomifun_companion::CompanionService;
use nomifun_cron::service::CronService;
use nomifun_db::IProviderRepository;
use nomifun_idmm::IdmmService;
use nomifun_knowledge::KnowledgeService;
use nomifun_requirement::{AutoWorkRunner, RequirementService};
use nomifun_system::{
    ClientPrefService, ModelFetchService, ProviderService, SettingsService,
};
use nomifun_terminal::TerminalService;

use crate::conversation_port::ConversationCapabilityPort;

/// Compatibility-only composition input for the legacy Gateway transport.
///
/// Fresh-v4 production entry points dispatch through `AgentPlatform` and never
/// construct this host. The explicit compatibility router builds it after its
/// legacy module states exist, then wires it into [`crate::GatewayMcpServer`].
///
/// Capability modules must immediately project a domain-specific dependency
/// view before executing business logic. This root is not a canonical service
/// locator and must not be added to Fresh-v4 composition.
pub struct CompatibilityCapabilityHost {
    /// Canonical installation owner. Every installation-scoped capability is
    /// gated against this same immutable identity before its handler runs.
    pub authoritative_user_id: Arc<str>,
    pub conversation: Arc<dyn ConversationCapabilityPort>,
    pub cron_service: Arc<CronService>,
    /// MUST be the router-state instance with its session-owner and
    /// terminal-driver attachments from `build_requirement_state`; AutoWork
    /// config tools need those attachments and the bare singleton would error
    /// "not attached".
    pub requirement_service: Arc<RequirementService>,
    pub companion_service: Arc<CompanionService>,
    /// Singleton terminal service (owns the live PTY map shared with the
    /// terminal routes + AutoWork runner).
    pub terminal_service: Arc<TerminalService>,
    /// Main-db provider rows: model listing + the nomi model resolution chain.
    pub provider_repo: Arc<dyn IProviderRepository>,
    /// Authoritative per-model rows (membership + enabled flags) backing the
    /// provider summaries since migration 016 dropped the legacy columns.
    pub provider_model_repo: Arc<dyn nomifun_db::IProviderModelRepository>,
    /// Task-scoped capabilities for each provider model. Gateway model
    /// selection never infers a task from a model name.
    pub provider_model_capability_repo:
        Arc<dyn nomifun_db::IProviderModelCapabilityRepository>,
    /// IDMM supervision config (same instance as `/api/idmm` so save also
    /// arms/stops the live supervisor).
    pub idmm_service: Arc<IdmmService>,
    /// Canonical Creative Studio project/asset service, the same singleton the
    /// `/api/creative-studio/*` routes use.
    pub workshop_service: Arc<nomifun_workshop::WorkshopService>,
    /// Creative Studio generation task queue, the same singleton the canonical
    /// task routes use.
    pub creation_service: Arc<nomifun_creation::CreationService>,
    /// Knowledge base registry + bindings (same instance the conversation
    /// service mounts from at task start).
    pub knowledge_service: Arc<KnowledgeService>,
    /// AutoWork live-loop control shared with the REST routes.
    pub auto_work_runner: Arc<AutoWorkRunner>,
    /// System domain services shared with the REST routes.
    pub settings_service: SettingsService,
    pub client_pref_service: ClientPrefService,
    pub provider_service: ProviderService,
    pub model_fetch_service: ModelFetchService,
    /// Channel domain state shared with the channel routes.
    pub channel_state: nomifun_channel::ChannelRouterState,
    /// Filesystem service (path-scoped to the configured allowed roots).
    pub file_service: nomifun_file::FileServiceRef,
    /// Shell-open service (OS ShellExecute / `open`).
    pub shell_service: Arc<nomifun_shell::ShellService>,
    /// MCP server CRUD (same instance as the `/api/mcp` routes).
    pub mcp_config_service: nomifun_mcp::McpConfigService,
    /// Extension registry + hub + skills.
    pub extension_registry: nomifun_extension::ExtensionRegistry,
    pub hub_index_manager: nomifun_extension::HubIndexManager,
    pub hub_installer: nomifun_extension::HubInstaller,
    pub skill_paths: nomifun_extension::SkillPaths,
    /// Agent catalog (same instance as the agent routes).
    pub agent_service: Arc<AgentService>,
    /// Client-preference repo backing the global model-failover config.
    pub client_pref_repo: Arc<dyn nomifun_db::IClientPreferenceRepository>,
    /// One shared persistent collaboration facade.
    pub agent_execution_engine: Arc<nomifun_agent_execution::AgentExecutionEngine>,
}

/// Identity of the calling Agent session, reconstructed only from the
/// validated signed Gateway child capability forwarded by the stdio bridge.
#[derive(Debug, Clone)]
pub struct CallerCtx {
    /// The conversation the calling agent lives in.
    pub conversation_id: Option<ConversationId>,
    /// The desktop user every tool scopes its data access to.
    pub user_id: UserId,
    /// The companion the calling session is bound to.
    pub companion_id: Option<CompanionId>,
    /// IM platform when this is a Channel Agent session.
    pub channel_platform: Option<String>,
    /// Authenticated, transport-derived identity for the current mutating
    /// operation.
    pub operation_id: Option<String>,
}

impl Default for CallerCtx {
    fn default() -> Self {
        Self {
            conversation_id: None,
            user_id: UserId::new(),
            companion_id: None,
            channel_platform: None,
            operation_id: None,
        }
    }
}
