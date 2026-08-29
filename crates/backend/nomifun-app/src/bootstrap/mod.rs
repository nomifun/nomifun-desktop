//! Process-level bootstrap helpers for the binary.
//!
//! These are *not* subcommands — they are layered initialization steps
//! (logging, work_dir resolution, builtin-skill materialization, database
//! init) that subcommands compose to start the application.

mod admin;
mod bind;
mod boot_log;
mod builtin_skills;
mod data_root;
mod environment;
mod relocation;
mod server_lock;
mod tracing_init;
mod v4_root;
mod webui_dist;
mod work_dir;

pub use admin::{AdminBootstrap, ensure_admin_credentials};
pub use bind::{PORT_FILE, PortAnnouncement, SCAN_SPAN, announce_bound_port, bind_with_fallback, write_port_file};
pub use boot_log::{BootNoteLevel, record_boot_note};
pub use data_root::{
    LAYOUT_MIGRATION_PENDING_MARKER, RELOCATED_DONE_MARKER,
    RELOCATED_FROM_MARKER, RelocationMarker, is_known_default_location,
    resolve_startup_data_root,
};
pub use environment::{
    ServerEnvironment, finalize_data_layer, init_data_layer, init_environment,
};
pub(crate) use environment::{
    acquire_distinct_work_root_lock, acquire_work_root_lock,
};
pub use server_lock::{BootServerLockAuthority, SERVER_LOCK_FILE, ServerLock};
pub use webui_dist::{
    UI_BUILD_MANIFEST_FILE, UI_BUILD_MANIFEST_SCHEMA, UiBuildManifest, ui_api_contract_version,
    validate_webui_dist, validate_webui_manifest_bytes,
};
pub(crate) use work_dir::resolve_work_dir;

/// Acquire the canonical data-directory lock for an offline maintenance
/// command. Kept separate from `init_environment` so backup can avoid logging,
/// factory-reset processing, and all other server boot side effects.
pub fn acquire_offline_server_lock(data_dir: &std::path::Path) -> anyhow::Result<ServerLock> {
    server_lock::acquire_server_lock(data_dir)
}
