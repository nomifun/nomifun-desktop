//! Cross-platform cache directory resolution for the managed tool suite.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Override for [`runtime_root`], set by [`init`] from the backend
/// startup path so cached tool binaries land under `AppConfig.data_dir`
/// instead of the OS-default cache location.
///
/// Lifecycle: written once by `nomifun-app`'s `main()` before
/// [`crate::resolve_bun`] runs, read
/// every time [`runtime_root`] is queried thereafter. Callers that miss
/// the init window (e.g. the `mcp-*` subcommands, unit tests,
/// `build.rs`) transparently fall back to `dirs::cache_dir()`.
/// Startup PATH enhancement runs earlier, before storage coordination can
/// create runtime/SQLx threads; its cache-only fallback never admits a tool.
static RUNTIME_ROOT_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Anchor the runtime root to a caller-supplied data directory — typically
/// the backend's `AppConfig.data_dir`. Idempotent on repeat calls (only
/// the first value wins); a warning is logged if a second path is
/// attempted so unexpected double-inits are visible.
pub fn init(data_dir: impl AsRef<Path>) {
    let path = data_dir.as_ref().join("runtime");
    let existing = RUNTIME_ROOT_OVERRIDE.get_or_init(|| path.clone());
    if existing != &path {
        tracing::warn!(
            attempted = %path.display(),
            existing = %existing.display(),
            "nomifun_runtime::init called twice with different paths; keeping first"
        );
    }
}

/// Returns the root cache directory used for all nomifun runtime artifacts.
///
/// Priority:
/// 1. Path supplied via [`init`] (`{data_dir}/runtime`) when the backend
///    started with `--data-dir`.
/// 2. Platform cache dir (via `dirs::cache_dir()`):
///    - macOS:   `~/Library/Caches/nomifun/runtime`
///    - Linux:   `$XDG_CACHE_HOME/nomifun/runtime` (fallback `~/.cache/nomifun/runtime`)
///    - Windows: `%LOCALAPPDATA%\nomifun\runtime`
///
/// Returns `None` only when neither [`init`] has run nor a platform cache
/// dir is determinable (exotic envs).
pub fn runtime_root() -> Option<PathBuf> {
    if let Some(p) = RUNTIME_ROOT_OVERRIDE.get() {
        return Some(p.clone());
    }
    dirs::cache_dir().map(|d| d.join("nomifun").join("runtime"))
}
