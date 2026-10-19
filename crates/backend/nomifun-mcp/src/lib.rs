//! MCP server configuration, multi-agent sync adapters, OAuth, and connection testing.
pub mod adapter;
pub mod adapters;
pub mod connection_test;
pub mod error;
pub mod identity;
pub mod auth_service;
pub mod oauth_failure;
pub mod oauth_protocol;
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
pub use identity::{
    MCP_TOOL_CAPABILITY_PREFIX, MCP_TOOL_MATERIALIZATION_REVISION,
    RETIRED_MCP_AUTHORING_CAPABILITY_IDS, McpToolIdentityError,
    canonical_mcp_connection_config_ref, canonical_mcp_tool_action_id, canonical_mcp_tool_capability_id,
    is_namespaced_mcp_tool_capability, is_retired_mcp_authoring_capability,
};
pub use auth_service::{McpAuthOptions, McpAuthService};
pub use owner::{
    AnonymousMcpCredentialAuthority, McpCredential, McpCredentialAuthority, McpCredentialLookup,
    McpOwner, McpOwnerError, McpServerBinding, McpToolBinding, McpToolInvocationRequest,
    McpInvocationControl, McpInvocationOutcome, McpExecutionFacts, McpCleanupFacts,
    McpCleanupStatus, McpSetupPolicy,
    McpToolInvocationResult, MCP_CONNECT_OPERATION,
    McpResourceFailure, McpResourceOperation, McpResourceRequest, McpResourceResult,
    MCP_INVOKE_OPERATION, MCP_READ_OPERATION, MCP_SERVER_RESOURCE_KIND,
};
pub use routes::{McpRouterState, mcp_routes};
pub use service::McpConfigService;
pub use sync_service::McpSyncService;
pub use types::{McpServer, McpServerTransport, McpTool};
