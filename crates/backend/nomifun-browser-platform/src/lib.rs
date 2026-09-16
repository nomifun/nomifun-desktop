//! Conversation-owned native browser contracts, input authority, and lifecycle.
//!
//! Native workspaces share one live runtime with the Agent. Background search
//! and rendering have separate application owners; no legacy Hub/Lane facade
//! or lease issuer is part of this crate.

pub mod revision;
pub mod downloads;
pub mod run_guard;
pub mod runtime;
pub mod system_browser;
pub mod uploads;
pub mod url_projection;
pub mod workspace;
