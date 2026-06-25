//! IDMM (Intelligent Decision-Making Mode): per-session supervision that keeps
//! agent/terminal sessions alive through provider faults and decision stalls.
//! Rule tier (no LLM) + sidecar backup-model tier, stacking on AutoWork.
//!
//! Layering: `signal`/`config`/`detector`/`prompt`/`util` are pure; `probe`
//! abstracts the target; `sidecar` calls the backup model; `policy` is the
//! escalation ladder; `supervisor` runs the per-session loop + `IdmmManager`
//! (which implements `nomifun_requirement::IdmmHandle`); `service`/`state`/
//! `routes` are the domain API surface.

pub mod config;
pub mod detector;
pub mod events;
pub mod policy;
pub mod probe;
pub mod prompt;
pub mod routes;
pub mod service;
pub mod sidecar;
pub mod signal;
pub mod state;
pub mod supervisor;
pub mod util;

pub use events::IdmmEventEmitter;
pub use routes::idmm_routes;
pub use service::{IdmmService, ProbeDeps};
pub use sidecar::{Completer, LiveCompleter, SidecarClient};
pub use state::IdmmRouterState;
pub use supervisor::{IdmmManager, LoopDeps};
