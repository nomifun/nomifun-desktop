//! Team Guide module — capability descriptor, lead-facing tool arg parsing,
//! `nomi_*` MCP tool handlers, and Guide MCP server.
//!
//! The Guide MCP server is injected into single-chat agents to expose
//! `nomi_create_team` / `nomi_list_models` tools. Independent from the
//! per-team `TeamMcpServer`.
//!
//! Current tool set:
//! - `nomi_create_team` — build a new team from a natural-language summary
//! - `nomi_list_models` — enumerate backend × model options

pub mod capability;
pub mod handlers;
pub mod server;

pub use handlers::{CreateTeamParams, handle_nomi_list_models, parse_create_team_args};
pub use server::GuideMcpServer;
