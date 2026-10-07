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
//! - canonicalize and require the browsed path and any followed directory link
//!   to stay inside the root; mounting a link does not grant its target;
//! - cap relative depth at [`MAX_DIR_DEPTH`];
//! - optional case-insensitive name `search` filter.
//!
//! Entries are returned directories-first, then case-insensitively
//! alphabetical.

use std::path::{Path, PathBuf};

use nomifun_api_types::WorkspaceEntry;
use nomifun_common::AppError;

use crate::{PathAuthority, workspace_read_dir};

/// Maximum relative directory depth that may be browsed under a workspace
/// root. Guards against unbounded recursion when a client walks a deep tree.
pub const MAX_DIR_DEPTH: usize = 10;

/// Enumerate a single directory level under `base`, scoped to `rel`.
///
/// `base` is the (already-resolved) workspace root. `rel` is the
/// workspace-relative path to list (`""`, `"."` or `"/"` lists the root itself).
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
    list_workspace_level_with_hook(base, rel, search, || {})
}

fn list_workspace_level_with_hook(
    base: &Path,
    rel: &str,
    search: Option<&str>,
    after_resolve: impl FnOnce(),
) -> Result<Vec<WorkspaceEntry>, AppError> {
    let relative_path_obj = if rel.is_empty() || rel == "." || rel == "/" {
        PathBuf::new()
    } else {
        crate::artifact_store::normalized_workspace_relative(rel, false)?
    };

    let depth = relative_path_obj.components().count();
    if depth > MAX_DIR_DEPTH {
        return Err(AppError::BadRequest(format!(
            "Directory depth exceeds maximum of {MAX_DIR_DEPTH}"
        )));
    }

    // Resolve the browsed path relative to the workspace root.
    let browse_path = if relative_path_obj.as_os_str().is_empty() {
        base.to_path_buf()
    } else {
        base.join(&relative_path_obj)
    };

    // A workspace root that does not exist (e.g. a hung AutoWork task whose
    // workspace was never materialized, or a torn-down temp workspace) is a
    // NotFound (404), NOT an internal server error — otherwise the workspace
    // rail re-polls and every poll logs a spurious 500 (see the crash-report
    // triage: the conversation-#2 "500 storm" was this exact misc: a missing
    // root canonicalize failure surfacing as 500 instead of 404).
    let canonical_base = base.canonicalize().map_err(listing_io_error)?;
    let canonical_browse = browse_path
        .canonicalize()
        .map_err(listing_io_error)?;
    crate::path_safety::reject_workspace_owner_canonical_path(
        &canonical_base,
        &canonical_browse,
    )?;
    let authority = PathAuthority::Workspace(canonical_base.clone());

    let search_lower = search
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase());

    let mut entries = Vec::new();
    after_resolve();
    // Re-open only within the same authority. On Windows this retains the
    // directory and its ancestors while names and types are enumerated.
    let dir_reader = workspace_read_dir::read_directory(&canonical_browse, &authority)?;

    for entry in dir_reader {
        // An interrupted enumeration is not a complete, successful snapshot.
        let entry = entry.map_err(listing_io_error)?;
        let name = entry.file_name().into_string().map_err(|_| {
            AppError::BadRequest("Workspace entry name cannot be represented as UTF-8".into())
        })?;
        if canonical_browse == canonical_base
            && crate::artifact_store::is_workspace_owner_component(
                std::ffi::OsStr::new(&name),
            )
        {
            continue;
        }

        // Apply search filter if provided.
        if let Some(ref needle) = search_lower
            && !name.to_lowercase().contains(needle)
        {
            continue;
        }

        let kind = entry.file_type().map_err(|error| {
            listing_io_error(std::io::Error::new(error.kind(), error.to_string()))
        })?;
        // Ordinary entries use the enumerated type, never an ambient path
        // metadata read that could follow a concurrent replacement. In-root
        // links remain expandable; outside/private/dangling links remain leaves.
        let is_dir = if kind.is_symlink() {
            match entry.path().canonicalize() {
                Ok(target) if crate::path_safety::reject_workspace_owner_canonical_path(
                    &canonical_base, &target,
                ).is_ok() => {
                    workspace_read_dir::metadata(&target, &authority)?
                        .is_some_and(|entry| entry.metadata.is_dir() && !entry.metadata.file_type().is_symlink())
                }
                Ok(_) => false,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => return Err(listing_io_error(error)),
            }
        } else {
            kind.is_dir()
        };

        let entry_type = if is_dir { "directory" } else { "file" };

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

fn listing_io_error(error: std::io::Error) -> AppError {
    match error.kind() {
        std::io::ErrorKind::NotFound => AppError::NotFound("Workspace directory entry not found".into()),
        std::io::ErrorKind::PermissionDenied => AppError::Forbidden("Workspace directory access denied".into()),
        _ => AppError::Internal(format!("Failed to read workspace directory: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[cfg(any(unix, windows))]
    fn listing_fixture() -> tempfile::TempDir {
        let mut builder = tempfile::Builder::new();
        builder.prefix("workspace-listing-");
        match std::env::var_os("NOMIFUN_RELIABILITY_FIXTURE_PARENT") {
            Some(parent) => builder.disable_cleanup(true).tempdir_in(parent).unwrap(),
            None => builder.tempdir().unwrap(),
        }
    }

    #[cfg(any(unix, windows))]
    fn directory_link(target: &Path, link: &Path) {
        #[cfg(windows)]
        junction::create(target, link).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn outside_junction_cannot_supply_workspace_entries() {
        let fixture = listing_fixture();
        let root = fixture.path().join("workspace");
        let outside = fixture.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("outside-only.txt"), b"outside sentinel").unwrap();
        directory_link(&outside, &root.join("alias"));
        let result = list_workspace_level(&root, "alias", None);
        assert_eq!(fs::read(outside.join("outside-only.txt")).unwrap(), b"outside sentinel");
        assert!(matches!(result, Err(AppError::Forbidden(_))),
            "outside directory must be denied; got {result:?}; fixture: {}", fixture.path().display());
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn outside_and_private_links_are_leaves_without_listing_their_targets() {
        let fixture = listing_fixture();
        let root = fixture.path().join("workspace");
        let outside = fixture.path().join("outside");
        let private = root.join(".nomifun");
        fs::create_dir_all(&private).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("outside-only.txt"), b"outside").unwrap();
        fs::write(private.join("receipt"), b"owned").unwrap();
        directory_link(&outside, &root.join("outside-alias"));
        directory_link(&private, &root.join("private-alias"));
        let entries = list_workspace_level(&root, ".", None).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "outside-alias");
        assert_eq!(entries[1].name, "private-alias");
        assert!(entries.iter().all(|entry| entry.entry_type == "file"));
    }

    #[cfg(windows)]
    fn replaced_listing_directory_is_rejected(component: &str) {
        let fixture = listing_fixture();
        let root = fixture.path().join("workspace");
        let selected = root.join("parent/selected");
        let replaced = root.join(component);
        let relative = selected.strip_prefix(&replaced).unwrap();
        let outside = fixture.path().join("outside");
        let outside_selected = outside.join(relative);
        fs::create_dir_all(&selected).unwrap();
        fs::create_dir_all(&outside_selected).unwrap();
        fs::write(selected.join("inside.txt"), b"inside").unwrap();
        fs::write(outside_selected.join("outside-only.txt"), b"outside sentinel").unwrap();
        let result = list_workspace_level_with_hook(&root, "parent/selected", None, || {
            fs::rename(&replaced, fixture.path().join("retained-original")).unwrap();
            junction::create(&outside, &replaced).unwrap();
        });
        assert_eq!(fs::read(outside_selected.join("outside-only.txt")).unwrap(), b"outside sentinel");
        assert!(result.is_err(),
            "replacement after resolution must not return outside entries: {result:?}; fixture: {}", fixture.path().display());
    }

    #[cfg(windows)]
    #[test]
    fn selected_directory_replaced_after_resolution_is_rejected() {
        replaced_listing_directory_is_rejected("parent/selected");
    }

    #[cfg(windows)]
    #[test]
    fn ancestor_replaced_after_resolution_is_rejected() {
        replaced_listing_directory_is_rejected("parent");
    }

    #[cfg(windows)]
    #[test]
    fn root_replaced_after_resolution_is_rejected() {
        replaced_listing_directory_is_rejected("");
    }

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
        // The desktop adapter uses "." for a workspace-root request. Keep
        // the route and filesystem owner in agreement so the rail can hydrate.
        let dot_root = list_workspace_level(dir.path(), ".", None).unwrap();
        assert_eq!(dot_root.iter().map(|entry| entry.name.as_str()).collect::<Vec<_>>(), vec!["sub", "a.txt"]);
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

    #[test]
    fn workspace_owner_directory_is_not_a_user_browsable_entry() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".nomifun").join("artifacts")).unwrap();
        fs::write(
            dir.path().join(".nomifun").join("artifacts").join("receipt"),
            "owned",
        )
        .unwrap();
        fs::write(dir.path().join("visible.txt"), "visible").unwrap();

        let out = list_workspace_level(dir.path(), "", None).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "visible.txt");
        assert!(matches!(
            list_workspace_level(dir.path(), ".nomifun/artifacts", None),
            Err(AppError::NotFound(_))
        ));
        assert!(list_workspace_level(dir.path(), "./.nomifun/artifacts", None).is_err());
        assert!(list_workspace_level(dir.path(), "nested//path", None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_alias_cannot_browse_workspace_owner_namespace() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".nomifun/artifacts")).unwrap();
        fs::write(dir.path().join(".nomifun/artifacts/receipt"), "owned").unwrap();
        std::os::unix::fs::symlink(".nomifun", dir.path().join("alias")).unwrap();
        assert!(matches!(
            list_workspace_level(dir.path(), "alias/artifacts", None),
            Err(AppError::NotFound(_))
        ));
    }

    #[cfg(windows)]
    #[test]
    fn junction_alias_cannot_browse_workspace_owner_namespace() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".nomifun/artifacts")).unwrap();
        fs::write(dir.path().join(".nomifun/artifacts/receipt"), "owned").unwrap();
        junction::create(dir.path().join(".nomifun"), dir.path().join("alias")).unwrap();
        assert!(matches!(
            list_workspace_level(dir.path(), "alias/artifacts", None),
            Err(AppError::NotFound(_))
        ));
    }

    #[test]
    fn missing_workspace_root_is_not_found_not_internal() {
        // A conversation whose workspace dir was never materialized (e.g. a hung
        // AutoWork install task) must not 500 on every poll — a missing root is
        // a 404, not an internal server error.
        let dir = tempdir().unwrap();
        let missing = dir.path().join("never-created");
        let err = list_workspace_level(&missing, "", None).unwrap_err();
        assert!(
            matches!(err, AppError::NotFound(_)),
            "missing workspace root must map to NotFound (404), got {err:?}"
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn dangling_symlink_entry_does_not_fail_the_whole_listing() {
        // A hung installer readily leaves a dangling symlink (target never
        // downloaded). One unstattable entry must NOT hard-fail the request —
        // it is listed (as a plain entry) and the good entries still return.
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("good.txt"), "x").unwrap();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        directory_link(&target, &dir.path().join("broken-link"));
        fs::rename(&target, dir.path().join("retained-target")).unwrap();

        let out = list_workspace_level(dir.path(), "", None).expect("a dangling symlink must not 500 the listing");
        let names: Vec<&str> = out.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"good.txt"), "good entries must still be returned: {names:?}");
        assert!(
            names.contains(&"broken-link"),
            "the dangling symlink itself should still be listed, not drop the request: {names:?}"
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn symlinked_subdir_stays_classified_as_directory() {
        // Only links to directories inside this workspace are expandable.
        let dir = tempdir().unwrap();
        let real = dir.path().join("real-dir");
        fs::create_dir(&real).unwrap();
        fs::write(real.join("inside.txt"), b"inside").unwrap();
        directory_link(&real, &dir.path().join("link-dir"));

        let out = list_workspace_level(dir.path(), "", None).unwrap();
        let link = out.iter().find(|e| e.name == "link-dir").expect("symlinked dir must be listed");
        assert_eq!(
            link.entry_type, "directory",
            "a symlinked sub-directory must remain classified as a directory"
        );
        let children = list_workspace_level(dir.path(), "link-dir", None).unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "inside.txt");
    }
}
