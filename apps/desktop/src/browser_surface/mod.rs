//! Native browser host integration. External browser pages never own app IPC.

pub(crate) mod security;
pub(crate) mod commands;
#[cfg(windows)]
pub(crate) mod host;
#[cfg(windows)]
pub(crate) mod windows;
#[cfg(any(windows, target_os = "macos"))]
pub(crate) mod automation;
#[cfg(windows)]
pub(crate) use windows as native;
#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(target_os = "macos")]
pub(crate) use macos::native;
