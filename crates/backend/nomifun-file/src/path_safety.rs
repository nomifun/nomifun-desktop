use std::path::{Component, Path, PathBuf};

use nomifun_common::AppError;

/// Canonicalize `path` and verify it falls within one of the `allowed_roots`.
///
/// This prevents path traversal attacks (e.g. `../../etc/passwd`) by:
/// 1. Resolving symlinks and `..` components via `std::fs::canonicalize`.
/// 2. Checking that the resolved path starts with at least one allowed root.
///
/// # Errors
///
/// - `AppError::BadRequest` if `path` does not exist or cannot be
///   canonicalized, or if it falls outside all allowed roots.
pub fn validate_path(path: &str, allowed_roots: &[&Path]) -> Result<PathBuf, AppError> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|e| AppError::BadRequest(format!("cannot resolve path '{}': {}", path, e)))?;

    let is_allowed = allowed_roots.iter().any(|root| {
        // Canonicalize the root as well so that symlinks (e.g. macOS
        // /var → /private/var) are handled consistently.
        match std::fs::canonicalize(root) {
            Ok(canonical_root) => canonical.starts_with(&canonical_root),
            Err(_) => false,
        }
    });

    if is_allowed {
        Ok(canonical)
    } else {
        Err(AppError::Forbidden(format!(
            "path '{}' is outside the allowed sandbox",
            path
        )))
    }
}

/// Like [`validate_path`], but also accepts a request-scoped extra root.
pub fn validate_path_with_extra_root(
    path: &str,
    base_roots: &[&Path],
    extra: Option<&Path>,
) -> Result<PathBuf, AppError> {
    let mut allowed_roots = base_roots.to_vec();
    if let Some(extra_root) = extra {
        allowed_roots.push(extra_root);
    }
    validate_path(path, &allowed_roots)
}

/// Like [`validate_path`] but the target does not need to exist yet.
///
/// Canonicalizes the *parent directory* and verifies it is within the sandbox,
/// then appends the file name component. Existing targets are resolved and
/// checked too, so a final symlink cannot redirect a write outside the roots.
/// Dangling links fail closed. This is not a lock against concurrent renames.
///
/// # Errors
///
/// Same as [`validate_path`], plus `AppError::BadRequest` if the path has
/// no parent or no file-name component.
pub fn validate_path_for_write(path: &str, allowed_roots: &[&Path]) -> Result<PathBuf, AppError> {
    let p = Path::new(path);

    let parent = p
        .parent()
        .ok_or_else(|| AppError::BadRequest(format!("path '{}' has no parent directory", path)))?;

    let file_name = p
        .file_name()
        .ok_or_else(|| AppError::BadRequest(format!("path '{}' has no file name component", path)))?;

    // Hygiene before containment: a drive-relative file name such as `c:evil`
    // makes the `join` below discard `canonical_parent` outright, so proving the
    // parent is inside the sandbox would tell us nothing about the result.
    reject_unsafe_file_name(path, file_name)?;

    let canonical_parent = std::fs::canonicalize(parent)
        .map_err(|e| AppError::BadRequest(format!("cannot resolve parent of '{}': {}", path, e)))?;

    let is_allowed = allowed_roots.iter().any(|root| match std::fs::canonicalize(root) {
        Ok(canonical_root) => canonical_parent.starts_with(&canonical_root),
        Err(_) => false,
    });

    if !is_allowed {
        return Err(AppError::Forbidden(format!(
            "path '{}' is outside the allowed sandbox",
            path
        )));
    }

    let joined = canonical_parent.join(file_name);
    // Belt and braces: containment was proven for `canonical_parent`, but it is
    // `joined` that the caller writes to, and `join` is not guaranteed to extend
    // the parent. Re-check the thing we actually return.
    if !joined.starts_with(&canonical_parent) {
        return Err(AppError::Forbidden(format!(
            "path '{}' is outside the allowed sandbox",
            path
        )));
    }
    match std::fs::symlink_metadata(&joined) {
        Ok(_) => validate_path(path, allowed_roots),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(joined),
        Err(error) => Err(AppError::BadRequest(format!(
            "cannot inspect write target '{}': {}", path, error
        ))),
    }
}

/// Compare already validated, canonical-root-relative patch targets before
/// creating directories or publishing any file. Windows case sensitivity is a
/// property of each parent directory, including newly inherited directories.
pub(crate) fn patch_targets_overlap(
    root: &Path,
    left: &Path,
    right: &Path,
) -> Result<bool, AppError> {
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let _ = root;
        Ok(left.starts_with(right) || right.starts_with(left))
    }
    #[cfg(target_os = "macos")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        use unicode_normalization::UnicodeNormalization;

        let relative = |path: &Path| {
            path.strip_prefix(root).map(Path::to_path_buf).map_err(|_| {
                AppError::Forbidden("patch target is outside the bound workspace".to_owned())
            })
        };
        let root_path = CString::new(root.as_os_str().as_bytes()).map_err(|_| {
            AppError::BadRequest("workspace root contains a NUL byte".to_owned())
        })?;
        // SAFETY: root_path is a live NUL-terminated path. _PC_CASE_SENSITIVE
        // returns the filesystem comparison mode without mutating the volume.
        let case_sensitive = unsafe { libc::pathconf(root_path.as_ptr(), libc::_PC_CASE_SENSITIVE) };
        if case_sensitive < 0 {
            return Err(AppError::Conflict(format!(
                "cannot determine macOS volume case sensitivity for '{}': {}",
                root.display(),
                std::io::Error::last_os_error()
            )));
        }
        let component_key = |component: Component<'_>| -> Result<String, AppError> {
            let Component::Normal(name) = component else {
                return Err(AppError::BadRequest(
                    "patch target contains a non-normal path component".to_owned(),
                ));
            };
            let name = name.to_str().ok_or_else(|| {
                AppError::BadRequest("patch target is not valid UTF-8".to_owned())
            })?;
            let normalized = name.nfd().collect::<String>();
            Ok(if case_sensitive == 0 {
                normalized.to_lowercase()
            } else {
                normalized
            })
        };
        let left = relative(left)?;
        let right = relative(right)?;
        let mut left = left.components();
        let mut right = right.components();
        loop {
            match (left.next(), right.next()) {
                (Some(a), Some(b)) if component_key(a)? == component_key(b)? => continue,
                (Some(_), Some(_)) => return Ok(false),
                // Equality or either normalized path being an ancestor is an overlap.
                (None, _) | (_, None) => return Ok(true),
            }
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Globalization::{CompareStringOrdinal, CSTR_EQUAL};

        let relative = |path: &Path| {
            path.strip_prefix(root).map(Path::to_path_buf).map_err(|_| {
                AppError::Forbidden("patch target is outside the bound workspace".to_owned())
            })
        };
        let left = relative(left)?;
        let right = relative(right)?;
        let mut parent = root.to_path_buf();
        for (a, b) in left.components().zip(right.components()) {
            if a != b {
                let left_name: Vec<u16> = a.as_os_str().encode_wide().collect();
                let right_name: Vec<u16> = b.as_os_str().encode_wide().collect();
                let count = |len| i32::try_from(len).map_err(|_| {
                    AppError::BadRequest("patch path component is too long".to_owned())
                });
                let left_len = count(left_name.len())?;
                let right_len = count(right_name.len())?;
                // Ordinal comparison does not expand characters or use the
                // process locale. Keep exact names on case-sensitive parents.
                // SAFETY: both UTF-16 buffers remain live for their checked lengths.
                let compared = unsafe {
                    CompareStringOrdinal(left_name.as_ptr(), left_len, right_name.as_ptr(), right_len, 1)
                };
                if compared == 0 {
                    return Err(AppError::Internal(format!(
                        "cannot compare patch target names: {}", std::io::Error::last_os_error()
                    )));
                }
                if compared != CSTR_EQUAL || windows_directory_case_sensitive(&parent, root)? {
                    return Ok(false);
                }
            }
            parent.push(a.as_os_str());
        }
        Ok(true)
    }
}

#[cfg(windows)]
fn windows_directory_case_sensitive(path: &Path, workspace_root: &Path) -> Result<bool, AppError> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_CASE_SENSITIVE_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
        FileCaseSensitiveInfo, GetFileInformationByHandleEx,
    };
    use windows_sys::Win32::System::SystemServices::FILE_CS_FLAG_CASE_SENSITIVE_DIR;

    // Native Windows directory creation inherits this flag. Walk only absent
    // parents; denied or unsupported queries cannot prove distinct resources.
    for parent in path.ancestors().take_while(|parent| parent.starts_with(workspace_root)) {
        let opened = std::fs::OpenOptions::new().read(true)
            .access_mode(FILE_READ_ATTRIBUTES).custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(parent);
        let file = match opened {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(AppError::Conflict(format!(
                "cannot inspect patch parent '{}': {error}", parent.display()
            ))),
        };
        let mut info = FILE_CASE_SENSITIVE_INFO::default();
        // SAFETY: File owns the live handle and info is a correctly sized output buffer.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(), FileCaseSensitiveInfo,
                (&mut info as *mut FILE_CASE_SENSITIVE_INFO).cast(),
                std::mem::size_of_val(&info) as u32,
            )
        };
        if ok == 0 {
            return Err(AppError::Conflict(format!(
                "cannot determine case sensitivity of patch parent '{}': {}",
                parent.display(), std::io::Error::last_os_error()
            )));
        }
        return Ok(info.Flags & FILE_CS_FLAG_CASE_SENSITIVE_DIR != 0);
    }
    Err(AppError::Conflict("patch parent disappeared during preparation".to_owned()))
}

/// Reject a file-name component that `Path::join` would not treat as a plain
/// child of its parent. Shared by both [`PathAuthority`] arms: containment is
/// what `Unrestricted` drops, and this is path *hygiene*, which it keeps.
fn reject_unsafe_file_name(path: &str, file_name: &std::ffi::OsStr) -> Result<(), AppError> {
    let unsafe_name = file_name.to_str().is_none_or(is_unsafe_path_segment);
    if unsafe_name {
        return Err(AppError::BadRequest(format!(
            "path '{}' has an invalid file name component",
            path
        )));
    }
    Ok(())
}

/// Check whether a raw path string contains suspicious traversal patterns.
///
/// This is a fast pre-check that catches obvious `..` usage before the
/// more expensive `canonicalize` call. It does NOT replace full validation
/// — always call [`validate_path`] or [`validate_path_for_write`] as the
/// authoritative check.
pub fn has_traversal(path: &str) -> bool {
    path.contains('\0')
        || Path::new(path)
            .components()
            .any(|component| matches!(component, Component::ParentDir))
}

/// Reject a *single* path segment (a new file name, an id used as a directory)
/// that would not stay a segment once joined onto a parent.
///
/// [`has_traversal`] plus a separator check is not enough on Windows:
///
/// - `"c:evil"` is a **drive-relative** path. `Path::is_absolute` reads `false`
///   for it, so no absolute check fires, yet `parent.join("c:evil")` discards
///   `parent` entirely and resolves against the current directory of drive C:.
///   For a [`PathAuthority::Confined`] caller that walks the file straight out
///   of the sandbox.
/// - `"a.txt:x"` names an NTFS **alternate data stream** on `a.txt` rather than
///   a file, so the bytes land on a hidden stream that `read_dir` never lists.
///
/// A colon cannot appear in a legal Windows file name at all, so rejecting it
/// there costs nothing. On Unix it is an ordinary name byte and stays allowed —
/// the drive-prefix shape is still rejected everywhere so that a name accepted
/// on one platform cannot become an escape on another.
///
/// A lone `"."` is rejected because `parent.join(".")` resolves back to
/// `parent`, aiming a per-entry operation at the whole directory.
///
/// The Windows-only rules below match `nomifun-knowledge`'s
/// `validate_portable_path_component`, which is this repo's stricter
/// portable-name gate; they are conditional here because this crate is a
/// general-purpose file manager and all of these names are legal on Unix:
///
/// - A trailing `'.'` or `' '` is silently stripped by Win32, so `"a.txt."` and
///   `"a.txt"` become the same file — a validated rename can clobber an
///   unrelated one.
/// - A reserved device name (`CON`, `NUL`, `COM1`…) opens the *device*, so the
///   write appears to succeed, stores nothing, and never appears in `read_dir`.
pub fn is_unsafe_path_segment(name: &str) -> bool {
    if name.is_empty() || name == "." || name.contains('\0') || name.contains('/') || name.contains('\\') {
        return true;
    }
    if has_traversal(name) {
        return true;
    }
    let bytes = name.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return true;
    }
    if !cfg!(windows) {
        return false;
    }
    if name.chars().any(|c| c <= '\u{1f}' || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        || name.ends_with('.') || name.ends_with(' ')
    {
        return true;
    }
    is_windows_reserved_device_name(name)
}

/// True for a Win32 reserved device name, with or without an extension
/// (`NUL`, `nul.txt`, `COM1`). Matched case-insensitively on the basename.
fn is_windows_reserved_device_name(name: &str) -> bool {
    let basename = name.split_once('.').map_or(name, |(base, _)| base);
    let lower = basename.trim_end().to_ascii_lowercase();
    if matches!(lower.as_str(), "con" | "prn" | "aux" | "nul") {
        return true;
    }
    lower
        .strip_prefix("com")
        .or_else(|| lower.strip_prefix("lpt"))
        .is_some_and(|suffix| {
            // Win32 also recognizes these ISO-8859-1 superscript digits in
            // DOS device names, including when followed by an extension.
            let mut chars = suffix.chars();
            matches!(chars.next(), Some('1'..='9' | '¹' | '²' | '³')) && chars.next().is_none()
        })
}

/// The filesystem authority a single file operation runs under, resolved
/// per-call from the caller's **trust surface** (see the gateway
/// `CallerCtx::surface`): a trusted local desktop session (the machine owner
/// driving their own agent) gets [`PathAuthority::Unrestricted`] — the OS
/// user's own permissions are the only boundary; external channel / remote
/// sessions get [`PathAuthority::Confined`] to their session workspace.
///
/// File access uses a single, surface-scoped model. Traversal / NUL bytes are
/// rejected in BOTH modes — `Unrestricted` removes root *containment*, not path
/// hygiene.
#[derive(Debug, Clone)]
pub enum PathAuthority {
    /// No sandbox-root containment: the OS user's own filesystem permissions
    /// are the boundary. For the trusted local owner (desktop surface).
    Unrestricted,
    /// The path must resolve within one of these roots (the historical
    /// `allowed_roots` behaviour). For untrusted / external surfaces, or the
    /// default the UI/file-routes pass (`allowed_roots ∪ workspace`).
    Confined(Vec<PathBuf>),
    /// Agent workspace authority with the platform-owned `.nomifun` subtree
    /// excluded even when a symlink/junction aliases it under another name.
    Workspace(PathBuf),
}

pub(crate) fn reject_workspace_owner_canonical_path(
    canonical_root: &Path,
    canonical_target: &Path,
) -> Result<(), AppError> {
    let relative = canonical_target.strip_prefix(canonical_root).map_err(|_| {
        AppError::Forbidden("workspace path is outside the bound resource".into())
    })?;
    if relative.components().next().is_some_and(|component| {
        matches!(component, Component::Normal(value)
            if crate::artifact_store::is_workspace_owner_component(value))
    }) {
        return Err(AppError::NotFound("workspace path was not found".into()));
    }
    Ok(())
}

/// Validate the nearest existing ancestor so a missing write target cannot
/// enter `.nomifun` through an already-present alias directory.
pub(crate) fn validate_workspace_candidate(path: &Path, root: &Path) -> Result<(), AppError> {
    let canonical_root = std::fs::canonicalize(root).map_err(|error| {
        AppError::BadRequest(format!("cannot resolve workspace root: {error}"))
    })?;
    let mut probe = path;
    let canonical_probe = loop {
        match std::fs::canonicalize(probe) {
            Ok(value) => break value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                probe = probe.parent().ok_or_else(|| {
                    AppError::BadRequest("workspace path has no existing ancestor".into())
                })?;
            }
            Err(error) => {
                return Err(AppError::BadRequest(format!(
                    "cannot resolve workspace path ancestor: {error}"
                )));
            }
        }
    };
    reject_workspace_owner_canonical_path(&canonical_root, &canonical_probe)
}

/// Authority-aware variant of [`validate_path`]: the target must exist.
///
/// - [`PathAuthority::Unrestricted`] → canonicalise only (no root check).
/// - [`PathAuthority::Confined`] → identical to [`validate_path`] against the
///   confined roots.
pub fn validate_path_authority(path: &str, authority: &PathAuthority) -> Result<PathBuf, AppError> {
    match authority {
        PathAuthority::Unrestricted => std::fs::canonicalize(path)
            .map_err(|e| AppError::BadRequest(format!("cannot resolve path '{}': {}", path, e))),
        PathAuthority::Confined(roots) => {
            let refs: Vec<&Path> = roots.iter().map(PathBuf::as_path).collect();
            validate_path(path, &refs)
        }
        PathAuthority::Workspace(root) => {
            let canonical_root = std::fs::canonicalize(root).map_err(|error| {
                AppError::BadRequest(format!("cannot resolve workspace root: {error}"))
            })?;
            let canonical = validate_path(path, &[root.as_path()])?;
            reject_workspace_owner_canonical_path(&canonical_root, &canonical)?;
            Ok(canonical)
        }
    }
}

/// Authority-aware variant of [`validate_path_for_write`]: the target need not
/// exist yet (its parent directory must).
///
/// - [`PathAuthority::Unrestricted`] → canonicalise the parent (no root check),
///   re-append the file name.
/// - [`PathAuthority::Confined`] → identical to [`validate_path_for_write`].
pub fn validate_path_for_write_authority(
    path: &str,
    authority: &PathAuthority,
) -> Result<PathBuf, AppError> {
    match authority {
        PathAuthority::Unrestricted => {
            let p = Path::new(path);
            let parent = p
                .parent()
                .ok_or_else(|| AppError::BadRequest(format!("path '{}' has no parent directory", path)))?;
            let file_name = p
                .file_name()
                .ok_or_else(|| AppError::BadRequest(format!("path '{}' has no file name component", path)))?;
            // `Unrestricted` drops root containment, not path hygiene: a
            // drive-relative name would still silently relocate the write off
            // the parent the caller named.
            reject_unsafe_file_name(path, file_name)?;
            let canonical_parent = std::fs::canonicalize(parent)
                .map_err(|e| AppError::BadRequest(format!("cannot resolve parent of '{}': {}", path, e)))?;
            Ok(canonical_parent.join(file_name))
        }
        PathAuthority::Confined(roots) => {
            let refs: Vec<&Path> = roots.iter().map(PathBuf::as_path).collect();
            validate_path_for_write(path, &refs)
        }
        PathAuthority::Workspace(root) => {
            let canonical_root = std::fs::canonicalize(root).map_err(|error| {
                AppError::BadRequest(format!("cannot resolve workspace root: {error}"))
            })?;
            let target = validate_path_for_write(path, &[root.as_path()])?;
            reject_workspace_owner_canonical_path(&canonical_root, &target)?;
            Ok(target)
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn missing_patch_root_does_not_inherit_case_rules_from_outside_the_workspace() {
        let fixture = tempfile::tempdir().unwrap();
        let missing_root = fixture.path().join("missing-workspace");
        assert!(super::patch_targets_overlap(
            &missing_root, &missing_root.join("Report.txt"), &missing_root.join("report.txt"),
        ).is_err());
    }

    use super::*;
    use std::fs;

    #[test]
    fn validate_path_within_sandbox() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hello.txt");
        fs::write(&file, "hi").unwrap();

        let result = validate_path(file.to_str().unwrap(), &[dir.path()]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), fs::canonicalize(&file).unwrap());
    }

    #[test]
    fn validate_path_rejects_outside_sandbox() {
        let sandbox = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("secret.txt");
        fs::write(&file, "secret").unwrap();

        let result = validate_path(file.to_str().unwrap(), &[sandbox.path()]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)), "unexpected error: {err}");
    }

    #[test]
    fn validate_path_rejects_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("does_not_exist.txt");

        let result = validate_path(fake.to_str().unwrap(), &[dir.path()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot resolve"));
    }

    #[cfg(unix)]
    #[test]
    fn validate_path_resolves_symlink_within_sandbox() {
        let dir = tempfile::tempdir().unwrap();
        let real_file = dir.path().join("real.txt");
        fs::write(&real_file, "content").unwrap();

        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(&real_file, &link).unwrap();

        let result = validate_path(link.to_str().unwrap(), &[dir.path()]);
        assert!(result.is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn validate_path_rejects_symlink_escaping_sandbox() {
        let sandbox = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.txt");
        fs::write(&secret, "secret").unwrap();

        let link = sandbox.path().join("escape");
        std::os::unix::fs::symlink(&secret, &link).unwrap();

        let result = validate_path(link.to_str().unwrap(), &[sandbox.path()]);
        assert!(result.is_err());
    }

    #[test]
    fn validate_path_for_write_new_file() {
        let dir = tempfile::tempdir().unwrap();
        // File does not exist yet, but parent does
        let new_file = dir.path().join("new.txt");

        let result = validate_path_for_write(new_file.to_str().unwrap(), &[dir.path()]);
        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(resolved.ends_with("new.txt"));
    }

    #[test]
    fn validate_path_for_write_rejects_outside_sandbox() {
        let sandbox = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("evil.txt");

        let result = validate_path_for_write(target.to_str().unwrap(), &[sandbox.path()]);
        assert!(result.is_err());
    }

    #[test]
    fn validate_path_for_write_rejects_no_parent() {
        // A bare root path on unix is "/" which has no parent in some
        // interpretations, but Path::new("/").parent() returns Some("").
        // Test a truly pathological case.
        let result = validate_path_for_write("", &[Path::new("/tmp")]);
        assert!(result.is_err());
    }

    #[test]
    fn validate_path_multiple_allowed_roots() {
        let root_a = tempfile::tempdir().unwrap();
        let root_b = tempfile::tempdir().unwrap();
        let file_a = root_a.path().join("a.txt");
        let file_b = root_b.path().join("b.txt");
        fs::write(&file_a, "a").unwrap();
        fs::write(&file_b, "b").unwrap();

        let roots = [root_a.path(), root_b.path()];

        assert!(validate_path(file_a.to_str().unwrap(), &roots).is_ok());
        assert!(validate_path(file_b.to_str().unwrap(), &roots).is_ok());
    }

    #[test]
    fn has_traversal_detects_dot_dot() {
        assert!(has_traversal("../etc/passwd"));
        assert!(has_traversal("/safe/../../etc"));
        assert!(has_traversal("a\0b"));
    }

    /// The parent can sit safely inside the sandbox while the *joined* result
    /// does not: `join("c:evil.txt")` is drive-relative and discards the parent,
    /// so `validate_path_for_write` used to return `Ok` with an escaped path.
    /// `is_absolute()` reads `false` for it, so no absolute check would fire.
    #[test]
    fn validate_path_for_write_rejects_drive_relative_file_name() {
        let sandbox = tempfile::tempdir().unwrap();
        for bad in ["c:evil.txt", "C:evil.txt"] {
            let attack = format!("{}{}{}", sandbox.path().display(), std::path::MAIN_SEPARATOR, bad);
            assert!(!has_traversal(&attack), "premise: the traversal pre-check misses it");
            let result = validate_path_for_write(&attack, &[sandbox.path()]);
            assert!(result.is_err(), "must reject {attack:?}, got {result:?}");
        }
        // A legitimate not-yet-existing file in the same directory still works.
        let ok = format!("{}{}ok.txt", sandbox.path().display(), std::path::MAIN_SEPARATOR);
        let resolved = validate_path_for_write(&ok, &[sandbox.path()]).unwrap();
        assert!(resolved.starts_with(fs::canonicalize(sandbox.path()).unwrap()));
    }

    /// Every `Ok` from the write validator must be inside the sandbox — the
    /// property the callers actually rely on.
    #[test]
    fn validate_path_for_write_ok_results_stay_in_root() {
        let sandbox = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(sandbox.path()).unwrap();
        for name in ["plain.txt", "with space.md", "dotted.name.txt", ".hidden"] {
            let candidate = format!("{}{}{}", sandbox.path().display(), std::path::MAIN_SEPARATOR, name);
            if let Ok(resolved) = validate_path_for_write(&candidate, &[sandbox.path()]) {
                assert!(resolved.starts_with(&root), "{name:?} escaped: {resolved:?}");
            }
        }
    }

    #[test]
    fn unsafe_path_segment_covers_windows_name_hazards() {
        for bad in ["", ".", "..", "a/b", "a\\b", "c:evil", "C:evil", "a\0b"] {
            assert!(is_unsafe_path_segment(bad), "must reject {bad:?}");
        }
        // A non-prefix colon is an NTFS stream on Windows, an ordinary byte on Unix.
        assert_eq!(is_unsafe_path_segment("note.md:evil"), cfg!(windows));
        // Trailing dot/space collapse onto a different file; device names swallow
        // the write. Both are legal Unix names, so both are Windows-only rules.
        for windows_only in ["a.txt.", "a.txt ", "NUL", "nul.txt", "CON", "com1", "LPT9"] {
            assert_eq!(
                is_unsafe_path_segment(windows_only),
                cfg!(windows),
                "{windows_only:?} must be rejected exactly on Windows"
            );
        }
        // Names that merely resemble device names stay usable everywhere.
        for ok in ["console.txt", "nulls.md", "com0", "com10", "lpt.md"] {
            assert!(!is_unsafe_path_segment(ok), "must accept {ok:?}");
        }
        for ok in ["note.md", "with space.txt", ".hidden", "a.b.c"] {
            assert!(!is_unsafe_path_segment(ok), "must accept {ok:?}");
        }
    }

    #[test]
    fn has_traversal_clean_paths() {
        assert!(!has_traversal("/home/user/project/src/main.rs"));
        assert!(!has_traversal("relative/path/file.txt"));
        assert!(!has_traversal(".hidden_file"));
    }

    #[test]
    fn has_traversal_allows_legal_filename_with_dots() {
        assert!(!has_traversal("foo..bar.md"));
        assert!(!has_traversal("README..old"));
        assert!(!has_traversal("my..file.txt"));
    }

    #[test]
    fn has_traversal_still_rejects_parent_dir() {
        assert!(has_traversal("../etc"));
        assert!(has_traversal("a/../b"));
        assert!(has_traversal(".."));
        assert!(has_traversal("/foo/../bar"));
    }

    #[test]
    fn validate_path_accepts_extra_workspace_root() {
        let sandbox = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let file = workspace.path().join("hello.txt");
        fs::write(&file, "hi").unwrap();

        let result = validate_path_with_extra_root(file.to_str().unwrap(), &[sandbox.path()], Some(workspace.path()));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), fs::canonicalize(file).unwrap());
    }

    #[test]
    fn authority_unrestricted_allows_any_existing_path() {
        // A path in a directory that is NOT an allowed root is accepted under
        // Unrestricted (the trusted local owner's OS permissions are the boundary).
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("owned.txt");
        fs::write(&file, "x").unwrap();
        let result = validate_path_authority(file.to_str().unwrap(), &PathAuthority::Unrestricted);
        assert!(result.is_ok(), "unrestricted must allow any existing path");
        assert_eq!(result.unwrap(), fs::canonicalize(&file).unwrap());
    }

    #[test]
    fn authority_unrestricted_write_allows_new_file_outside_roots() {
        let outside = tempfile::tempdir().unwrap();
        let new_file = outside.path().join("new.txt"); // parent exists, file doesn't
        let result = validate_path_for_write_authority(new_file.to_str().unwrap(), &PathAuthority::Unrestricted);
        assert!(result.is_ok(), "unrestricted write must allow a new file anywhere the parent exists");
        assert!(result.unwrap().ends_with("new.txt"));
    }

    #[test]
    fn authority_confined_matches_allowed_roots_behaviour() {
        let sandbox = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let inside = sandbox.path().join("ok.txt");
        let evil = outside.path().join("evil.txt");
        fs::write(&inside, "hi").unwrap();
        fs::write(&evil, "no").unwrap();

        let authority = PathAuthority::Confined(vec![sandbox.path().to_path_buf()]);
        assert!(validate_path_authority(inside.to_str().unwrap(), &authority).is_ok());
        let err = validate_path_authority(evil.to_str().unwrap(), &authority).unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)), "confined must reject outside root: {err}");
    }

    #[test]
    fn authority_confined_write_rejects_outside_root() {
        let sandbox = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("evil.txt");
        let authority = PathAuthority::Confined(vec![sandbox.path().to_path_buf()]);
        assert!(validate_path_for_write_authority(target.to_str().unwrap(), &authority).is_err());
        let inside = sandbox.path().join("ok.txt");
        assert!(validate_path_for_write_authority(inside.to_str().unwrap(), &authority).is_ok());
    }
}
