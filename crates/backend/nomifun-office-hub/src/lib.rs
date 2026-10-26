//! Instance-owned manual office integration. Agent facts remain in AgentSession.
pub mod error;
pub mod types;
pub mod runtime;
pub mod adapter;
pub mod store;
pub mod service;
mod auth;
pub mod routes;
pub use routes::office_hub_routes;
pub use service::OfficeHubService;
pub use error::HubError;
pub use types::*;
