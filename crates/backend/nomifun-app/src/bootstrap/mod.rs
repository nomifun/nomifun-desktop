//! Process-level bootstrap helpers for the binary.
//!
//! These are *not* subcommands — they are layered initialization steps
//! (logging, work_dir resolution, builtin-skill materialization, database
//! init) that subcommands compose to start the application.

mod admin;
mod bind;
mod boot_log;
mod builtin_skills;
mod environment;
mod relocation;
mod server_lock;
mod tracing_init;
mod work_dir;

pub use admin::{AdminBootstrap, ensure_admin_credentials};
pub use bind::{PORT_FILE, PortAnnouncement, SCAN_SPAN, announce_bound_port, bind_with_fallback, write_port_file};
pub use boot_log::{BootNoteLevel, record_boot_note};
pub use environment::{ServerEnvironment, init_data_layer, init_environment};
pub use relocation::{RELOCATED_DONE_MARKER, RELOCATED_FROM_MARKER, RelocationMarker, rewrite_relocated_paths};
pub use server_lock::{SERVER_LOCK_FILE, ServerLock};
