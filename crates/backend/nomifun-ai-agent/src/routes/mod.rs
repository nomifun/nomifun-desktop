//! HTTP routes for the ai-agent crate, grouped by capability.
//!
//! - [`agent`] — agent-registry endpoints (`/api/agents*`, including
//!   custom-agent CRUD and the ACP health-check probe).
//!
//! Session-scoped endpoints (mode / model / config / usage /
//! agent-capabilities / slash-commands / side-question / workspace /
//! openclaw-runtime) now live in the `nomifun-conversation` crate, where
//! they dispatch through `AgentRuntimeHandle` via `ConversationService`.

pub mod agent;
pub mod state;

pub use agent::agent_routes;
pub use state::AgentRouterState;
