//! Shallow, WebUI-only directory browser backing `GET /api/fs/browse`.
//!
//! Unlike the workspace-scoped `/api/fs/dir` endpoint, this handler lists a
//! single directory level and surfaces navigation hints (`can_go_up`,
//! `parent_path`) plus a `__ROOT__` sentinel for the Windows drive picker.
//! It is only reachable in WebUI deployments; the Electron desktop path uses
//! the native OS dialog and never hits this route.
//!
//! Allowed roots intentionally widen to `cwd` + `home` + (on Windows) every
//! available drive letter + (on Unix) `/`, matching the pre-M6 Express
//! implementation that this replaces.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use nomifun_api_types::{BrowseDirectoryResponse, BrowseEntry};
use nomifun_common::AppError;

/// Sentinel returned as `parent_path` on Windows drive roots, signaling the
/// frontend to navigate back to the drive-list screen.
pub const ROOT_SENTINEL: &str = "__ROOT__";

/// Upper bound on directory entries returned per call. Large directories are
/// truncated to keep the response cheap to render.
pub const MAX_BROWSE_ITEMS: usize = 500;

// ---------------------------------------------------------------------------
// Allowed-root resolution
// ---------------------------------------------------------------------------

/// Build the allow-list of roots that `/api/fs/browse` may traverse.
///
/// Returns canonicalized paths; callers compare against these with
/// `Path::starts_with`. Duplicates and unreadable entries are silently
/// dropped.
pub fn default_browse_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();

    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Some(home) = dirs::home_dir() {
        roots.push(home);
    }

    #[cfg(windows)]
    {
        roots.extend(enumerate_windows_drives());
    }

    #[cfg(unix)]
    {
        // Widest possible sandbox on Unix — the pre-M6 Express endpoint
        // allowed `/`, and the WebUI host-files use case genuinely needs to
        // reach outside $HOME (e.g. `/Volumes/*` on macOS).
        roots.push(PathBuf::from("/"));
    }

    let mut canonical: Vec<PathBuf> = roots
        .into_iter()
        .filter_map(|p| fs::canonicalize(&p).ok().or(Some(p)))
        .collect();
    canonical.sort();
    canonical.dedup();
    canonical
}

#[cfg(windows)]
fn enumerate_windows_drives() -> Vec<PathBuf> {
    let mut drives = Vec::new();
    for letter in b'A'..=b'Z' {
        let path = PathBuf::from(format!("{}:\\", letter as char));
        if path.is_dir() {
            drives.push(path);
        }
    }
    drives
}

/// Produce the Windows drive-list screen response.
#[cfg(windows)]
pub fn drive_list_response() -> BrowseDirectoryResponse {
    let items = enumerate_windows_drives()
        .into_iter()
        .map(|drive| {
            let letter = drive.to_string_lossy().chars().next().unwrap_or('?');
            BrowseEntry {
                name: format!("{letter}:"),
                path: drive.to_string_lossy().into_owned(),
                is_directory: true,
                is_file: false,
                size: None,
                modified: None,
            }
        })
        .collect();
    BrowseDirectoryResponse {
        current_path: String::new(),
        parent_path: None,
        items,
        can_go_up: false,
        truncated: false,
        is_root: Some(true),
    }
}

// ---------------------------------------------------------------------------
// Path validation
// ---------------------------------------------------------------------------

/// Canonicalize `raw` and verify it lives under one of the allowed roots.
///
/// `~` expansion is handled explicitly so users can paste `~/Documents`
/// into the picker. Symlinks are resolved via `canonicalize` before the
/// sandbox check, so a link pointing outside the allow-list is rejected.
pub fn resolve_browse_path(raw: &str, allowed_roots: &[PathBuf]) -> Result<PathBuf, AppError> {
    if raw.contains('\0') {
        return Err(AppError::BadRequest("path contains null byte".into()));
    }

    let expanded = expand_tilde(raw.trim());
    let canonical = fs::canonicalize(&expanded).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => AppError::NotFound(format!("path not found: {}", raw)),
        _ => AppError::BadRequest(format!("cannot resolve path '{}': {}", raw, e)),
    })?;

    let allowed = allowed_roots.iter().any(|root| match fs::canonicalize(root) {
        Ok(canonical_root) => canonical.starts_with(&canonical_root),
        Err(_) => false,
    });

    if !allowed {
        return Err(AppError::Forbidden(format!(
            "path '{}' is outside the allowed sandbox",
            raw
        )));
    }

    Ok(canonical)
}

fn expand_tilde(input: &str) -> PathBuf {
    if let Some(stripped) = input.strip_prefix('~')
        && let Some(home) = dirs::home_dir()
    {
        let relative = stripped.trim_start_matches(['/', '\\']);
        return if relative.is_empty() { home } else { home.join(relative) };
    }
    PathBuf::from(input)
}

// ---------------------------------------------------------------------------
// Directory listing
// ---------------------------------------------------------------------------

/// List a single directory level.
///
/// `dir` must already be a canonicalized path that the caller has verified
/// lives under `allowed_roots`. Hidden entries (name starting with `.`) are
/// filtered out to match the legacy Express behavior.
pub fn list_directory(
    dir: &Path,
    show_files: bool,
    allowed_roots: &[PathBuf],
) -> Result<BrowseDirectoryResponse, AppError> {
    let metadata = fs::metadata(dir).map_err(|e| AppError::NotFound(format!("cannot access directory: {}", e)))?;
    if !metadata.is_dir() {
        return Err(AppError::BadRequest("path is not a directory".into()));
    }

    let read = fs::read_dir(dir).map_err(|e| AppError::Internal(format!("readdir failed: {}", e)))?;

    let mut items: Vec<BrowseEntry> = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let entry_path = entry.path();
        let stat = match fs::metadata(&entry_path) {
            Ok(m) => m,
            Err(_) => continue, // skip unreadable entries (permission, dangling symlink, etc.)
        };
        let is_dir = stat.is_dir();
        let is_file = stat.is_file();
        if !show_files && !is_dir {
            continue;
        }
        items.push(BrowseEntry {
            name,
            path: entry_path.to_string_lossy().into_owned(),
            is_directory: is_dir,
            is_file,
            size: Some(stat.len()),
            modified: system_time_to_millis(stat.modified().ok()),
        });
    }

    items.sort_by(|a, b| match (a.is_directory, b.is_directory) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });

    let truncated = items.len() > MAX_BROWSE_ITEMS;
    if truncated {
        items.truncate(MAX_BROWSE_ITEMS);
    }

    let (parent_path, can_go_up) = navigation_hints(dir, allowed_roots);

    Ok(BrowseDirectoryResponse {
        current_path: dir.to_string_lossy().into_owned(),
        parent_path,
        items,
        can_go_up,
        truncated,
        is_root: None,
    })
}

fn system_time_to_millis(t: Option<SystemTime>) -> Option<i64> {
    let time = t?;
    let duration = time.duration_since(UNIX_EPOCH).ok()?;
    duration.as_millis().try_into().ok()
}

/// Compute `(parent_path, can_go_up)` for a listed directory.
///
/// - At a Windows drive root (`C:\` whose parent is itself), returns
///   `("__ROOT__", true)` so the UI jumps back to the drive picker.
/// - When the natural parent is still inside the allow-list, returns that
///   parent with `can_go_up = true`.
/// - Otherwise, returns the parent path with `can_go_up = false`; the UI
///   hides the up-arrow but keeps the path for display.
fn navigation_hints(dir: &Path, allowed_roots: &[PathBuf]) -> (Option<String>, bool) {
    let parent = match dir.parent() {
        Some(p) => p,
        None => return (None, false),
    };

    if parent == dir {
        // Drive root on Windows — parent path is the drive itself.
        if cfg!(windows) {
            return (Some(ROOT_SENTINEL.to_owned()), true);
        }
        return (None, false);
    }

    let parent_allowed = allowed_roots.iter().any(|root| match fs::canonicalize(root) {
        Ok(canonical_root) => parent.starts_with(&canonical_root),
        Err(_) => false,
    });

    (Some(parent.to_string_lossy().into_owned()), parent_allowed)
}

// ---------------------------------------------------------------------------
// High-level entry point
// ---------------------------------------------------------------------------

/// Handler-facing entry point: apply the special-case routing (empty path,
/// `__ROOT__`) and delegate to the real lister.
pub fn browse(
    raw_path: Option<&str>,
    show_files: bool,
    allowed_roots: &[PathBuf],
) -> Result<BrowseDirectoryResponse, AppError> {
    let requested = raw_path.map(str::trim).unwrap_or("");

    #[cfg(windows)]
    {
        if requested.is_empty() || requested == ROOT_SENTINEL {
            return Ok(drive_list_response());
        }
    }

    let target = if requested.is_empty() {
        std::env::current_dir()
            .map_err(|e| AppError::Internal(format!("cannot read cwd: {}", e)))?
            .to_string_lossy()
            .into_owned()
    } else {
        requested.to_owned()
    };

    let canonical = resolve_browse_path(&target, allowed_roots)?;
    list_directory(&canonical, show_files, allowed_roots)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn roots_from(paths: &[&Path]) -> Vec<PathBuf> {
        paths
            .iter()
            .map(|p| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
            .collect()
    }

    #[test]
    fn lists_directories_only_when_show_files_false() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("sub")).unwrap();
        fs::write(tmp.path().join("a.txt"), "x").unwrap();
        let roots = roots_from(&[tmp.path()]);

        let resp = browse(Some(tmp.path().to_str().unwrap()), false, &roots).unwrap();
        assert_eq!(resp.items.len(), 1);
        assert_eq!(resp.items[0].name, "sub");
    }

    #[test]
    fn lists_files_when_show_files_true() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("sub")).unwrap();
        fs::write(tmp.path().join("a.txt"), "x").unwrap();
        let roots = roots_from(&[tmp.path()]);

        let resp = browse(Some(tmp.path().to_str().unwrap()), true, &roots).unwrap();
        assert_eq!(resp.items.len(), 2);
        // directories sort before files
        assert_eq!(resp.items[0].name, "sub");
        assert_eq!(resp.items[1].name, "a.txt");
        assert!(resp.items[1].is_file);
    }

    #[test]
    fn filters_hidden_entries() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".secret"), "x").unwrap();
        fs::write(tmp.path().join("visible.txt"), "y").unwrap();
        let roots = roots_from(&[tmp.path()]);

        let resp = browse(Some(tmp.path().to_str().unwrap()), true, &roots).unwrap();
        assert_eq!(resp.items.len(), 1);
        assert_eq!(resp.items[0].name, "visible.txt");
    }

    #[test]
    fn rejects_path_outside_sandbox() {
        let sandbox = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let roots = roots_from(&[sandbox.path()]);

        let err = browse(Some(outside.path().to_str().unwrap()), false, &roots).unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)), "expected forbidden, got {err}");
    }

    #[test]
    fn rejects_nonexistent_path() {
        let sandbox = tempfile::tempdir().unwrap();
        let fake = sandbox.path().join("does-not-exist");
        let roots = roots_from(&[sandbox.path()]);

        let err = browse(Some(fake.to_str().unwrap()), false, &roots).unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)), "expected not-found, got {err}");
    }

    #[test]
    fn rejects_null_byte() {
        let sandbox = tempfile::tempdir().unwrap();
        let roots = roots_from(&[sandbox.path()]);

        let err = browse(Some("/tmp/\0evil"), false, &roots).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn rejects_file_as_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("hi.txt");
        fs::write(&file, "x").unwrap();
        let roots = roots_from(&[tmp.path()]);

        let err = browse(Some(file.to_str().unwrap()), false, &roots).unwrap_err();
        assert!(
            matches!(err, AppError::BadRequest(_)),
            "expected bad-request, got {err}"
        );
    }

    #[test]
    fn can_go_up_when_parent_inside_sandbox() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("child");
        fs::create_dir(&sub).unwrap();
        let roots = roots_from(&[tmp.path()]);

        let resp = browse(Some(sub.to_str().unwrap()), false, &roots).unwrap();
        assert!(resp.can_go_up);
        assert!(resp.parent_path.is_some());
    }

    #[test]
    fn can_go_up_false_when_parent_outside_sandbox() {
        let sandbox = tempfile::tempdir().unwrap();
        let roots = roots_from(&[sandbox.path()]);

        let resp = browse(Some(sandbox.path().to_str().unwrap()), false, &roots).unwrap();
        // The sandbox's parent is outside the allow-list, so can_go_up must be false.
        assert!(!resp.can_go_up);
    }

    #[test]
    fn truncates_large_directories() {
        let tmp = tempfile::tempdir().unwrap();
        // Create MAX_BROWSE_ITEMS + 5 directories so the filter keeps them all.
        for i in 0..(MAX_BROWSE_ITEMS + 5) {
            fs::create_dir(tmp.path().join(format!("d{i:05}"))).unwrap();
        }
        let roots = roots_from(&[tmp.path()]);

        let resp = browse(Some(tmp.path().to_str().unwrap()), false, &roots).unwrap();
        assert_eq!(resp.items.len(), MAX_BROWSE_ITEMS);
        assert!(resp.truncated);
    }

    #[test]
    fn empty_path_defaults_to_cwd_on_unix() {
        #[cfg(unix)]
        {
            let cwd = std::env::current_dir().unwrap();
            let roots = roots_from(&[cwd.as_path()]);
            let resp = browse(Some(""), false, &roots).unwrap();
            assert!(!resp.is_root.unwrap_or(false));
            assert_eq!(
                fs::canonicalize(&resp.current_path).unwrap(),
                fs::canonicalize(&cwd).unwrap()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escaping_sandbox() {
        let sandbox = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = sandbox.path().join("escape");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        let roots = roots_from(&[sandbox.path()]);

        let err = browse(Some(link.to_str().unwrap()), false, &roots).unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)), "expected forbidden, got {err}");
    }

    #[test]
    fn tilde_expands_to_home() {
        let home = dirs::home_dir().expect("home dir");
        let expanded = expand_tilde("~/Documents");
        assert_eq!(expanded, home.join("Documents"));
        assert_eq!(expand_tilde("~"), home);
    }
}
