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
static BUN_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Returns the path to a usable `bun` executable.
///
/// Priority: `NOMIFUN_BUN_PATH` env override > embedded + extract >
/// `which("bun")`.
pub fn resolve_bun() -> Result<PathBuf, ResolveError> {
    if let Some(path) = RESOLVED_BUN.get() {
        return Ok(path.clone());
    }
    let resolved = resolve_with(&ProductionEmbed)?;
    let _ = RESOLVED_BUN.set(resolved.clone());
    Ok(resolved)
}

/// Returns the directory that holds `bun` and `bunx`, if a bundled
/// runtime was extracted. `None` when no embed + no override was used.
pub fn bun_bin_dir() -> Option<PathBuf> {
    BUN_DIR
        .get_or_init(|| {
            resolve_with(&ProductionEmbed)
                .ok()
                .and_then(|p| p.parent().map(PathBuf::from))
        })
        .clone()
}

fn resolve_with<E: EmbeddedBun>(embed: &E) -> Result<PathBuf, ResolveError> {
    if let Some(p) = env_override() {
        return Ok(p);
    }
    if !embed.has() {
        return which::which("bun").map_err(|_| ResolveError::NotFound);
    }
    let dir = cache::bun_dir(embed.version(), embed.sha256()).ok_or(ResolveError::NotFound)?;
    let bun_path = dir.join(extract::bun_filename());

    // Stamp says fresh AND the executable is actually on disk: fast path.
    if extract::is_fresh(&dir, embed.sha256(), embed.version()) && bun_path.is_file() {
        return Ok(bun_path);
    }

    // One retry on checksum mismatch: wipe dir and re-extract.
    let extracted = match extract::extract_into(&dir, embed.blob(), embed.sha256(), embed.version()) {
        Ok(p) => p,
        Err(ExtractError::ChecksumMismatch { .. }) => {
            tracing::warn!("bun cache checksum mismatch; wiping and retrying");
            let _ = std::fs::remove_dir_all(&dir);
            extract::extract_into(&dir, embed.blob(), embed.sha256(), embed.version())?
        }
        Err(e) => return Err(e.into()),
    };

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

fn env_override() -> Option<PathBuf> {
    let raw = std::env::var("NOMIFUN_BUN_PATH").ok()?;
    let trimmed = raw.trim();
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
/// For `bun` / `bunx` we go through `nomifun_runtime` so the bundled
/// runtime is used when present; everything else falls back to the
/// user's `$PATH` via `which::which`.
///
/// On Windows, if a bare name lookup fails we retry with the common
/// shim suffixes (`.cmd`, `.ps1`, `.bat`). Tools installed via npm
/// global / pnpm / yarn typically ship as `name.cmd`, and a user with a
/// trimmed `PATHEXT` would otherwise see them as missing.
pub fn resolve_command_path(cmd: &str) -> Option<PathBuf> {
    match cmd {
        "bun" => resolve_bun().ok().or_else(|| which::which("bun").ok()),
        "bunx" => {
            let bunx_name = if cfg!(windows) { "bunx.exe" } else { "bunx" };
            if let Some(dir) = bun_bin_dir() {
                let p = dir.join(bunx_name);
                if p.exists() {
                    return Some(p);
                }
            }
            which::which("bunx").ok()
        }
        other => which::which(other).ok().or_else(|| windows_shim_fallback(other)),
    }
}

#[cfg(windows)]
fn windows_shim_fallback(cmd: &str) -> Option<PathBuf> {
    // If the caller already passed an extension, no point retrying.
    if Path::new(cmd).extension().is_some() {
        return None;
    }
    for ext in ["cmd", "ps1", "bat"] {
        if let Ok(p) = which::which(format!("{cmd}.{ext}")) {
            return Some(p);
        }
    }
    None
}

#[cfg(not(windows))]
fn windows_shim_fallback(_cmd: &str) -> Option<PathBuf> {
    None
}

/// Resolve `cmd` to an absolute path **within `dir` only** — does not walk
/// `PATH`. Honours `PATHEXT` (so `widget.exe` is found on Windows), and on
/// Windows additionally tries `.cmd`, `.ps1`, `.bat` shim suffixes for
/// npm-/pnpm-installed CLIs whose extension `PATHEXT` may not list.
///
/// `dir` is wrapped via `std::env::join_paths` before being handed to
/// `which::which_in`, so a `dir` that itself contains the OS PATH
/// separator (`:` on Unix, `;` on Windows) cannot be misinterpreted as
/// two directories. If `dir` cannot be expressed as a single PATH
/// entry, we return `None` rather than searching a phantom location.
///
/// Returns `None` if the command cannot be resolved inside the directory.
pub fn resolve_command_in(cmd: &str, dir: &Path) -> Option<PathBuf> {
    let paths = std::env::join_paths([dir]).ok()?;
    if let Ok(p) = which::which_in(cmd, Some(&paths), dir) {
        return Some(p);
    }
    windows_shim_fallback_in(cmd, dir)
}

/// Try `cmd` plus the common Windows shim suffixes (`.cmd`, `.ps1`, `.bat`)
/// inside a single directory. Used by `resolve_command_in` for callers that
/// want a directory-scoped lookup (the global `windows_shim_fallback` below
/// goes through `which::which`, which walks the entire `PATH`).
#[cfg(windows)]
fn windows_shim_fallback_in(cmd: &str, dir: &Path) -> Option<PathBuf> {
    if Path::new(cmd).extension().is_some() {
        return None;
    }
    for ext in ["cmd", "ps1", "bat"] {
        let candidate = dir.join(format!("{cmd}.{ext}"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(not(windows))]
fn windows_shim_fallback_in(_cmd: &str, _dir: &Path) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::FakeEmbed;
    use std::io::Write as _;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn make_blob(payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = zstd::stream::write::Encoder::new(&mut out, 0).unwrap();
        enc.write_all(payload).unwrap();
        enc.finish().unwrap();
        out
    }

    fn sha(payload: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(payload);
        hex::encode(h.finalize())
    }

    #[test]
    fn no_embed_falls_back_to_which() {
        let _guard = ENV_LOCK.lock().unwrap();
        // Safety: unset to avoid env override winning.
        // SAFETY: ENV_LOCK serializes tests that mutate NOMIFUN_BUN_PATH.
        unsafe {
            std::env::remove_var("NOMIFUN_BUN_PATH");
        }

        let fake = FakeEmbed {
            has: false,
            blob: b"",
            sha256: "",
            version: "",
        };
        let res = resolve_with(&fake);
        // If bun is on the test host's PATH -> Ok; otherwise NotFound.
        // Both are correct behaviors for this branch.
        match res {
            Ok(_) | Err(ResolveError::NotFound) => {}
            Err(e) => panic!("unexpected error: {e:?}"),
        }
    }

    #[test]
    fn env_override_wins_over_embed() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        // SAFETY: ENV_LOCK serializes tests that mutate NOMIFUN_BUN_PATH.
        unsafe {
            std::env::set_var("NOMIFUN_BUN_PATH", &path);
        }

        let payload = b"anything";
        let fake_blob: &'static [u8] = Box::leak(make_blob(payload).into_boxed_slice());
        let fake_sha: &'static str = Box::leak(sha(payload).into_boxed_str());
        let fake = FakeEmbed {
            has: true,
            blob: fake_blob,
            sha256: fake_sha,
            version: "1.0",
        };

        let result = resolve_with(&fake).unwrap();
        assert_eq!(result, path);

        // SAFETY: ENV_LOCK serializes tests that mutate NOMIFUN_BUN_PATH.
        unsafe {
            std::env::remove_var("NOMIFUN_BUN_PATH");
        }
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
    fn bad_env_override_falls_through_to_embed() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: ENV_LOCK serializes tests that mutate NOMIFUN_BUN_PATH.
        unsafe {
            std::env::set_var("NOMIFUN_BUN_PATH", "/definitely/does/not/exist");
        }

        let fake = FakeEmbed {
            has: false,
            blob: b"",
            sha256: "",
            version: "",
        };
        let res = resolve_with(&fake);
        // Must not error out as `Extract(...)` from env override branch;
        // must fall through to which() (Ok or NotFound — both fine).
        match res {
            Ok(_) | Err(ResolveError::NotFound) => {}
            Err(e) => panic!("unexpected error: {e:?}"),
        }

        // SAFETY: ENV_LOCK serializes tests that mutate NOMIFUN_BUN_PATH.
        unsafe {
            std::env::remove_var("NOMIFUN_BUN_PATH");
        }
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
