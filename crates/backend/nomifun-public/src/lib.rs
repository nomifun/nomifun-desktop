//! Canonical installation-owner Remote transport.
//!
//! MCP delegates to host-injected operations, independent of any Engine or
//! legacy AgentPlatform. Transport session state is kept only for rmcp
//! lifecycle and admission; it is never a second product identity.

mod canonical;
mod result;
mod router;
mod session;

pub use canonical::{
    CANONICAL_REMOTE_CANCEL_TOOL, CANONICAL_REMOTE_OBSERVE_TOOL, CANONICAL_REMOTE_OPEN_TOOL,
    CANONICAL_REMOTE_TURN_TOOL, CanonicalRemoteMcpHandler, CanonicalRemoteOperationError,
    CanonicalRemoteOperationFuture, CanonicalRemoteOperations,
    canonical_remote_mcp_router_with_operations,
};
pub use result::build_tool_result;
pub use router::{PublicMcpState, RemoteInstanceOwner, instance_token_middleware};
pub use session::RemoteMcpSessionAdmissionAuthority;
