//! User-authored assistant management.
//!
//! Owns the `assistants` and `assistant_overrides` tables, built-in
//! assistant loading from on-disk manifest, and merge logic for
//! `GET /api/assistants` across builtin + user + extension sources.

pub mod builtin;
pub mod routes;
pub mod service;
pub mod state;

pub use builtin::{AvatarAsset, BuiltinAssistant, BuiltinAssistantRegistry};
pub use routes::{AssistantRouterState, assistant_routes};
pub use service::AssistantService;
