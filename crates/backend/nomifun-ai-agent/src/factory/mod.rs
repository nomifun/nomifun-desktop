#[cfg(feature = "browser-use")]
pub mod browser_lane;
pub mod provider_config;

mod context;
pub(crate) mod nomi;

use std::path::PathBuf;
use std::sync::Arc;

use futures_util::FutureExt;
use nomi_agent::companion_tools::{CompanionMemorySink, CompanionSkillSink};
use nomi_agent::requirement_tools::RequirementSink;
use nomifun_api_types::{GatewayMcpConfig, ModelTask};
use nomifun_common::{AppError, ExecutionAuthority};
use nomifun_db::{IClientPreferenceRepository, IMcpServerRepository, ISettingsRepository};
use nomifun_model_invoke::{ModelInvokeService, ModelRef};

use crate::runtime_handle::AgentRuntimeHandle;
use crate::factory::context::FactoryContext;
use crate::runtime_registry::{
    AgentRuntimeFactory, AgentRuntimeModelConfigResolver, RuntimeModelConfigBinding,
};
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
    /// Single task-capability and connection resolver used by every Nomi Chat
    /// build and by native image generation.
    pub model_invoke: Arc<ModelInvokeService>,
    /// Native image generation uses the same process-wide invoke service as
    /// chat when this capability is enabled. `None` is reserved for
    /// lightweight tests and standalone hosts that must not expose the tool.
    pub model_invoke_service: Option<Arc<ModelInvokeService>>,
    pub encryption_key: [u8; 32],
    pub data_dir: PathBuf,
    /// Root for auto-provisioned managed workspaces
    /// (`{work_dir}/conversations/{uuidv7}`). Defaults to the data
    /// dir at composition; kept as its own field so the fallback in
    /// `FactoryContext::resolve` stays in sync with `ConversationService`,
    /// which provisions under `AppConfig.work_dir` — a `--work-dir` /
    /// `NOMIFUN_WORK_DIR` override must not split the two roots.
    pub work_dir: PathBuf,
    /// Platform Gateway MCP server config. When `Some`, the factory injects it
    /// only after resolving installation-owner authority. `None` when the
    /// gateway server failed to start (graceful degradation).
    pub gateway_mcp_config: Option<GatewayMcpConfig>,
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
    /// System-settings repo for localized deterministic native messages (for
    /// example image-generation acknowledgements). Agent reasoning and replies
    /// follow the language of each current user request instead of this UI
    /// setting. `Option` lets tests omit the repository and use the host/default
    /// locale.
    pub settings_repo: Option<Arc<dyn ISettingsRepository>>,
    /// User-configured MCP servers repository. Used by the nomi factory to
    /// inject enabled servers into the session's MCP client set.
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
    /// Optional sink enabling the nomi native computer-history tools
    /// (`computer_history_*`). Only registered for installation-owner nomi
    /// sessions; `None` (standalone, tests, restricted principals) leaves them
    /// unregistered.
    pub computer_history_sink: Option<Arc<dyn nomi_agent::computer_history_tools::ComputerHistorySink>>,
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
    /// read-only `recall_memories` tool and injects the
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
/// runtime is currently polling it. This lets construction await IO directly,
/// without the scoped-thread + `block_on` bridge the old sync-factory version
/// needed.
pub fn build_agent_factory(deps: AgentFactoryDeps) -> AgentRuntimeFactory {
    let deps = Arc::new(deps);

    Arc::new(move |options: AgentRuntimeBuildOptions| {
        let deps = deps.clone();
        async move { build_agent(deps, options).await }.boxed()
    })
}

/// Build the exact provider-configuration revision resolver used to fence
/// long-lived Nomi runtime reuse. It intentionally reuses ModelInvoke's Chat
/// resolver so runtime admission and the factory consume one capability graph.
pub fn build_agent_model_config_resolver(
    model_invoke: Arc<ModelInvokeService>,
) -> AgentRuntimeModelConfigResolver {
    Arc::new(move |selection| {
        let model_invoke = Arc::clone(&model_invoke);
        async move {
            let model = selection.use_model.unwrap_or(selection.model);
            let resolved = model_invoke
                .resolve_task_config(
                    &ModelRef {
                        provider_id: selection.provider_id,
                        model,
                    },
                    ModelTask::Chat,
                )
                .await
                .map_err(|error| AppError::BadRequest(error.to_string()))?;
            Ok(RuntimeModelConfigBinding {
                provider_id: resolved.provider_id,
                model: resolved.model,
                config_revision: resolved.config_revision,
            })
        }
        .boxed()
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

    // Nomi is the only executor, and it is safe for a model-only principal:
    // its own factory applies the model-only capability ceiling. There is no
    // longer a host-code-executing engine to reject at this boundary.
    let ctx = FactoryContext::resolve(&deps, &options).await?;
    nomi::build(deps, options, ctx, authority).await
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
