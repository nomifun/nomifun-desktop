pub mod capability;
pub mod fallback_backend;
pub mod input;
pub mod keys;
pub mod launch;
#[cfg(target_os = "linux")]
pub mod linux_x11;
#[cfg(target_os = "linux")]
mod linux_native_worker;
#[cfg(target_os = "linux")]
pub mod linux_portal;
#[cfg(target_os = "linux")]
pub mod linux_wayland;
#[cfg(target_os = "linux")]
pub mod linux_wayland_input;
#[cfg(target_os = "linux")]
pub mod linux_pipewire;
mod macos_main;
mod native_control;
pub mod permissions;
pub mod scale;
pub mod screen;
pub mod tool;

pub use tool::ComputerTool;

// Re-exported so a host that pulls computer-use (but not nomi-a11y directly)
// can brand the permission-error guidance with its own app name — see
// `nomi_a11y::set_host_app_label`.
pub use nomi_a11y::{host_app_label, set_host_app_label};
