//! Reusable preset catalog, persistence facade, and execution resolver.

pub mod builtin;
pub mod routes;
pub mod service;
pub mod state;

pub use builtin::{AvatarAsset, BuiltinPreset, BuiltinPresetRegistry};
pub use routes::{PresetRouterState, preset_routes};
pub use service::PresetService;
