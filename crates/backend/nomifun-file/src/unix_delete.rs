//! Delete relative to the authorized directory handles. Ambient paths are
//! observations used to reject namespace changes, never deletion authority.

use std::path::{Component, Path, PathBuf};
use std::os::unix::fs::MetadataExt as _;

use cap_std::fs::{Dir, MetadataExt as _};
use nomifun_common::AppError;

use crate::PathAuthority;

fn changed() -> AppError {
    AppError::Conflict("FILE_CHANGED_DURING_DELETE: workspace namespace changed".into())
}

fn open_error(error: std::io::Error) -> AppError {
    crate::path_safety::native_io_error("opening native deletion directory", error)
}

struct PinnedDirectory {
    directory: Dir,
    path: PathBuf,
    identity: (u64, u64),
}

impl PinnedDirectory {
    fn new(directory: Dir, path: PathBuf) -> Result<Self, AppError> {
        let metadata = directory.dir_metadata().map_err(open_error)?;
        let identity = (metadata.dev(), metadata.ino());
        let pinned = Self { directory, path, identity };
        pinned.verify()?;
        Ok(pinned)
    }

    fn verify(&self) -> Result<(), AppError> {
        // The directory handle keeps its inode live, so the observed dev/ino
        // cannot be recycled while this capability remains owned. A stat-only
        // comparison adds no read/listing requirement to ordinary deletion.
        let metadata = std::fs::metadata(&self.path).map_err(|_| changed())?;
        if std::fs::canonicalize(&self.path).map_err(|_| changed())? != self.path
            || (metadata.dev(), metadata.ino()) != self.identity
        {
            return Err(changed());
        }
        Ok(())
    }
}

pub(crate) fn remove_entry(
    target: &Path,
    authority: &PathAuthority,
    before_open: impl FnOnce(),
    after_open: impl FnOnce(),
) -> Result<(), AppError> {
    let root = match authority {
        PathAuthority::Unrestricted => target.parent().map(Path::to_path_buf),
        PathAuthority::Workspace(root) => std::fs::canonicalize(root).ok(),
        PathAuthority::Confined(roots) => roots.iter()
            .filter_map(|root| std::fs::canonicalize(root).ok())
            .filter(|root| target.starts_with(root))
            .max_by_key(|root| root.components().count()),
    }.ok_or_else(|| AppError::Forbidden("deletion target no longer belongs to an authorized root".into()))?;
    if matches!(authority, PathAuthority::Workspace(_)) {
        crate::path_safety::reject_workspace_owner_canonical_path(&root, target)?;
    }
    let relative = target.strip_prefix(&root)
        .map_err(|_| AppError::Forbidden("deletion escaped its authorized root".into()))?;
    let name = relative.file_name()
        .ok_or_else(|| AppError::Forbidden("the authorized root itself cannot be deleted".into()))?;
    let root_directory = crate::unix_directory::open_search(None, &root).map_err(open_error)?;
    let root = PinnedDirectory::new(root_directory, root)?;

    before_open();
    root.verify()?;
    let mut parent = PinnedDirectory::new(root.directory.try_clone().map_err(open_error)?, root.path.clone())?;
    for component in relative.parent().unwrap_or(Path::new("")).components() {
        let Component::Normal(component) = component else { return Err(changed()); };
        let directory = crate::unix_directory::open_search(Some(&parent.directory), Path::new(component))
            .map_err(|_| changed())?;
        parent = PinnedDirectory::new(directory, parent.path.join(component))?;
    }
    let metadata = parent.directory.symlink_metadata(name)
        .map_err(|error| AppError::NotFound(format!("cannot remove '{}': {error}", target.display())))?;
    // The caller resolved symlinks before authorizing the target. A newly
    // introduced final symlink is a different entry and must not be adopted.
    if metadata.is_symlink() && !matches!(authority, PathAuthority::Unrestricted) { return Err(changed()); }

    after_open();
    root.verify()?;
    parent.verify()?;
    let current = parent.directory.symlink_metadata(name).map_err(|_| changed())?;
    if current.is_symlink() != metadata.is_symlink() || current.dev() != metadata.dev() || current.ino() != metadata.ino() {
        return Err(changed());
    }

    if metadata.is_dir() {
        // cap-std recurses from open directories without following symlinks.
        // Replacing an ambient ancestor cannot redirect this operation.
        parent.directory.remove_dir_all(name).map_err(|error| AppError::Internal(format!(
            "{}; recursive removal of '{}' failed: {error}; inspect the remaining tree before retry",
            crate::service::FILE_DELETE_OUTCOME_UNKNOWN, target.display(),
        )))
    } else {
        parent.directory.remove_file(name).map_err(|error| crate::path_safety::native_io_error(format!("removing native file '{}'", target.display()), error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    #[test]
    fn deletion_does_not_require_listing_a_write_and_search_only_parent() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("workspace");
        let parent = root.join("parent");
        fs::create_dir_all(&parent).unwrap();
        fs::write(parent.join("target"), b"remove").unwrap();
        let target = fs::canonicalize(parent.join("target")).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o300)).unwrap();
        let result = remove_entry(&target, &PathAuthority::Workspace(root), || {}, || {});
        // Restore before assertions so even a failed regression has ordinary
        // tempfile cleanup authority. Privileged test hosts can list anyway.
        let listing_was_denied = fs::read_dir(&parent).is_err();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        result.unwrap();
        assert!(!parent.join("target").exists());
        if unsafe { libc::geteuid() } != 0 { assert!(listing_was_denied); }
    }

    #[test]
    fn parent_replacement_never_deletes_outside_the_authorized_root() {
        for directory in [false, true] {
            for replace_after_open in [false, true] {
                let fixture = tempfile::tempdir().unwrap();
                let root = fixture.path().join("workspace");
                let parent = root.join("parent");
                let retained = root.join("retained");
                let outside = fixture.path().join("outside");
                fs::create_dir_all(&parent).unwrap();
                fs::create_dir(&outside).unwrap();
                let relative = if directory { "target/value.txt" } else { "target" };
                for base in [&parent, &outside] {
                    if directory { fs::create_dir(base.join("target")).unwrap(); }
                    fs::write(base.join(relative), b"preserve").unwrap();
                }
                let target = fs::canonicalize(parent.join("target")).unwrap();
                let replace = || {
                    fs::rename(&parent, &retained).unwrap();
                    symlink(&outside, &parent).unwrap();
                };
                let result = remove_entry(&target, &PathAuthority::Workspace(root.clone()),
                    || if !replace_after_open { replace(); },
                    || if replace_after_open { replace(); });
                assert!(result.is_err(), "directory={directory}, after_open={replace_after_open}");
                assert_eq!(fs::read(outside.join(relative)).unwrap(), b"preserve");
                assert_eq!(fs::read(retained.join(relative)).unwrap(), b"preserve");
                fs::remove_file(&parent).unwrap();
                fs::rename(&retained, &parent).unwrap();
                remove_entry(&target, &PathAuthority::Workspace(root), || {}, || {}).unwrap();
                assert!(!parent.join("target").exists());
                assert_eq!(fs::read(outside.join(relative)).unwrap(), b"preserve");
            }
        }
    }

    #[test]
    fn replaced_leaf_is_rejected_and_recursive_links_are_only_unlinked() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("workspace");
        let outside = fixture.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), b"outside").unwrap();
        let target = root.join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("inside"), b"inside").unwrap();
        symlink(&outside, target.join("outside-link")).unwrap();
        let target = fs::canonicalize(target).unwrap();
        let retained = root.join("retained");
        let result = remove_entry(&target, &PathAuthority::Workspace(root.clone()), || {}, || {
            fs::rename(&target, &retained).unwrap();
            symlink(&outside, &target).unwrap();
        });
        assert!(matches!(result, Err(AppError::Conflict(_))));
        assert_eq!(fs::read(outside.join("keep")).unwrap(), b"outside");
        assert_eq!(fs::read(retained.join("inside")).unwrap(), b"inside");
        fs::remove_file(&target).unwrap();
        fs::rename(&retained, &target).unwrap();
        remove_entry(&target, &PathAuthority::Workspace(root), || {}, || {}).unwrap();
        assert!(!target.exists());
        assert_eq!(fs::read(outside.join("keep")).unwrap(), b"outside");
    }
}
