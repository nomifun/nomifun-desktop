//! Agent file creation may create missing parent directories. Validation is
//! separate from creation so an invalid multi-file patch has no side effects.
use std::path::{Component, Path, PathBuf};

#[cfg(not(windows))]
use cap_fs_ext::DirExt as _;
#[cfg(not(windows))]
use cap_std::ambient_authority;
use cap_std::fs::Dir;
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

/// Keep the parent namespace stable until publication and cleanup finish.
pub(crate) struct PreparedParent {
    path: PathBuf,
    // This empty, delete-on-close file keeps the directory nonempty, so it
    // cannot acquire a name-surrogate reparse point during publication.
    #[cfg(windows)]
    _guard: std::fs::File,
    #[cfg(windows)]
    _directories: Vec<Dir>,
}

impl PreparedParent {
    pub(crate) fn as_path(&self) -> &Path { &self.path }
}

/// Create only within the bound workspace, relative to pinned directories.
pub(crate) fn prepare_parent(path: &Path, root: &Path) -> Result<PreparedParent, AppError> {
    let target = validate_target(path, root)?;
    let root = std::fs::canonicalize(root)
        .map_err(|error| AppError::BadRequest(format!("cannot resolve workspace root: {error}")))?;
    let parent = target.parent().ok_or_else(|| AppError::BadRequest("write path has no parent".into()))?;
    let relative = parent.strip_prefix(&root)
        .map_err(|_| AppError::Forbidden("workspace write parent is outside the bound resource".into()))?;
    #[cfg(not(windows))]
    let mut dir = Dir::open_ambient_dir(&root, ambient_authority())
        .map_err(|error| AppError::BadRequest(format!("cannot open workspace root: {error}")))?;
    #[cfg(windows)]
    let mut dir = open_windows_root(&root)?;
    #[cfg(windows)]
    let mut directories = vec![dir.try_clone().map_err(parent_io_error)?];
    #[cfg(windows)]
    let mut resolved_parent = root.clone();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(AppError::BadRequest("invalid workspace write parent".into()));
        };
        #[cfg(not(windows))]
        let open_child = |directory: &Dir| directory.open_dir_nofollow(name);
        #[cfg(windows)]
        let open_child = |directory: &Dir| open_windows_child(directory,name);
        dir = match open_child(&dir) {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                #[cfg(windows)]
                let creation_parent = pin_windows_directory(
                    &dir, &resolved_parent,
                    windows_sys::Win32::Storage::FileSystem::FILE_ADD_SUBDIRECTORY,
                )?;
                #[cfg(not(windows))]
                let creation_parent = &dir;
                if let Err(error) = creation_parent.create_dir(name)
                    && error.kind() != std::io::ErrorKind::AlreadyExists
                {
                    return Err(AppError::BadRequest(format!("cannot create workspace write parent: {error}")));
                }
                open_child(&creation_parent).map_err(|error| {
                    AppError::Forbidden(format!("cannot open workspace write parent without following links: {error}"))
                })?
            }
            Err(error) => return Err(AppError::Forbidden(format!(
                "cannot open workspace write parent without following links: {error}"
            ))),
        };
        #[cfg(windows)]
        {
            resolved_parent.push(name);
            verify_windows_parent(&dir,&resolved_parent)?;
            directories.push(dir.try_clone().map_err(parent_io_error)?);
        }
    }
    #[cfg(windows)]
    let guard = {
        let pinned = pin_windows_directory(
            &dir, parent, windows_sys::Win32::Storage::FileSystem::FILE_ADD_FILE,
        )?;
        let guard = create_windows_parent_guard(&pinned)?;
        verify_windows_parent(&pinned, parent)?;
        directories.push(pinned);
        guard
    };
    let canonical = validate_path_for_write_authority(
        &target.to_string_lossy(), &PathAuthority::Workspace(root),
    )?;
    if canonical != target {
        return Err(AppError::Conflict("workspace write target changed identity before publication".into()));
    }
    Ok(PreparedParent {
        path: canonical,
        #[cfg(windows)] _guard: guard,
        #[cfg(windows)] _directories: directories,
    })
}

#[cfg(windows)]
fn parent_io_error(error: std::io::Error) -> AppError {
    AppError::Conflict(format!("cannot pin workspace write directory: {error}"))
}

#[cfg(windows)]
fn open_windows_root(path: &Path) -> Result<Dir, AppError> {
    let file=crate::windows_directory::open(None,path).map_err(parent_io_error)?;
    let metadata=file.metadata().map_err(parent_io_error)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(AppError::Conflict("workspace write root changed to a non-directory or link".into()));
    }
    let directory=Dir::from_std_file(file);
    verify_windows_parent(&directory,path)?;
    Ok(directory)
}

#[cfg(windows)]
fn open_windows_child(directory: &Dir, name: &std::ffi::OsStr) -> std::io::Result<Dir> {
    let file=crate::windows_directory::open(Some(directory),Path::new(name))?;
    let metadata=file.metadata()?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput,"write parent is not a regular directory"));
    }
    Ok(Dir::from_std_file(file))
}

#[cfg(windows)]
fn verify_windows_parent(directory: &Dir, expected: &Path) -> Result<(), AppError> {
    let metadata = directory.dir_metadata().map_err(parent_io_error)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink()
        || crate::windows_read::final_path(directory)? != expected
    {
        return Err(AppError::Conflict("workspace write directory changed identity during preparation".into()));
    }
    Ok(())
}

#[cfg(windows)]
fn pin_windows_directory(directory: &Dir, expected: &Path, access: u32) -> Result<Dir, AppError> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_READ_ATTRIBUTES, SYNCHRONIZE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT};
    // Metadata-only access does not participate in Windows sharing checks.
    // Request the add-file/add-directory right this mutation already needs,
    // without requiring directory listing or traverse access. Compare the
    // acquired object with the existing handle before any mutation.
    let original = same_file::Handle::from_file(directory.try_clone().map_err(parent_io_error)?.into_std_file())
        .map_err(parent_io_error)?;
    let file = std::fs::OpenOptions::new().access_mode(access | FILE_READ_ATTRIBUTES | SYNCHRONIZE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(expected).map_err(parent_io_error)?;
    let acquired = same_file::Handle::from_file(file.try_clone().map_err(parent_io_error)?)
        .map_err(parent_io_error)?;
    if original != acquired {
        return Err(AppError::Conflict("workspace write parent was replaced before creation".into()));
    }
    let pinned = Dir::from_std_file(file);
    verify_windows_parent(&pinned, expected)?;
    Ok(pinned)
}

#[cfg(windows)]
fn create_windows_parent_guard(directory: &Dir) -> Result<std::fs::File, AppError> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    use cap_std::fs::{OpenOptions, OpenOptionsExt};
    use windows_sys::Win32::Storage::FileSystem::{DELETE, FILE_WRITE_DATA, FILE_READ_ATTRIBUTES, SYNCHRONIZE, FILE_FLAG_DELETE_ON_CLOSE};
    let mut options = OpenOptions::new();
    options.write(true).create_new(true).access_mode(FILE_WRITE_DATA | DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE)
        .share_mode(0).custom_flags(FILE_FLAG_DELETE_ON_CLOSE).follow(FollowSymlinks::No);
    // Creating relative to the pinned handle cannot follow a newly installed
    // reparse point on that directory. The guard contains no user data and is
    // removed by the OS even if the operation or process is interrupted.
    directory.open_with(format!(".nomifun-write-{}.guard", nomifun_common::generate_id()), &options)
        .map(|file| file.into_std()).map_err(|error| AppError::Conflict(format!("cannot guard workspace write directory: {error}")))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::windows_test_support::rename_with_posix_semantics;
    use std::{fs, io, ptr};
    use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{FILE_ADD_FILE, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_SHARE_DELETE,
        FILE_WRITE_ATTRIBUTES, SYNCHRONIZE};

    // Unlike junction::create, this attempts to tag an existing directory.
    // Attribute-only access bypasses ordinary read/write sharing checks.
    fn set_junction(path: &Path, target: &Path) -> io::Result<()> {
        use windows_sys::Win32::System::{IO::DeviceIoControl, Ioctl::FSCTL_SET_REPARSE_POINT};
        let file = fs::OpenOptions::new().access_mode(FILE_WRITE_ATTRIBUTES | SYNCHRONIZE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).open(path)?;
        let target = fs::canonicalize(target)?;
        let target = target.to_string_lossy();
        let target = target.strip_prefix(r"\\?\").unwrap();
        let substitute: Vec<u16> = std::ffi::OsStr::new(&format!(r"\??\{target}")).encode_wide().collect();
        let print: Vec<u16> = std::ffi::OsStr::new(target).encode_wide().collect();
        let substitute_len = u16::try_from(substitute.len() * 2).unwrap();
        let print_len = u16::try_from(print.len() * 2).unwrap();
        let mut data = Vec::new();
        data.extend(0xa0000003_u32.to_le_bytes()); // IO_REPARSE_TAG_MOUNT_POINT
        data.extend((8 + substitute_len + 2 + print_len + 2).to_le_bytes());
        data.extend(0_u16.to_le_bytes());
        for field in [0, substitute_len, substitute_len + 2, print_len] { data.extend(field.to_le_bytes()); }
        for unit in substitute.into_iter().chain([0]).chain(print).chain([0]) { data.extend(unit.to_le_bytes()); }
        let mut returned = 0;
        // SAFETY: file and the complete REPARSE_DATA_BUFFER remain live; no
        // output or overlapped buffer is requested. No privilege is enabled.
        let ok = unsafe { DeviceIoControl(file.as_raw_handle(), FSCTL_SET_REPARSE_POINT,
            data.as_ptr().cast(), data.len() as u32, ptr::null_mut(), 0, &mut returned, ptr::null_mut()) };
        if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    #[test]
    fn prepared_guard_prevents_attribute_only_reparse_and_releases_on_drop() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("workspace");
        let parent = root.join("parent");
        let outside = fixture.path().join("outside");
        fs::create_dir_all(&parent).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("sentinel"), b"outside").unwrap();
        let prepared = prepare_parent(&parent.join("new.txt"), &root).unwrap();
        let guard = fs::read_dir(&parent).unwrap().next().unwrap().unwrap().path();
        assert_eq!(fs::metadata(&guard).unwrap().len(), 0);
        assert!(matches!(fs::remove_file(&guard).unwrap_err().raw_os_error(), Some(5 | 32)));
        assert_eq!(set_junction(&parent, &outside).unwrap_err().raw_os_error(), Some(145));
        assert!(matches!(fs::rename(&root, fixture.path().join("moved")).unwrap_err().raw_os_error(), Some(5 | 32)));
        let posix_error = rename_with_posix_semantics(&root, &fixture.path().join("moved"), false).unwrap_err();
        assert!(matches!(posix_error.raw_os_error(), Some(5 | 32)), "{posix_error:?}");
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"outside");
        drop(prepared);
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
        set_junction(&parent, &outside).unwrap(); // Prove the same fixture is otherwise writable.
        junction::delete(&parent).unwrap();
        rename_with_posix_semantics(&root, &fixture.path().join("moved"), false).unwrap();
    }

    #[test]
    fn parent_reparsed_before_guard_creation_cannot_receive_an_outside_guard() {
        let fixture = tempfile::tempdir().unwrap();
        let parent = fixture.path().join("parent");
        let outside = fixture.path().join("outside");
        fs::create_dir(&parent).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("sentinel"), b"outside").unwrap();
        let canonical = fs::canonicalize(&parent).unwrap();
        let directory = open_windows_root(&canonical).unwrap();
        let pinned = pin_windows_directory(&directory, &canonical, FILE_ADD_FILE).unwrap();
        set_junction(&parent, &outside).unwrap();
        assert!(create_windows_parent_guard(&pinned).is_err());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"outside");
        drop(pinned);
        drop(directory);
        junction::delete(&parent).unwrap();
    }

    #[test]
    fn acquiring_creation_access_rejects_a_replaced_parent_object() {
        let fixture = tempfile::tempdir().unwrap();
        let parent = fixture.path().join("parent");
        let moved = fixture.path().join("moved");
        fs::create_dir(&parent).unwrap();
        let canonical = fs::canonicalize(&parent).unwrap();
        let directory = open_windows_root(&canonical).unwrap();
        fs::rename(&parent, &moved).unwrap();
        fs::create_dir(&parent).unwrap();
        assert!(pin_windows_directory(&directory, &canonical, FILE_ADD_FILE).is_err());
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&moved).unwrap().count(), 0);
    }
}
