//! Public API for the bundled bun runtime.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::cache;
use crate::embed::{EmbeddedBun, ProductionEmbed};
use crate::extract::{self, ExtractError};

/// Max time to wait for a freshly-extracted `bun` binary to become
/// observable via `Path::is_file()` after `extract_into()` returns.
const BUN_OBSERVABLE_TIMEOUT: Duration = Duration::from_secs(2);
const BUN_OBSERVABLE_POLL: Duration = Duration::from_millis(100);

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("bun not found")]
    NotFound,
    #[error("failed to extract embedded bun: {0}")]
    Extract(#[from] std::io::Error),
    #[error("embedded bun checksum mismatch")]
    ChecksumMismatch,
    #[error("serde_json: {0}")]
    Json(#[from] serde_json::Error),
}

impl From<ExtractError> for ResolveError {
    fn from(err: ExtractError) -> Self {
        match err {
            ExtractError::Io(e) => ResolveError::Extract(e),
            ExtractError::ChecksumMismatch { .. } => ResolveError::ChecksumMismatch,
            ExtractError::Json(e) => ResolveError::Json(e),
        }
    }
}

static RESOLVED_BUN: OnceLock<PathBuf> = OnceLock::new();

/// Returns the path to a usable `bun` executable.
///
/// Priority: `NOMIFUN_BUN_PATH` env override > embedded + extract >
/// `which("bun")`.
pub fn resolve_bun() -> Result<PathBuf, ResolveError> {
    if let Some(path) = RESOLVED_BUN.get() {
        return Ok(path.clone());
    }
    let resolved = resolve_with(&ProductionEmbed, std::env::var("NOMIFUN_BUN_PATH").ok().as_deref())?;
    Ok(RESOLVED_BUN.get_or_init(|| resolved).clone())
}

/// Return the directory of the resolved Bun executable, including overrides
/// and PATH fallback. Failed lookups remain retryable, as in `resolve_bun`.
pub fn bun_bin_dir() -> Option<PathBuf> {
    resolve_bun().ok().and_then(|path| path.parent().map(PathBuf::from))
}

fn resolve_with<E: EmbeddedBun>(embed: &E, override_raw: Option<&str>) -> Result<PathBuf, ResolveError> {
    if let Some(p) = env_override(override_raw) {
        return Ok(p);
    }
    if !embed.has() {
        return nomi_process_runtime::resolve_command_path("bun").ok_or(ResolveError::NotFound);
    }
    let dir = cache::bun_dir(embed.version(), embed.sha256()).ok_or(ResolveError::NotFound)?;
    let bun_path = dir.join(extract::bun_filename());

    // Stamp and all required executables are present: fast path.
    if extract::is_fresh(&dir, embed.sha256(), embed.version()) {
        return Ok(bun_path);
    }

    // A mismatching immutable blob cannot be repaired by retrying it. Keep
    // failure cleanup inside extract_into's lock; never wipe the cache here.
    let extracted = extract::extract_into(&dir, embed.blob(), embed.sha256(), embed.version())?;

    // Guard against returning a phantom path: wait until the executable
    // is observable on disk. Without this, a caller that immediately
    // spawns the returned path can race with the OS file-cache flush and
    // see ENOENT, as seen on cold start right after first extract.
    wait_until_observable(&extracted)?;
    Ok(extracted)
}

fn wait_until_observable(path: &Path) -> Result<(), ResolveError> {
    let deadline = Instant::now() + BUN_OBSERVABLE_TIMEOUT;
    loop {
        if path.is_file() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            tracing::warn!(
                path = %path.display(),
                "extracted bun path not observable after timeout"
            );
            return Err(ResolveError::NotFound);
        }
        std::thread::sleep(BUN_OBSERVABLE_POLL);
    }
}

fn env_override(raw: Option<&str>) -> Option<PathBuf> {
    let trimmed = raw?.trim();
    if trimmed.is_empty() {
        return None;
    }
    let p = PathBuf::from(trimmed);
    if p.is_file() {
        Some(p)
    } else {
        tracing::warn!(path = %p.display(), "NOMIFUN_BUN_PATH does not point to a file; ignoring");
        None
    }
}

/// Resolve a command name to an absolute path.
///
/// For `bun` / `bunx` this crate resolves the bundled toolchain first;
/// everything else delegates to `nomi-process-runtime`'s platform-neutral
/// `PATH` resolver.
///
/// On Windows, if a bare name lookup fails we retry with the common
/// shim suffixes (`.cmd`, `.ps1`, `.bat`). Tools installed via npm
/// global / pnpm / yarn typically ship as `name.cmd`, and a user with a
/// trimmed `PATHEXT` would otherwise see them as missing.
pub fn resolve_command_path(cmd: &str) -> Option<PathBuf> {
    match cmd {
        "bun" => resolve_bun()
            .ok()
            .or_else(|| nomi_process_runtime::resolve_command_path("bun")),
        "bunx" => {
            let bunx_name = if cfg!(windows) { "bunx.exe" } else { "bunx" };
            if let Some(dir) = bun_bin_dir() {
                let p = dir.join(bunx_name);
                if p.exists() {
                    return Some(p);
                }
            }
            nomi_process_runtime::resolve_command_path("bunx")
        }
        other => nomi_process_runtime::resolve_command_path(other),
    }
}

/// Resolve `cmd` to an absolute path **within `dir` only** — does not walk
/// `PATH`. Honours `PATHEXT` (so `widget.exe` is found on Windows), and on
/// Windows additionally tries `.cmd`, `.ps1`, `.bat` shim suffixes for
/// npm-/pnpm-installed CLIs whose extension `PATHEXT` may not list.
///
/// The shared process resolver treats `dir` as one exact PATH entry, so a
/// directory containing the OS separator (`:` on Unix, `;` on Windows)
/// cannot be misinterpreted as multiple directories.
///
/// Returns `None` if the command cannot be resolved inside the directory.
pub fn resolve_command_in(cmd: &str, dir: &Path) -> Option<PathBuf> {
    nomi_process_runtime::resolve_command_in(cmd, dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::FakeEmbed;

    #[test]
    fn bun_directory_recovers_after_initial_resolution_failure() {
        // Compile-time embeds cannot exercise the no-runtime startup case.
        if ProductionEmbed.has() {
            return;
        }
        if let Some(fixture) = std::env::var_os("NOMIFUN_RUNTIME_CACHE_TEST") {
            let fixture = PathBuf::from(fixture);
            assert_eq!(bun_bin_dir(), None);
            assert!(matches!(resolve_bun(), Err(ResolveError::NotFound)));
            let binary = fixture.join(extract::bun_filename());
            std::fs::write(&binary, b"fixture only: never executed").unwrap();
            assert_eq!(resolve_bun().unwrap(), binary);
            assert_eq!(bun_bin_dir(), Some(fixture));
            return;
        }
        // Isolate process-wide caches and environment from parallel tests.
        // The child only resolves an artificial file; it never launches Bun.
        let tmp = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "resolver::tests::bun_directory_recovers_after_initial_resolution_failure"])
            .current_dir(tmp.path())
            .env("NOMIFUN_RUNTIME_CACHE_TEST", tmp.path())
            .env("NOMIFUN_BUN_PATH", tmp.path().join(extract::bun_filename()))
            .env("PATH", tmp.path())
            .output().unwrap();
        assert!(output.status.success(), "{}\n{}",
            String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }

    #[test]
    fn env_override_wins_over_embed() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let fake = FakeEmbed {
            has: true,
            blob: b"invalid zstd: override must bypass extraction",
            sha256: "unused",
            version: "1.0",
        };

        let raw = format!("  {}  ", path.display());
        let result = resolve_with(&fake, Some(&raw)).unwrap();
        assert_eq!(result, path);
    }

    #[test]
    fn wait_until_observable_returns_immediately_when_present() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        // File exists, so this must be cheap.
        let start = Instant::now();
        wait_until_observable(tmp.path()).unwrap();
        assert!(start.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn wait_until_observable_errors_when_path_never_appears() {
        let tmp = tempfile::TempDir::new().unwrap();
        let phantom = tmp.path().join("does-not-exist");
        let res = wait_until_observable(&phantom);
        match res {
            Err(ResolveError::NotFound) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn env_override_ignores_absent_blank_missing_and_directory_values() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("missing");
        assert_eq!(env_override(None), None);
        assert_eq!(env_override(Some(" \t\n")), None);
        assert_eq!(env_override(missing.to_str()), None);
        assert_eq!(env_override(tmp.path().to_str()), None);
    }

    #[cfg(unix)]
    #[test]
    fn resolve_command_in_finds_executable_in_dir() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::TempDir::new().unwrap();
        let bin = tmp.path().join("widget");
        std::fs::write(&bin, b"#!/bin/sh\necho hi\n").unwrap();
        let mut perms = std::fs::metadata(&bin).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin, perms).unwrap();

        let found = resolve_command_in("widget", tmp.path()).expect("must find");
        assert_eq!(found, bin);
    }

    #[test]
    fn resolve_command_in_returns_none_for_missing_command() {
        let tmp = tempfile::TempDir::new().unwrap();
        let found = resolve_command_in("definitely-not-here", tmp.path());
        assert!(found.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn resolve_command_in_handles_dir_with_colon_safely() {
        // A path containing `:` is a separator-collision hazard for the
        // PATH string `which_in` consumes. We must NOT internally split
        // and search a wrong second segment — return None instead.
        let tmp = tempfile::TempDir::new().unwrap();
        let weird = tmp.path().join("with:colon");
        std::fs::create_dir(&weird).unwrap();
        // No `widget` file is created anywhere — the only way this could
        // return Some is if the function wrongly split `with:colon` and
        // found something in another segment.
        let found = resolve_command_in("widget", &weird);
        assert!(found.is_none(), "must not split on `:` inside dir; got {:?}", found);
    }

    #[cfg(windows)]
    #[test]
    fn resolve_command_in_falls_back_to_cmd_shim_on_windows() {
        // Simulate an npm-installed CLI: only `widget.cmd` exists, not `widget.exe`.
        let tmp = tempfile::TempDir::new().unwrap();
        let shim = tmp.path().join("widget.cmd");
        std::fs::write(&shim, b"@echo off\r\necho hi\r\n").unwrap();

        let found = resolve_command_in("widget", tmp.path()).expect("must find shim");
        assert!(
            found.to_string_lossy().to_lowercase().ends_with("widget.cmd"),
            "expected the .cmd shim; got {}",
            found.display()
        );
    }
}
