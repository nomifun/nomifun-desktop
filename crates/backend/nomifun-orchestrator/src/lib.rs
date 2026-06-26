//! 智能编排 (orchestration) services.
//!
//! - [`FleetService`] — CRUD over per-user fleets (编队) and their members,
//!   handling Row↔DTO mapping and JSON (de)serialization of the per-member
//!   `capability_profile` / `constraints` (fail-soft on decode).
//! - [`OrchestratorError`] — service-layer error mapped into `AppError`.
//! - [`OrchestratorRouterState`] — minimal router state shell (extended in
//!   Task 6, consumed by routes in Task 7).
//!
//! Routes (`orchestrator_routes`) are intentionally NOT exported yet — they
//! arrive in Task 7.

pub mod error;
pub mod service;
pub mod state;

pub use error::OrchestratorError;
pub use service::FleetService;
pub use state::OrchestratorRouterState;
