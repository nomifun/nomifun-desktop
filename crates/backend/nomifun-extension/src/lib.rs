//! Extension registry: manifest parsing, hub installation, and lifecycle hooks.

mod asset_paths;
pub mod constants;
pub mod dependency;
pub mod error;
pub mod hub;
pub mod hub_routes;
pub mod lifecycle;
pub mod loader;
pub mod manifest;
pub mod permission;
pub mod registry;
mod registry_helpers;
pub mod resolvers;
pub mod routes;
pub mod state;
pub mod template;
pub mod types;

pub use constants::*;
pub use dependency::{DependencyIssue, DependencyValidationResult, validate_dependencies};
pub use error::ExtensionError;
pub use lifecycle::{HookKind, execute_hook, needs_install_hook, resolve_hook_path};
pub use loader::{
    ScanPath, filter_by_engine_compatibility, load_all, resolve_install_target_dir_for_data_dir, resolve_scan_paths,
    resolve_scan_paths_for_data_dir,
};
pub use manifest::{parse_manifest, validate_manifest};
pub use permission::{build_permission_summary, calculate_risk_level};
pub use registry::{ExtensionRegistry, ExtensionSummary};
pub use resolvers::{resolve_all_contributions, resolve_extension_contributions, resolve_i18n_for_all};
pub use state::{ExtensionStateStore, load_states_from_file, resolve_state_file_path, save_states_to_file};
pub use template::{resolve_env_map, resolve_env_templates, resolve_file_reference};
pub use types::*;

pub use hub::{HubIndexManager, HubInstaller};
pub use hub_routes::{HubRouterState, hub_routes};
pub use routes::{ExtensionRouterState, extension_routes};
