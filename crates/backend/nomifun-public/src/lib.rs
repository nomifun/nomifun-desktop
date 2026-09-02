//! Canonical installation-owner Remote transport.
//!
//! MCP is a thin adapter over `AgentPlatform` and the product
//! `AgentSession` aggregate. Transport session state is kept only for rmcp
//! lifecycle and admission; it is never a second product identity.

mod canonical;
mod result;
mod router;
mod session;

pub use canonical::{
    CANONICAL_REMOTE_CANCEL_TOOL, CANONICAL_REMOTE_OBSERVE_TOOL,
    CANONICAL_REMOTE_OPEN_TOOL, CANONICAL_REMOTE_TURN_TOOL,
    CanonicalRemoteMcpHandler, CanonicalRemoteRuntimeAdmission,
    canonical_remote_mcp_router,
};
pub use result::build_tool_result;
pub use router::{
    PublicMcpState, RemoteInstanceOwner, instance_token_middleware,
};
pub use session::RemoteMcpSessionAdmissionAuthority;
