//! MCP server configuration, multi-agent sync adapters, OAuth, and connection testing.
pub mod adapter;
pub mod adapters;
pub mod connection_test;
pub mod error;
pub mod oauth_service;
pub mod routes;
pub mod service;
pub mod session_injection;
pub mod sync_service;
pub mod types;

pub use adapter::{DetectedServer, McpAgentAdapter};
pub use adapters::{
    ClaudeAdapter, CodeBuddyAdapter, CodexAdapter, GeminiAdapter, NomiAdapter, NomifunAdapter, OpencodeAdapter,
    QwenAdapter,
};
pub use connection_test::McpConnectionTestService;
pub use error::McpError;
pub use oauth_service::McpOAuthService;
pub use routes::{McpRouterState, mcp_routes};
pub use service::McpConfigService;
pub use session_injection::{
    AcpMcpCapabilities, AcpSessionMcpServer, ImageGenConfig, NameValuePair, build_builtin_image_gen_server,
    build_session_mcp_servers, parse_acp_mcp_capabilities,
};
pub use sync_service::McpSyncService;
pub use types::{McpServer, McpServerTransport, McpTool};
