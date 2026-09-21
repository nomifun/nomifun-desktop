//! The single official Agent Runtime, its lifecycle, and host-owned adapters.
pub(crate) mod runtime_state;
pub mod artifact_store;
pub mod boot_process_reaper;
mod process_registry;
pub mod runtime_handle;
pub mod runtime_instance;
pub mod runtime_provider;
pub mod runtime_admission;
pub mod runtime_driver;
pub mod unified_runtime;
pub mod context_contributor;
pub mod runtime_model_middleware_contract;
pub mod runtime_tool_middleware_contract;
pub mod companion_tools;
pub mod cron_tools;
pub mod host_skills;
pub mod knowledge_tools;
pub mod requirement_tools;
pub mod runtime_output;
pub mod session_control_tools;
pub mod ssh_backend;
pub mod engine_sdk;
mod engine_tasks;
pub mod engine_effect_scope;
pub mod model_attachments;
pub mod nomi_skills;
pub mod nomi_resources;
pub mod cc_switch;
pub mod factory;
pub mod image_generation;
pub mod knowledge_completer;
pub mod one_shot;
pub mod plugin_tools;
pub mod tool_discovery;
pub use plugin_tools::model_middleware;
pub use plugin_tools::tool_middleware;
pub mod plugin_skills;
mod plugin_tool_error_projection;
pub mod protocol;
pub mod registry;
pub mod routes;
pub(crate) mod services;
pub mod runtime_sessions;
pub mod terminal_title_completer;
pub mod types;
#[path = "web_search_provider.rs"]
pub mod web_search;
#[cfg(feature = "browser-use")]
pub mod local_web_search;
// Host/domain adapters extracted from the retired standalone Nomi loop. They
// carry no model loop, Session store, or alternate Runtime authority.
pub use companion_tools::{CompanionMemorySink, CompanionSkillSink, SkillListing};
pub use cron_tools::{CronJobSummary, CronSink};
pub use ssh_backend::{
    RemoteCommandOutput, RemoteFileStat, SshBackend, SshBackendProvider, SshLeaseRelease,
    SshSessionBinding, SshSessionLease,
};
pub use requirement_tools::RequirementSink;
pub use context_contributor::{ContextContributor, TurnContext};
pub use session_control_tools::{
    AGENT_EXECUTION_OBSERVE_TOOL_NAME, AGENT_EXECUTION_STEER_TOOL_NAME,
    AGENT_FORK_TOOL_NAME, AgentExecutionObserveTool, AgentExecutionSteerTool,
    AgentForkTool, SessionControlSink,
};
pub use nomi_config;
pub use nomi_types;

pub use runtime_state::AgentRuntimeState;
pub use runtime_instance::{OfficialAgentRuntime, RuntimeSteerDelivery, RuntimeTeardown};
pub use runtime_admission::{RuntimeAdmission, RuntimeSupport};
pub use runtime_driver::{
    HostedNomiRuntime, NomiRuntimeDriver, NomiRuntimeDriverFactory,
    NomiRuntimeTurnOutcome, NomiRuntimeTurnOutput, NomiRuntimeTurnTerminal,
    OFFICIAL_NOMI_RUNTIME_FAMILY_ID,
};
pub use runtime_provider::{
    OfficialRuntimeFactory, OfficialRuntimeProvider, RUNTIME_HOST_CONTRACT_VERSION,
    RuntimeBuildBinding, RuntimeBuildDescriptor,
};
pub use boot_process_reaper::{
    AgentProcessReapReport, ConversationProcessReapVerdict, reap_orphan_agent_processes,
};
#[cfg(any(test, feature = "test-support"))]
pub use runtime_handle::MockAgentRuntime;
pub use runtime_handle::{
    AgentCapabilityActivationSnapshot, AgentRuntimeControl, AgentRuntimeHandle,
    SystemResourceNoticeDelivery,
};
pub use factory::provider_config::{
    one_shot_completion, one_shot_completion_bounded, resolve_provider_config,
    resolve_provider_config_at_revision, streaming_completion,
    streaming_completion_text_or_reasoning, user_message, DeltaKind,
};
pub use one_shot::{OneShotDeps, OneShotTool, OneShotTurnRequest, one_shot_handler, run_one_shot_turn};
pub use plugin_tools::{
    supports_nomi_plugin_capability,
    KernelNomiPluginToolSession,
    NomiHostDynamicToolDescriptor, NomiHostDynamicToolError,
    NomiHostDynamicToolInvocation,
    NomiHostDynamicToolInvoker,
    NomiInitialContextContribution,
    NomiPlatformBuiltinContextAdmission, NomiPluginToolAction,
    NomiPluginToolError, NomiPluginToolInvocation, NomiPluginToolInvoker,
    NomiPluginToolSchemaResolver,
    NomiPlatformBuiltinToolAdmission,
    NomiPlatformBuiltinToolSchemaResolver,
    NomiPlatformBuiltinToolSchemaRouter,
    NomiPlatformBuiltinLifecycleAdmission,
    NomiPlatformBuiltinLifecycleInvocation,
    NomiPlatformBuiltinLifecycleInvoker,
    NomiHostedSessionBindings, NomiPluginToolSession, NomiPluginToolSessionProvider,
    NomiPluginToolSessionRequest, NomiPluginProductToolAction,
    NomiPluginProductToolInvocation, NomiPluginProductToolInvoker,
    NomiPluginProductToolSchemaResolver,
};
pub use factory::build_agent_model_config_resolver;
pub use knowledge_completer::LiveKnowledgeCompleter;
pub use knowledge_completer::resolve_default_model;
pub use terminal_title_completer::LiveTerminalTitleCompleter;
pub use nomifun_api_types::{NomiBuildExtra, SlashCommandItem};
pub use protocol::events::{AgentStreamEvent, FinishEventData, TurnStopReason};
pub use protocol::send_error::AgentSendError;
pub use registry::{AgentRegistry, UnavailableReason};
pub use routes::{AgentRouterState, agent_routes};
pub use services::AgentService;
pub use runtime_sessions::{
    RuntimeModelConfigResolver, AgentRuntimeSessions, InMemoryAgentRuntimeSessions,
    RuntimeModelConfigBinding,
};
