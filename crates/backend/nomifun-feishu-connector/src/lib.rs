//! Instance-owned Feishu connector integration. Agent facts remain in AgentSession.
pub mod error;
pub mod types;
pub mod runtime;
pub mod adapter;
pub mod store;
pub mod service;
mod auth;
pub mod routes;
pub use routes::feishu_connector_routes;
pub use service::FeishuConnectorService;
pub use error::FeishuConnectorError;
pub use types::*;
