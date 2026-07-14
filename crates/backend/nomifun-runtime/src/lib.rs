//! Bundled runtime (bun) resolver for nomicore.
//!
//! Embeds the bun runtime at build time (zstd-compressed) and extracts it
//! to the user's OS cache directory on first call. Callers use
//! [`resolve_bun`] to obtain a usable executable path and [`bun_bin_dir`]
//! to prepend the runtime directory to child-process `PATH`.

mod cache;
mod embed;
mod extract;
mod resolver;
mod shell_env;

pub use cache::{init, runtime_root};
pub use resolver::{ResolveError, bun_bin_dir, resolve_bun, resolve_command_in, resolve_command_path};
pub use shell_env::enhance_process_path;

#[cfg(test)]
#[path = "../build_support.rs"]
mod build_support_tests;
