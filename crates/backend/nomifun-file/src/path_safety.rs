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
/// then appends the file name component. Useful for write/create operations
/// where the file itself may not exist yet.
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

    Ok(canonical_parent.join(file_name))
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

#[cfg(test)]
mod tests {
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

    #[test]
    fn validate_path_resolves_symlink_within_sandbox() {
        let dir = tempfile::tempdir().unwrap();
        let real_file = dir.path().join("real.txt");
        fs::write(&real_file, "content").unwrap();

        let link = dir.path().join("link.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_file, &link).unwrap();
        #[cfg(not(unix))]
        {
            // Skip on non-unix
            return;
        }

        let result = validate_path(link.to_str().unwrap(), &[dir.path()]);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_path_rejects_symlink_escaping_sandbox() {
        let sandbox = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.txt");
        fs::write(&secret, "secret").unwrap();

        let link = sandbox.path().join("escape");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        #[cfg(not(unix))]
        {
            return;
        }

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
}
