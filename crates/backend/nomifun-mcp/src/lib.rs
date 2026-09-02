//! MCP server configuration, multi-agent sync adapters, OAuth, and connection testing.
pub mod adapter;
pub mod adapters;
pub mod connection_test;
pub mod error;
pub mod oauth_service;
pub mod owner;
pub mod routes;
pub mod service;
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
pub use owner::{
    AnonymousMcpCredentialAuthority, McpCredential, McpCredentialAuthority, McpCredentialLookup,
    McpOwner, McpOwnerError, McpServerBinding, McpToolBinding, McpToolInvocationRequest,
    McpToolInvocationResult, OAuthMcpCredentialAuthority, MCP_CONNECT_OPERATION,
    MCP_INVOKE_OPERATION, MCP_READ_OPERATION, MCP_SERVER_RESOURCE_KIND,
};
pub use routes::{McpRouterState, mcp_routes};
pub use service::McpConfigService;
pub use sync_service::McpSyncService;
pub use types::{McpServer, McpServerTransport, McpTool};
