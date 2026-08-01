//! Knowledge-base registration for external agent CLIs.
//!
//! Backs the owner-local HTTP endpoints in [`super::health`]
//! (`/api/terminals/register-knowledge*` and the MCP register template);
//! there is no corresponding `nomicore` subcommand.

pub(crate) mod mcp_register_template;
pub(crate) mod register_knowledge;
pub(crate) mod register_knowledge_global;
