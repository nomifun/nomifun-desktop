//! Single-level, workspace-scoped directory listing shared by the
//! conversation workspace rail (`GET /api/conversations/{id}/workspace`) and
//! the terminal workspace rail (`GET /api/terminals/{id}/workspace`).
//!
//! The caller resolves the workspace root (a conversation's
//! `extra.workspace`, a terminal's cwd, …); this function takes that root plus
//! a relative path and enumerates exactly one directory level under it,
//! enforcing workspace isolation:
//!
//! - reject `..` parent-traversal components in the relative path;
//! - canonicalize and require the browsed path to stay inside the root, with
//!   an allowance for symlinked sub-directories mounted inside the workspace
//!   (e.g. native skill dirs that point at the builtin skills corpus under the
//!   data-dir);
//! - cap relative depth at [`MAX_DIR_DEPTH`];
//! - optional case-insensitive name `search` filter.
//!
//! Entries are returned directories-first, then case-insensitively
//! alphabetical.

use std::path::{Component, Path};

use nomifun_api_types::WorkspaceEntry;
use nomifun_common::AppError;

/// Maximum relative directory depth that may be browsed under a workspace
/// root. Guards against unbounded recursion when a client walks a deep tree.
pub const MAX_DIR_DEPTH: usize = 10;

/// Enumerate a single directory level under `base`, scoped to `rel`.
///
/// `base` is the (already-resolved) workspace root. `rel` is the
/// workspace-relative path to list (`""` or `"/"` lists the root itself).
/// `search`, when set and non-empty, filters entries to names that contain it
/// case-insensitively.
///
/// Returns the directory's entries (directories first, then case-insensitive
/// alphabetical) or an [`AppError`] describing the isolation/IO failure.
pub fn list_workspace_level(
    base: &Path,
    rel: &str,
    search: Option<&str>,
) -> Result<Vec<WorkspaceEntry>, AppError> {
    let relative_path = rel.trim_start_matches('/');
    let relative_path_obj = Path::new(relative_path);
    if relative_path_obj
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(AppError::BadRequest(
            "Path traversal outside workspace is not allowed".into(),
        ));
    }

    // Resolve the browsed path relative to the workspace root.
    let browse_path = if relative_path.is_empty() {
        base.to_path_buf()
    } else {
        base.join(relative_path_obj)
    };

    // Security: reject direct traversal outside the workspace root, but allow
    // symlinked directories mounted inside the workspace (e.g. native skill
    // dirs that point at the builtin skills corpus under data-dir).
    let canonical_base = base
        .canonicalize()
        .map_err(|e| AppError::Internal(format!("Failed to resolve workspace path: {e}")))?;
    let canonical_browse = browse_path
        .canonicalize()
        .map_err(|_| AppError::NotFound("Directory not found".into()))?;
    if !browse_path.starts_with(base) && !canonical_browse.starts_with(&canonical_base) {
        return Err(AppError::BadRequest(
            "Path traversal outside workspace is not allowed".into(),
        ));
    }

    // Check depth limit.
    let depth = relative_path_obj.components().count();
    if depth > MAX_DIR_DEPTH {
        return Err(AppError::BadRequest(format!(
            "Directory depth exceeds maximum of {MAX_DIR_DEPTH}"
        )));
    }

    let search_lower = search
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase());

    let mut entries = Vec::new();
    let dir_reader = std::fs::read_dir(&canonical_browse)
        .map_err(|e| AppError::Internal(format!("Failed to read directory: {e}")))?;

    for entry in dir_reader {
        let entry = entry.map_err(|e| AppError::Internal(format!("Failed to read directory entry: {e}")))?;
        let name = entry.file_name().to_string_lossy().into_owned();

        // Apply search filter if provided.
        if let Some(ref needle) = search_lower
            && !name.to_lowercase().contains(needle)
        {
            continue;
        }

        let metadata = std::fs::metadata(entry.path())
            .map_err(|e| AppError::Internal(format!("Failed to read entry metadata: {e}")))?;

        let entry_type = if metadata.is_dir() { "directory" } else { "file" };

        entries.push(WorkspaceEntry {
            name,
            entry_type: entry_type.into(),
        });
    }

    // Sort: directories first, then alphabetically (case-insensitive).
    entries.sort_by(|a, b| {
        let type_cmp = a.entry_type.cmp(&b.entry_type);
        if type_cmp == std::cmp::Ordering::Equal {
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        } else {
            type_cmp
        }
    });

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn lists_one_level_with_type() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("a.txt"), "x").unwrap();
        let mut out = list_workspace_level(dir.path(), "", None).unwrap();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name, "a.txt");
        assert_eq!(out[0].entry_type, "file");
        assert_eq!(out[1].name, "sub");
        assert_eq!(out[1].entry_type, "directory");
    }

    #[test]
    fn rejects_parent_traversal() {
        let dir = tempdir().unwrap();
        let err = list_workspace_level(dir.path(), "../", None);
        assert!(err.is_err(), "`..` must be rejected");
    }

    #[test]
    fn search_filters_case_insensitive() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "x").unwrap();
        fs::write(dir.path().join("readme.md"), "x").unwrap();
        let out = list_workspace_level(dir.path(), "", Some("cargo")).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "Cargo.toml");
    }
}
