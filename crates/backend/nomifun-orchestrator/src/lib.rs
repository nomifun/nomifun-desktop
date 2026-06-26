//! 智能编排 (orchestration) services.
//!
//! - [`FleetService`] — CRUD over per-user fleets (编队) and their members,
//!   handling Row↔DTO mapping and JSON (de)serialization of the per-member
//!   `capability_profile` / `constraints` (fail-soft on decode).
//! - [`WorkspaceService`] — CRUD over per-user orchestration workspaces (Row↔DTO
//!   mapping; the DTO omits the internal `user_id` / `context` columns).
//! - [`OrchestratorError`] — service-layer error mapped into `AppError`.
//! - [`OrchestratorRouterState`] — router state shell (`fleet` + `workspace`),
//!   consumed by routes in Task 7.
//!
//! Routes (`orchestrator_routes`) are intentionally NOT exported yet — they
//! arrive in Task 7.

pub mod error;
pub mod service;
pub mod state;

pub use error::OrchestratorError;
pub use service::{FleetService, WorkspaceService};
pub use state::OrchestratorRouterState;
