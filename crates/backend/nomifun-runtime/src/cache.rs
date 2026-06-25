//! Cross-platform cache directory resolution for the bundled bun runtime.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Override for [`runtime_root`], set by [`init`] from the backend
/// startup path so cached bun binaries land under `AppConfig.data_dir`
/// instead of the OS-default cache location.
///
/// Lifecycle: written once by `nomifun-app`'s `main()` before
/// [`crate::enhance_process_path`] / [`crate::resolve_bun`] run, read
/// every time [`runtime_root`] is queried thereafter. Callers that miss
/// the init window (e.g. the `mcp-*` subcommands, unit tests,
/// `build.rs`) transparently fall back to `dirs::cache_dir()`.
static RUNTIME_ROOT_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Anchor the runtime root to a caller-supplied data directory — typically
/// the backend's `AppConfig.data_dir`. Idempotent on repeat calls (only
/// the first value wins); a warning is logged if a second path is
/// attempted so unexpected double-inits are visible.
pub fn init(data_dir: impl AsRef<Path>) {
    let path = data_dir.as_ref().join("runtime");
    if let Err(existing) = RUNTIME_ROOT_OVERRIDE.set(path.clone())
        && existing != path
    {
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

/// Per-version cache directory name: `bun-<version>-<sha12>`.
///
/// `sha12` is the first 12 hex chars of the bun binary sha256 — embedding
/// it means version bumps and content-level bumps both produce a new dir
/// so stale bytes never shadow a new build.
pub fn bun_dir_name(version: &str, sha256: &str) -> String {
    let sha12 = &sha256[..12.min(sha256.len())];
    format!("bun-{version}-{sha12}")
}

/// Full path for a specific (version, sha) cache directory.
pub fn bun_dir(version: &str, sha256: &str) -> Option<PathBuf> {
    runtime_root().map(|root| root.join(bun_dir_name(version, sha256)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bun_dir_name_format() {
        assert_eq!(bun_dir_name("1.1.38", "abc1234567890def"), "bun-1.1.38-abc123456789");
    }

    #[test]
    fn bun_dir_name_short_sha_does_not_panic() {
        // Defensive: if upstream ever passes <12 chars, don't panic.
        assert_eq!(bun_dir_name("1.0", "abc"), "bun-1.0-abc");
    }

    #[test]
    fn runtime_root_ends_with_expected_suffix() {
        let root = runtime_root().expect("cache dir available in test env");
        let tail: Vec<_> = root
            .components()
            .rev()
            .take(2)
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        assert_eq!(tail, vec!["runtime".to_string(), "nomifun".to_string()]);
    }

    #[test]
    fn bun_dir_embeds_version_and_sha() {
        let dir = bun_dir("1.1.38", "deadbeefcafebabe").expect("cache available");
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name, "bun-1.1.38-deadbeefcafe");
    }
}
