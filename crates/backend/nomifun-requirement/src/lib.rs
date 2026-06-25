//! Requirements Platform: CRUD store + AutoWork orchestrator for "requirements".
pub mod attachments;
mod convert;
pub mod events;
pub mod hooks;
pub mod mcp_server;
pub mod notifier;
pub mod orchestrator;
pub mod order_key;
pub mod prompt;
pub mod routes;
pub mod service;
pub mod sink;
pub mod state;

pub use attachments::{AttachmentStore, PromptAttachment};
pub use events::RequirementEventEmitter;
pub use hooks::IdmmHandle;
pub use mcp_server::RequirementMcpServer;
pub use notifier::CompletionNotifier;
pub use orchestrator::{Orchestrator, OrchestratorDeps};
pub use routes::requirement_routes;
pub use service::RequirementService;
pub use sink::RequirementServiceSink;
pub use state::RequirementRouterState;
