//! Agent runtime lifecycle, per-conversation runtime registration, and skill management.
pub(crate) mod runtime_state;
pub mod artifact_store;
pub mod boot_process_reaper;
pub mod runtime_handle;
// Rendering page-fetch adapter for knowledge URL sources. The implementation
// consumes the application-owned Browser Session Hub and keeps the knowledge
// crate browser-platform-free.
#[cfg(feature = "browser-use")]
pub mod browser_fetcher;
pub mod capability;
pub mod cc_switch;
pub mod computer_history_sink;
pub mod factory;
pub mod image_generation;
pub mod knowledge_completer;
pub mod knowledge_retrieval;
pub mod knowledge_writeback;
pub mod manager;
pub mod nomi_session_persistence;
pub mod one_shot;
pub mod protocol;
pub mod registry;
pub mod routes;
pub(crate) mod services;
pub mod runtime_registry;
pub mod terminal_title_completer;
pub mod types;

// ── Agent-layer re-exports (the seam) ──────────────────────────────────────
// Backend crates reach the agent (nomi-*) layer ONLY through nomifun-ai-agent.
// When the agent layer is later extracted into its own repo, these re-exports
// become the single integration surface.
pub use nomi_agent::computer_history_tools::ComputerHistorySink;
pub use nomi_agent::companion_tools::CompanionMemorySink;
pub use nomi_agent::companion_tools::{CompanionSkillSink, SkillListing};
pub use nomi_agent::summon_tools::SummonContextSink;
pub use nomi_agent::cron_tools::{CronJobSummary, CronSink};
pub use nomi_agent::ssh_backend::{
    RemoteCommandOutput, RemoteFileStat, SshBackend, SshBackendProvider, SshLeaseRelease,
    SshSessionBinding, SshSessionLease,
};
pub use nomi_agent::requirement_tools::RequirementSink;
pub use nomi_config;
pub use nomi_types;

pub use runtime_state::AgentRuntimeState;
pub use boot_process_reaper::{
    AgentProcessReapReport, ConversationProcessReapVerdict, reap_orphan_agent_processes,
};
#[cfg(any(test, feature = "test-support"))]
pub use runtime_handle::MockAgentRuntime;
pub use runtime_handle::{
    AgentRuntimeControl, AgentRuntimeHandle, SystemResourceNoticeDelivery,
};
pub use factory::provider_config::{
    one_shot_completion, one_shot_completion_bounded, resolve_provider_config,
    streaming_completion, streaming_completion_text_or_reasoning, user_message, DeltaKind,
};
pub use one_shot::{OneShotDeps, OneShotTool, OneShotTurnRequest, one_shot_handler, run_one_shot_turn};
pub use factory::{
    AgentFactoryDeps, CompanionPromptProvider, CompanionSummonProvider,
    build_agent_factory, build_agent_model_config_resolver,
};
#[cfg(feature = "browser-use")]
pub use factory::browser_lane::{
    BrowserLaneBinding, BrowserLaneClientProvider, BrowserLaneClientProviderSlot,
    BrowserOwnerLeaseGuard, TrustedBrowserRuntimeContext,
};
#[cfg(feature = "browser-use")]
pub use browser_fetcher::BrowserFetcher;
pub use knowledge_completer::LiveKnowledgeCompleter;
pub use knowledge_completer::resolve_default_model;
pub use knowledge_retrieval::LiveKnowledgeRetrievalSink;
pub use knowledge_writeback::LiveKnowledgeWritebackSink;
pub use nomi_session_persistence::{
    NomiSessionPersistence, NomiSessionRecoveryRewindOutcome, NomiSessionResetOutcome,
};
pub use terminal_title_completer::LiveTerminalTitleCompleter;
pub use nomifun_api_types::{NomiBuildExtra, SlashCommandItem};
pub use protocol::events::{
    AgentStreamEvent, FinishEventData, PermissionEventData, TurnStopReason,
};
pub use protocol::send_error::AgentSendError;
pub use registry::{AgentRegistry, UnavailableReason};
pub use routes::{AgentRouterState, agent_routes};
pub use services::AgentService;
pub use runtime_registry::{
    AgentRuntimeModelConfigResolver, AgentRuntimeRegistry, InMemoryAgentRuntimeRegistry,
    RuntimeModelConfigBinding,
};
