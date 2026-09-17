//! AgentSession-owned Browser Resource contracts, input authority, and lifecycle.
//!
//! Managed resources share one live runtime with the Agent. Background search
//! and rendering have separate application owners; no legacy Hub/Lane facade
//! or lease issuer is part of this crate.

pub mod revision;
pub mod bound_resource;
pub mod downloads;
pub mod product;
pub mod run_guard;
pub mod runtime;
pub mod attached_browser;
pub mod uploads;
pub mod url_projection;
pub mod workspace;
