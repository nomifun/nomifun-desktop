pub mod acp_assembler;
#[cfg(feature = "browser-use")]
pub mod browser_lane;
pub mod provider_config;

mod acp;
pub(crate) mod construction_guard;
mod context;
mod nanobot;
pub(crate) mod nomi;
pub(crate) mod platform_table;
mod openclaw;
mod remote;

use std::path::PathBuf;
use std::sync::Arc;

use futures_util::FutureExt;
use nomi_agent::companion_tools::{CompanionMemorySink, CompanionSkillSink};
use nomi_agent::requirement_tools::RequirementSink;
use nomifun_api_types::{
    BrowserMcpConfig, ComputerMcpConfig, GatewayMcpConfig, OpenMcpConfig, RequirementMcpConfig,
};
use nomifun_common::{AgentType, AppError, ExecutionAuthority};
use nomifun_db::{
    IClientPreferenceRepository, IMcpServerRepository, IProviderRepository, IRemoteAgentRepository,
    ISettingsRepository,
};

use crate::runtime_handle::AgentRuntimeHandle;
use crate::capability::skill_manager::AcpSkillManager;
use crate::factory::context::FactoryContext;
use crate::persistence::AcpSessionSyncService;
use crate::registry::AgentRegistry;
use crate::runtime_registry::AgentRuntimeFactory;
use crate::types::AgentRuntimeBuildOptions;

/// Builds the persona system prompt for companion-companion conversations that do
/// not carry one in their extra. Companion companion threads persist a prompt at
/// thread creation; Channel Agent sessions deliberately do NOT, so the
/// factory asks this provider at every agent build — the persona's memory
/// snapshot then refreshes whenever the agent restarts instead of being
/// frozen forever. Implemented by `nomifun-companion::CompanionService`.
#[async_trait::async_trait]
pub trait CompanionPromptProvider: Send + Sync {
    /// `companion_id` selects which companion's persona to build; `None` (or an unknown
    /// id) falls back to the host's default companion. `channel_platform` is the IM
    /// platform serving this session (e.g. "telegram"), `None` for local
    /// companion threads. Returns `None` when no companion exists.
    async fn build_system_prompt(
        &self,
        companion_id: Option<&str>,
        channel_platform: Option<&str>,
    ) -> Option<String>;
}

/// In-session companion summon support (spec §设计 B) — implemented by
/// `nomifun-companion::CompanionService` over its store; the factory only
/// consumes trait objects so the dependency direction stays acyclic (mirrors
/// [`CompanionPromptProvider`]). Consulted only for owner-authority,
/// non-companion nomi sessions whose `extra.summon` is present.
#[async_trait::async_trait]
pub trait CompanionSummonProvider: Send + Sync {
    /// Display name of the summoned companion, `None` when it no longer
    /// exists (the summon degrades to a nameless notice; tools still scope
    /// by id and simply see an empty memory set).
    async fn companion_name(&self, companion_id: &str) -> Option<String>;

    /// Read-only recall sink locked to the companion's visibility (shared +
    /// its own private memories). Its write methods refuse by construction.
    fn summon_memory_sink(
        &self,
        companion_id: &str,
    ) -> Result<Arc<dyn nomi_agent::companion_tools::CompanionMemorySink>, AppError>;

    /// `propose_companion_memory` sink: candidate memories become suggestion
    /// cards (owner-confirmed), never direct memory writes.
    fn summon_proposal_sink(
        &self,
        companion_id: &str,
    ) -> Result<Arc<dyn nomi_agent::summon_tools::SummonProposalSink>, AppError>;

    /// Per-turn live resolver for the summon's selected memory ids.
    fn summon_context_sink(
        &self,
        config: &nomifun_api_types::SummonConfig,
    ) -> Result<Arc<dyn nomi_agent::summon_tools::SummonContextSink>, AppError>;

    /// Materialize the companion's active skills minus `skill_exclusions` into
    /// the workspace `.nomi/skills` under manifest ownership (stale managed
    /// entries are pruned; user-created skills are never touched). Returns the
    /// materialized skill names.
    async fn sync_summon_workspace_skills(
        &self,
        conversation_id: &str,
        workspace: &std::path::Path,
        companion_id: &str,
        skill_exclusions: &[String],
    ) -> Result<Vec<String>, AppError>;

    /// Remove every manifest-owned summon skill from the workspace. Called on
    /// builds of non-summoned sessions so a cleared summon unloads its skills
    /// on the next runtime build. No-op for workspaces without a manifest.
    async fn clear_summon_workspace_skills(
        &self,
        conversation_id: &str,
        workspace: &std::path::Path,
    ) -> Result<(), AppError>;
}

/// Dependencies needed by the agent factory to construct agents.
pub struct AgentFactoryDeps {
    /// Canonical owner for installation-scoped tools. Every factory backend
    /// compares the persisted Conversation owner id against this immutable id
    /// before injecting host-wide MCP bridges or native singleton-domain tools.
    pub authoritative_user_id: Arc<str>,
    pub skill_manager: Arc<AcpSkillManager>,
    pub remote_agent_repo: Arc<dyn IRemoteAgentRepository>,
    pub provider_repo: Arc<dyn IProviderRepository>,
    /// Authoritative per-model rows (protocol/context-limit overrides for the
    /// selected model live here since migration 016).
    pub provider_model_repo: Arc<dyn nomifun_db::IProviderModelRepository>,
    pub encryption_key: [u8; 32],
    pub agent_registry: Arc<AgentRegistry>,
    pub acp_agent_service: Arc<AcpSessionSyncService>,
    pub data_dir: PathBuf,
    /// Root for auto-provisioned managed workspaces
    /// (`{work_dir}/conversations/{uuidv7}`). Defaults to the data
    /// dir at composition; kept as its own field so the fallback in
    /// `FactoryContext::resolve` stays in sync with `ConversationService`,
    /// which provisions under `AppConfig.work_dir` — a `--work-dir` /
    /// `NOMIFUN_WORK_DIR` override must not split the two roots.
    pub work_dir: PathBuf,
    /// Absolute path to the backend binary, reused as the `command` of stdio MCP
    /// bridges injected into ACP `session/new`.
    /// Captured once at app startup (`std::env::current_exe()`).
    pub backend_binary_path: Arc<PathBuf>,
    /// Requirement MCP server config. When `Some`, injected into ACP agent
    /// sessions so the agent gets the `requirement_complete` /
    /// `requirement_update_status` declaration tools — the ACP soft-failure fix
    /// (a clean turn with no declaration becomes `needs_review`, not silent
    /// `done`). `None` when the requirement MCP server failed to start.
    pub requirement_mcp_config: Option<RequirementMcpConfig>,
    /// Wiring for the scoped knowledge-search MCP. Injected into ACP sessions
    /// ONLY when they have bound knowledge bases (`!knowledge_mounts.is_empty()`).
    /// Its token reaches only the knowledge_search server, never the platform
    /// gateway. `None` disables ACP knowledge_search.
    pub knowledge_mcp_config: Option<nomifun_api_types::KnowledgeMcpConfig>,
    /// Platform Gateway MCP server config. When `Some`, the factory injects it
    /// only after resolving installation-owner authority. `None` when the
    /// gateway server failed to start (graceful degradation).
    pub gateway_mcp_config: Option<GatewayMcpConfig>,
    /// Reliable-launch (`open`) MCP server config. When `Some`, injected
    /// UNCONDITIONALLY into every ACP session so the agent gets the `open` tool
    /// (ShellExecute a URL/file/app) instead of fragile `cmd /c start` shell
    /// commands. Populated on Windows only — `None` on macOS/Linux (which launch
    /// reliably already) and so never injected there.
    pub open_mcp_config: Option<OpenMcpConfig>,
    /// Computer-use discrete-tool MCP server config. When `Some`, injected
    /// UNCONDITIONALLY into every ACP session so the agent gets discrete desktop
    /// tools (snapshot / click / type / launch / …). Populated on Windows only and
    /// only when the host binary has the `computer-use` feature — `None`
    /// otherwise, and so never injected there.
    pub computer_mcp_config: Option<ComputerMcpConfig>,
    /// Browser-use discrete-tool MCP server config. When `Some`, injected
    /// UNCONDITIONALLY into every ACP session so the agent gets discrete browser
    /// tools (navigate / observe / click / type / …). Populated on every desktop
    /// OS only when the host binary has the `browser-use` feature — `None`
    /// otherwise (web/headless), and so never injected there. Symmetric with
    /// `computer_mcp_config`.
    pub browser_mcp_config: Option<BrowserMcpConfig>,
    /// Late-wired issuer for native Browser Platform capabilities.
    ///
    /// `Some(slot)` means this host requires the process-wide Hub path. If the
    /// slot has not been installed when a browser-enabled Nomi runtime is
    /// built, construction fails closed; it never falls back to a private
    /// Chromium engine. `None` is reserved for explicit standalone/test hosts.
    #[cfg(feature = "browser-use")]
    pub browser_lane_provider: Option<browser_lane::BrowserLaneClientProviderSlot>,
    /// Client-preferences repo for reading user-facing settings at session-build
    /// time — currently the `agent.computerUse` toggle that gates the nomi
    /// Computer tool. `Option` so tests can omit it (then the default applies).
    /// Read live per session so toggling the setting affects new sessions without
    /// a restart.
    pub client_prefs: Option<Arc<dyn IClientPreferenceRepository>>,
    /// System-settings repo for reading the app UI language at session-build
    /// time. Companion-owned sessions (local 桌面伙伴 chat + IM Channel Agent)
    /// get a reply-language directive built from `SystemSettings.language` so the
    /// companion answers in the app's language instead of a hardcoded one.
    /// `Option` so tests can omit it (then the "en-US" default applies). Read live
    /// per build (mirrors `client_prefs`) so switching the language takes effect on
    /// the next agent (re)build.
    pub settings_repo: Option<Arc<dyn ISettingsRepository>>,
    /// User-configured MCP servers repository. Used by ACP factory to
    /// inject enabled servers into `session/new` (ELECTRON-1JG fix).
    /// `None` for tests/composition paths that do not need MCP injection.
    pub mcp_server_repo: Option<Arc<dyn IMcpServerRepository>>,
    /// Optional sink enabling nomi native requirement tools. When `Some`,
    /// `requirement_complete` / `requirement_update_status` are registered into
    /// the in-process engine. `None` (e.g. standalone) leaves them unregistered.
    pub requirement_sink: Option<Arc<dyn RequirementSink>>,
    /// Per-conversation factory for the agent's native cron tools. The app
    /// captures `CronService` here; the agent factory calls it with the
    /// authoritative user id and conversation id to build a bound `CronSink`.
    /// `None` leaves the cron tools unregistered (e.g. standalone, or cron
    /// disabled).
    pub cron_sink_factory:
        Option<Arc<dyn Fn(&str, &str) -> Arc<dyn crate::CronSink> + Send + Sync>>,
    /// Optional sink enabling the companion-companion memory tools
    /// (`recall_memories` / `save_memory` / `list_recent_events`). Only
    /// registered for conversations whose `extra.companion_session` is true.
    pub companion_sink: Option<Arc<dyn CompanionMemorySink>>,
    /// Optional sink enabling the companion's self-evolved skill auto-use
    /// (`companion_skill` tool + per-turn when_to_use ContextContributor). Only
    /// registered for companion sessions (`extra.companion_session` true).
    pub companion_skill_sink: Option<Arc<dyn CompanionSkillSink>>,
    /// Optional sink enabling the nomi native `knowledge_search` tool. When
    /// `Some` AND the session has bound knowledge bases, the tool is registered
    /// into the in-process engine. `None` (standalone) leaves it unregistered.
    pub knowledge_retrieval: Option<Arc<dyn nomi_agent::knowledge_tools::KnowledgeRetrievalSink>>,
    /// Optional sink enabling the nomi native `knowledge_write` (回血) tool. When
    /// `Some` AND the session has bound knowledge bases with write-back enabled,
    /// the tool is registered into the in-process engine and allow-listed past
    /// the approval gate. `None` (standalone) leaves it unregistered.
    pub knowledge_writeback: Option<Arc<dyn nomi_agent::knowledge_tools::KnowledgeWritebackSink>>,
    /// Optional persona prompt provider for companion_session conversations that
    /// carry no `extra.system_prompt` (Channel Agent sessions).
    pub companion_prompt: Option<Arc<dyn CompanionPromptProvider>>,
    /// Optional in-session companion summon provider (spec §设计 B). When `Some`
    /// AND an owner-authority non-companion nomi session carries `extra.summon`,
    /// the factory materializes the companion's skills, registers the read-only
    /// `recall_memories` + `propose_companion_memory` tools and injects the
    /// per-turn memory-snapshot contributor. `None` (standalone / tests) leaves
    /// summon unwired — `extra.summon` is then inert.
    pub companion_summon: Option<Arc<dyn CompanionSummonProvider>>,
    /// Optional SSH connection provider. When `Some` AND a nomi session carries
    /// `extra.ssh_host_id`, the factory connects the bound host and gives the
    /// runtime the remote tool family instead of the local one. `None` leaves
    /// SSH sessions unwired — `extra.ssh_host_id` is then inert.
    pub ssh_provider: Option<Arc<dyn crate::SshBackendProvider>>,
}

/// Build a production agent factory that dispatches to concrete agent types.
///
/// [`AgentRuntimeFactory`] is async: the returned `BoxFuture` is driven by
/// [`crate::runtime_registry::AgentRuntimeRegistry::get_or_create_runtime`] on whatever
/// runtime is currently polling it. This lets us spawn CLI processes and
/// await ACP handshakes directly, without the scoped-thread + `block_on`
/// bridge the old sync-factory version needed.
pub fn build_agent_factory(deps: AgentFactoryDeps) -> AgentRuntimeFactory {
    let deps = Arc::new(deps);

    Arc::new(move |options: AgentRuntimeBuildOptions| {
        let deps = deps.clone();
        async move { build_agent(deps, options).await }.boxed()
    })
}

fn validate_runtime_user_id(user_id: &str) -> Result<(), AppError> {
    nomifun_common::UserId::parse(user_id)
        .map(|_| ())
        .map_err(|error| AppError::BadRequest(format!("invalid Agent runtime owner id: {error}")))
}

async fn build_agent(
    deps: Arc<AgentFactoryDeps>,
    options: AgentRuntimeBuildOptions,
) -> Result<AgentRuntimeHandle, AppError> {
    validate_runtime_user_id(&options.user_id)?;
    let authority = ExecutionAuthority::resolve(
        &options.user_id,
        deps.authoritative_user_id.as_ref(),
    );

    // External ACP/OpenClaw/Nanobot/Remote runtimes execute arbitrary code as
    // the backend OS user.  Without an OS/container sandbox they can never be
    // made safe by hiding individual tools, so model-only principals are
    // rejected at the single factory boundary.  Nomi remains available under
    // the model-only ceiling applied in its factory.
    if !authority.controls_host() && options.agent_type != AgentType::Nomi {
        return Err(AppError::Forbidden(format!(
            "Agent runtime '{}' requires the installation owner; non-owner sessions are model-only",
            options.agent_type.serde_name()
        )));
    }

    let ctx = FactoryContext::resolve(&deps, &options).await?;
    match options.agent_type {
        AgentType::Acp => acp::build(deps, options, ctx).await,
        AgentType::OpenclawGateway => openclaw::build(deps, options, ctx).await,
        AgentType::Nanobot => nanobot::build(deps, options, ctx).await,
        AgentType::Remote => remote::build(deps, options, ctx).await,
        AgentType::Nomi => nomi::build(deps, options, ctx, authority).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_OWNER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";

    #[test]
    fn factory_deps_can_be_constructed() {
        // Verify types compile — actual construction requires DB
        let _: fn() -> AgentFactoryDeps = || {
            panic!("compile-time check only");
        };
    }

    #[test]
    fn installation_owner_identity_is_exact_and_fail_closed() {
        assert_eq!(
            ExecutionAuthority::resolve(TEST_OWNER_ID, TEST_OWNER_ID),
            ExecutionAuthority::InstanceOwner
        );
        assert_eq!(
            ExecutionAuthority::resolve("secondary", TEST_OWNER_ID),
            ExecutionAuthority::ModelOnly
        );
        assert_eq!(
            ExecutionAuthority::resolve("admin", TEST_OWNER_ID),
            ExecutionAuthority::ModelOnly
        );
    }

    #[test]
    fn runtime_owner_identity_is_first_class_and_canonical() {
        assert!(validate_runtime_user_id("0190f5fe-7c00-7a00-8000-000000000001").is_ok());
        assert!(validate_runtime_user_id("").is_err());
        assert!(validate_runtime_user_id("   ").is_err());
        assert!(validate_runtime_user_id(" 0190f5fe-7c00-7a00-8000-000000000001").is_err());
        assert!(validate_runtime_user_id("0190f5fe-7c00-7a00-8000-000000000001 ").is_err());
        assert!(validate_runtime_user_id("user-1").is_err());
    }
}
