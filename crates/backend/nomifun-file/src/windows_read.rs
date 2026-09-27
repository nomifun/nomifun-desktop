//! Open a previously resolved read target relative to the authorized directory.
//! Each component is opened without following a newly introduced reparse point.
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::os::windows::ffi::OsStringExt;
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::AsRawHandle;
use std::path::{Component, Path, PathBuf};

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
use cap_std::fs::{Dir, OpenOptions as CapOpenOptions, OpenOptionsExt as _};
use nomifun_common::AppError;
use same_file::Handle;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    GetFinalPathNameByHandleW, SYNCHRONIZE,
};

use crate::PathAuthority;

pub(crate) struct ReadRoot {
    directory: Dir,
    canonical: PathBuf,
    identity: Handle,
}

fn io_error(error: std::io::Error) -> AppError {
    AppError::BadRequest(format!("Workspace file read failed: {error}"))
}

fn changed() -> AppError {
    AppError::Conflict("FILE_CHANGED_DURING_READ: workspace directory identity changed".into())
}

fn final_path(handle: &impl AsRawHandle) -> Result<PathBuf, AppError> {
    let mut buffer = vec![0_u16; 256];
    loop {
        // The borrowed handle remains live and the writable buffer has the
        // supplied UTF-16 capacity. Flags 0 request the resolved DOS path.
        let length = unsafe { GetFinalPathNameByHandleW(handle.as_raw_handle(),buffer.as_mut_ptr(),buffer.len() as u32,0) } as usize;
        if length == 0 { return Err(io_error(std::io::Error::last_os_error())); }
        if length < buffer.len() { return Ok(PathBuf::from(OsString::from_wide(&buffer[..length]))); }
        if length > 32768 { return Err(AppError::BadRequest("workspace directory path exceeds the native path limit".into())); }
        buffer.resize(length.saturating_add(1),0);
    }
}

fn open_root(path: &Path) -> Result<File, AppError> {
    // Metadata access is sufficient. Do not require directory listing
    // rights merely to read a known, otherwise accessible file.
    let file = OpenOptions::new().read(true)
        .access_mode(FILE_READ_ATTRIBUTES | SYNCHRONIZE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path).map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || final_path(&file)? != path {
        return Err(changed());
    }
    Ok(file)
}

impl ReadRoot {
    pub(crate) fn for_target(target: &Path, authority: &PathAuthority) -> Result<Option<Self>, AppError> {
        let roots = match authority {
            PathAuthority::Unrestricted => return Ok(None),
            PathAuthority::Workspace(root) => std::slice::from_ref(root),
            PathAuthority::Confined(roots) => roots.as_slice(),
        };
        let canonical = roots.iter().filter_map(|root| std::fs::canonicalize(root).ok())
            .filter(|root| target.starts_with(root)).max_by_key(|root| root.components().count())
            .ok_or_else(|| AppError::Forbidden("workspace read target no longer belongs to an authorized root".into()))?;
        if matches!(authority,PathAuthority::Workspace(_)) {
            crate::path_safety::reject_workspace_owner_canonical_path(&canonical,target)?;
        }
        let file = open_root(&canonical)?;
        let identity = Handle::from_file(file.try_clone().map_err(io_error)?).map_err(io_error)?;
        Ok(Some(Self { directory:Dir::from_std_file(file),canonical,identity }))
    }

    pub(crate) fn verify(&self) -> Result<(), AppError> {
        let current = open_root(&self.canonical).map_err(|_| changed())?;
        if Handle::from_file(current).map_err(io_error)? != self.identity
            || final_path(&self.directory)? != self.canonical
        { return Err(changed()); }
        Ok(())
    }

    pub(crate) fn open(&self, canonical: &Path) -> Result<File, AppError> {
        self.open_with_parent_hook(canonical, || {})
    }

    fn open_with_parent_hook(&self, canonical: &Path, mut after_parent: impl FnMut()) -> Result<File, AppError> {
        self.verify()?;
        let relative = canonical.strip_prefix(&self.canonical)
            .map_err(|_| AppError::Forbidden("workspace read escaped its pinned root".into()))?;
        let mut components = relative.components().peekable();
        let mut directory = self.directory.try_clone().map_err(io_error)?;
        while let Some(component) = components.next() {
            let Component::Normal(name) = component else { return Err(changed()); };
            let mut options = CapOpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            if components.peek().is_some() {
                options.access_mode(FILE_READ_ATTRIBUTES | SYNCHRONIZE)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
            }
            let file = directory.open_with(name,&options).map_err(io_error)?.into_std();
            if components.peek().is_none() {
                if final_path(&file)? != canonical { return Err(changed()); }
                return Ok(file);
            }
            let metadata = file.metadata().map_err(io_error)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() { return Err(changed()); }
            directory = Dir::from_std_file(file);
            after_parent();
        }
        Err(AppError::BadRequest("Text source must be a regular file".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_root_ancestor_is_not_adopted_when_opening_the_root() {
        let fixture=tempfile::tempdir().unwrap();
        let parent=fixture.path().join("parent");
        let root=parent.join("workspace");
        let outside=fixture.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(outside.join("workspace")).unwrap();
        let expected=std::fs::canonicalize(&root).unwrap();
        std::fs::rename(&parent,fixture.path().join("retained")).unwrap();
        junction::create(&outside,&parent).unwrap();
        assert!(matches!(open_root(&expected),Err(AppError::Conflict(_))));
    }

    #[test]
    fn a_parent_moved_outside_after_opening_cannot_supply_a_new_read_handle() {
        let fixture=tempfile::tempdir().unwrap();
        let root=fixture.path().join("workspace");
        let parent=root.join("parent");
        let outside=fixture.path().join("outside");
        std::fs::create_dir_all(&parent).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(parent.join("value.txt"),b"inside").unwrap();
        let target=std::fs::canonicalize(parent.join("value.txt")).unwrap();
        let pinned=ReadRoot::for_target(&target,&PathAuthority::Workspace(root)).unwrap().unwrap();
        let result=pinned.open_with_parent_hook(&target,|| {
            std::fs::rename(&parent,outside.join("moved")).unwrap();
            std::fs::write(outside.join("moved/value.txt"),b"secret").unwrap();
        });
        assert!(matches!(result,Err(AppError::Conflict(_))));
        assert_eq!(std::fs::read(outside.join("moved/value.txt")).unwrap(),b"secret");
    }
}
