//! Native browser host integration. External browser pages never own app IPC.

pub(crate) mod security;
pub(crate) mod commands;
#[cfg(windows)]
pub(crate) mod host;
#[cfg(windows)]
pub(crate) mod windows;
#[cfg(windows)]
pub(crate) mod automation;
