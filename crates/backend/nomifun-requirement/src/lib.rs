//! Requirements Platform: CRUD store + AutoWork runner for "requirements".
pub mod auto_work_runner;
mod autowork_config;
pub mod attachments;
mod conversation_port;
mod convert;
pub mod events;
pub mod hooks;
pub mod mcp_server;
pub mod notifier;
pub mod order_key;
pub mod prompt;
pub mod routes;
pub mod service;
pub mod sink;
pub mod state;

pub use attachments::{AttachmentStore, PromptAttachment};
pub use autowork_config::{
    AutoWorkConfig, AutoWorkConfigSnapshot, AutoWorkSessionConfigCommand,
};
pub use conversation_port::{
    AutoWorkBindingIssue, AutoWorkBindingLookup, AutoWorkConversationPort, AutoWorkMessage,
    AutoWorkMessageDelivery, AutoWorkPreSendHook, AutoWorkReconciliationDisposition,
    AutoWorkRuntimeBuildLease, AutoWorkRuntimeLeaseIssuer, AutoWorkRuntimeOverlay,
    AutoWorkScheduledSessionLookup, AutoWorkSessionPort, AutoWorkSessionPreparation,
    AutoWorkSessionSnapshotToken, AutoWorkTurnDeliveryState, AutoWorkTurnRequest,
    PersistedAutoWorkBinding, ScheduledAutoWorkSession, ScheduledAutoWorkSessionScan,
};
pub use events::RequirementEventEmitter;
pub use hooks::IdmmHandle;
pub use mcp_server::RequirementMcpServer;
pub use notifier::CompletionNotifier;
pub use auto_work_runner::{
    AutoWorkRunner, AutoWorkRunnerDeps, AutoWorkStartOutcome, AutoWorkStopOutcome,
};
pub use routes::requirement_routes;
pub use service::RequirementService;
pub use sink::RequirementServiceSink;
pub use state::RequirementRouterState;
