//! HTTP routes for the ai-agent crate, grouped by capability.
//!
//! - [`agent`] — agent-registry endpoints (`/api/agents*`, including
//!   custom-agent CRUD and the ACP health-check probe).
//! - [`remote`] — remote-agent pairing endpoints (`/api/remote-agents/*`).
//!
//! Session-scoped endpoints (mode / model / config / usage /
//! agent-capabilities / slash-commands / side-question / workspace /
//! openclaw-runtime) now live in the `nomifun-conversation` crate, where
//! they dispatch through `AgentInstance` via `ConversationService`.

pub mod agent;
pub mod remote;
pub mod state;

pub use agent::agent_routes;
pub use remote::remote_agent_routes;
pub use state::AgentRouterState;
pub use state::RemoteAgentRouterState;
