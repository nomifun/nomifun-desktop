//! `nomifun-public` — the installation-owner **Remote front door**.
//!
//! Projects the platform's single capability source of truth
//! (`nomifun_gateway::Registry`) onto network-reachable MCP and REST endpoints
//! authenticated by one installation token. A validated token authenticates
//! the NomiFun Desktop owner and never selects or impersonates a companion.
//!
//! This crate is deliberately thin: it owns transport + auth + identity only.
//! Every capability, its schema, its danger tier and its surface gate already
//! live in `nomifun-gateway`; adding a capability there makes it appear here
//! automatically (the inheritance guarantee — see the design spec §2.1). It MUST
//! be mounted in-process by `nomifun-app` (the `server.lock` data-dir is
//! single-writer; a sidecar is impossible).

mod handler;
mod idempotency;
mod rest;
mod result;
mod router;
mod session;

pub use handler::RemoteMcpHandler;
pub use rest::public_rest_router;
pub use result::build_tool_result;
pub use router::{
    PublicMcpState, public_mcp_router, public_mcp_router_with_admission,
};
pub use session::RemoteMcpSessionAdmissionAuthority;

/// Curated "agent" profile for the Remote surface: the do-work capability
/// domains an external task-delegation agent typically needs, excluding
/// platform-management domains (channel/companion/cron/system/team/…). Keeps a
/// remote MCP client's tool list tight (better tool-selection) without changing
/// permissions — dispatch is still gated by the Remote surface, not the profile.
/// (`computer` lights up when the computer-use caps land.)
pub const AGENT_PROFILE_DOMAINS: &[&str] =
    &[
        "agent_execution",
        "agent",
        "remote",
        "conversation",
        "browser",
        "computer",
        "knowledge",
        "files",
        "memory",
    ];
