//! Agent file creation may create missing parent directories. Validation is
//! separate from creation so an invalid multi-file patch has no side effects.
use std::path::{Component, Path, PathBuf};

use cap_fs_ext::DirExt as _;
use cap_std::{ambient_authority, fs::Dir};
use nomifun_common::AppError;

use crate::path_safety::{
    has_traversal, is_unsafe_path_segment, reject_workspace_owner_canonical_path,
    validate_path_for_write_authority, PathAuthority,
};

pub(crate) fn validate_target(path: &Path, root: &Path) -> Result<PathBuf, AppError> {
    if has_traversal(&path.to_string_lossy()) || path.file_name().is_none() {
        return Err(AppError::BadRequest("invalid workspace write path".into()));
    }
    let root = std::fs::canonicalize(root)
        .map_err(|error| AppError::BadRequest(format!("cannot resolve workspace root: {error}")))?;
    let mut missing = Vec::new();
    let mut probe = path;
    let mut target = loop {
        match std::fs::symlink_metadata(probe) {
            Ok(_) => {
                // An existing dangling link must fail, not look like a missing
                // directory that we can safely create underneath.
                let canonical = std::fs::canonicalize(probe).map_err(|error| {
                    AppError::BadRequest(format!("cannot resolve workspace write ancestor: {error}"))
                })?;
                reject_workspace_owner_canonical_path(&root, &canonical)?;
                if !missing.is_empty() && !canonical.is_dir() {
                    return Err(AppError::BadRequest("workspace write parent is not a directory".into()));
                }
                break canonical;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = probe.file_name().ok_or_else(|| {
                    AppError::BadRequest("workspace write path has no existing ancestor".into())
                })?;
                if name.to_str().is_none_or(is_unsafe_path_segment) {
                    return Err(AppError::BadRequest("invalid workspace write path component".into()));
                }
                missing.push(name.to_owned());
                probe = probe.parent().ok_or_else(|| {
                    AppError::BadRequest("workspace write path has no parent".into())
                })?;
            }
            Err(error) => return Err(AppError::BadRequest(format!(
                "cannot inspect workspace write ancestor: {error}"
            ))),
        }
    };
    for name in missing.into_iter().rev() {
        target.push(name);
    }
    reject_workspace_owner_canonical_path(&root, &target)?;
    Ok(target)
}

/// Create only within the bound workspace, relative to pinned directories.
/// Each component is opened without following links; revalidate the returned
/// path before publication using the same authority as existing writes.
pub(crate) fn prepare_parent(path: &Path, root: &Path) -> Result<PathBuf, AppError> {
    let target = validate_target(path, root)?;
    let root = std::fs::canonicalize(root)
        .map_err(|error| AppError::BadRequest(format!("cannot resolve workspace root: {error}")))?;
    let parent = target.parent().ok_or_else(|| AppError::BadRequest("write path has no parent".into()))?;
    let relative = parent.strip_prefix(&root)
        .map_err(|_| AppError::Forbidden("workspace write parent is outside the bound resource".into()))?;
    let mut dir = Dir::open_ambient_dir(&root, ambient_authority())
        .map_err(|error| AppError::BadRequest(format!("cannot open workspace root: {error}")))?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(AppError::BadRequest("invalid workspace write parent".into()));
        };
        dir = match dir.open_dir_nofollow(name) {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Err(error) = dir.create_dir(name)
                    && error.kind() != std::io::ErrorKind::AlreadyExists
                {
                    return Err(AppError::BadRequest(format!("cannot create workspace write parent: {error}")));
                }
                dir.open_dir_nofollow(name).map_err(|error| {
                    AppError::Forbidden(format!("cannot open workspace write parent without following links: {error}"))
                })?
            }
            Err(error) => return Err(AppError::Forbidden(format!(
                "cannot open workspace write parent without following links: {error}"
            ))),
        };
    }
    let canonical = validate_path_for_write_authority(
        &target.to_string_lossy(), &PathAuthority::Workspace(root),
    )?;
    if canonical != target {
        return Err(AppError::Conflict("workspace write target changed identity before publication".into()));
    }
    Ok(canonical)
}
