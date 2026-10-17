//! Agent file creation may create missing parent directories. Validation is
//! separate from creation so an invalid multi-file patch has no side effects.
use std::path::{Component, Path, PathBuf};

#[cfg(all(not(windows), not(unix)))]
use cap_fs_ext::DirExt as _;
#[cfg(all(not(windows), not(unix)))]
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use nomifun_common::AppError;

use crate::path_safety::{
    validate_path_for_write_authority, PathAuthority,
};

pub(crate) fn validate_global_target(path: &Path) -> Result<PathBuf, AppError> {
    #[cfg(windows)]
    let normalized = nomi_native_files::windows::normalize_path(path)
        .map_err(|error| crate::path_safety::native_io_error("resolving native Windows write path", error))?;
    #[cfg(windows)]
    let path = normalized.as_path();
    if path.as_os_str().is_empty() || path.to_string_lossy().contains('\0') || path.file_name().is_none() {
        return Err(AppError::BadRequest("invalid native write path".into()));
    }
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    let mut probe = path;
    let mut target = loop {
        match std::fs::symlink_metadata(probe) {
            Ok(_) => {
                // An existing dangling link must fail, not look like a missing
                // directory that we can safely create underneath.
                let canonical = std::fs::canonicalize(probe).map_err(|error| {
                    crate::path_safety::native_io_error("resolving native write ancestor", error)
                })?;
                if !missing.is_empty() && !canonical.is_dir() {
                    return Err(AppError::BadRequest("workspace write parent is not a directory".into()));
                }
                break canonical;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = probe.file_name().ok_or_else(|| {
                    AppError::BadRequest("workspace write path has no existing ancestor".into())
                })?;
                if name.to_str().is_none_or(crate::path_safety::is_invalid_native_name) {
                    return Err(AppError::BadRequest("invalid workspace write path component".into()));
                }
                missing.push(name.to_owned());
                probe = probe.parent().ok_or_else(|| {
                    AppError::BadRequest("workspace write path has no parent".into())
                })?;
            }
            Err(error) => return Err(crate::path_safety::native_io_error("inspecting native write ancestor", error)),
        }
    };
    for name in missing.into_iter().rev() {
        #[cfg(windows)]
        { target = nomi_native_files::windows::join_component(&target, &name)
            .map_err(|error| crate::path_safety::native_io_error("joining native Windows write path", error))?; }
        #[cfg(not(windows))]
        target.push(name);
    }
    Ok(target)
}

/// Keep the parent namespace stable until publication and cleanup finish.
pub(crate) struct PreparedParent {
    path: PathBuf,
    #[cfg(unix)]
    directory: Dir,
    #[cfg(unix)]
    root_directory: Dir,
    #[cfg(unix)]
    root_path: PathBuf,
    // This empty, delete-on-close file keeps the directory nonempty, so it
    // cannot acquire a name-surrogate reparse point during publication.
    #[cfg(windows)]
    _guard: std::fs::File,
    #[cfg(windows)]
    _directories: Vec<Dir>,
}

impl PreparedParent {
    pub(crate) fn as_path(&self) -> &Path { &self.path }

    #[cfg(unix)]
    pub(crate) fn directory(&self) -> &Dir { &self.directory }

    #[cfg(unix)]
    pub(crate) fn verify(&self) -> Result<(), AppError> {
        let parent = self.path.parent().ok_or_else(|| AppError::BadRequest("write path has no parent".into()))?;
        crate::unix_directory::verify_named(&self.root_directory, &self.root_path)
            .and_then(|()| crate::unix_directory::verify_named(&self.directory, parent))
            .map_err(|_| AppError::Conflict("workspace write parent changed identity".into()))
    }
}

/// Pin the closest existing real parent, creating missing children relative
/// to its handle. This is a publication guard, not an allowlist or sandbox.
pub(crate) fn prepare_global_parent(path: &Path) -> Result<PreparedParent, AppError> {
    let target = validate_global_target(path)?;
    let parent = target.parent().ok_or_else(|| AppError::BadRequest("write path has no parent".into()))?;
    let root = parent.ancestors().find(|path| path.is_dir())
        .ok_or_else(|| AppError::BadRequest("write path has no existing directory ancestor".into()))?.to_owned();
    prepare_target_parent(target, root)
}

#[cfg(windows)]
pub(crate) fn prepare_global_rename_parent(path: &Path, directory: bool) -> Result<Dir, AppError> {
    let target = validate_global_target(path)?;
    let parent = target.parent().ok_or_else(|| AppError::BadRequest("rename path has no parent".into()))?;
    // Native rename does not create a missing destination directory. Pin only
    // this existing parent; write preparation's ancestor creation is separate.
    let existing = std::fs::canonicalize(parent).map_err(|error| crate::path_safety::native_io_error("resolving native rename parent", error))?;
    if !windows_parent_path_matches(&existing, parent) { return Err(AppError::Conflict("native rename parent changed identity before preparation".into())); }
    let original = open_windows_root(&existing)?;
    let original_identity = same_file::Handle::from_file(original.try_clone().map_err(parent_io_error)?.into_std_file()).map_err(parent_io_error)?;
    let file = nomi_native_files::windows::open_rename_parent(&existing, directory).map_err(parent_io_error)?;
    let acquired_identity = same_file::Handle::from_file(file.try_clone().map_err(parent_io_error)?).map_err(parent_io_error)?;
    if original_identity != acquired_identity { return Err(AppError::Conflict("native rename parent was replaced before pinning".into())); }
    let pinned = Dir::from_std_file(file); verify_windows_parent(&pinned, &existing)?; Ok(pinned)
}

fn prepare_target_parent(target: PathBuf, root: PathBuf) -> Result<PreparedParent, AppError> {
    prepare_target_parent_from(target, root, None)
}

#[cfg(windows)]
pub(crate) fn prepare_pinned_parent(target: PathBuf, root: PathBuf, directory: Dir) -> Result<PreparedParent, AppError> {
    prepare_target_parent_from(target, root, Some(directory))
}

fn prepare_target_parent_from(target: PathBuf, root: PathBuf, retained: Option<Dir>) -> Result<PreparedParent, AppError> {
    let parent = target.parent().ok_or_else(|| AppError::BadRequest("write path has no parent".into()))?;
    let relative = parent.strip_prefix(&root)
        .map_err(|_| AppError::Conflict("native write parent no longer matches its prepared ancestor".into()))?;
    #[cfg(all(not(windows), not(unix)))]
    let mut dir = Dir::open_ambient_dir(&root, ambient_authority())
        .map_err(|error| crate::path_safety::native_io_error("opening native write parent", error))?;
    #[cfg(unix)]
    let mut dir = crate::unix_directory::open_search(None, &root)
        .map_err(|error| crate::path_safety::native_io_error("opening native write parent", error))?;
    #[cfg(not(windows))]
    let _ = retained;
    #[cfg(unix)]
    let root_directory = dir.try_clone().map_err(|error| AppError::BadRequest(format!("cannot retain workspace root: {error}")))?;
    #[cfg(unix)]
    crate::unix_directory::verify_named(&root_directory, &root)
        .map_err(|_| AppError::Conflict("workspace root changed identity during write preparation".into()))?;
    #[cfg(windows)]
    let mut dir = match retained { Some(directory) => { verify_windows_parent(&directory, &root)?; directory }, None => open_windows_root(&root)? };
    #[cfg(windows)]
    let mut directories = vec![dir.try_clone().map_err(parent_io_error)?];
    #[cfg(windows)]
    let mut resolved_parent = root.clone();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(AppError::BadRequest("invalid workspace write parent".into()));
        };
        #[cfg(all(not(windows), not(unix)))]
        let open_child = |directory: &Dir| directory.open_dir_nofollow(name);
        #[cfg(unix)]
        let open_child = |directory: &Dir| crate::unix_directory::open_search(Some(directory), Path::new(name));
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
                    return Err(crate::path_safety::native_io_error("creating native write parent", error));
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
        &target.to_string_lossy(), &PathAuthority::Unrestricted,
    )?;
    if canonical != target {
        return Err(AppError::Conflict("workspace write target changed identity before publication".into()));
    }
    let prepared = PreparedParent {
        path: canonical,
        #[cfg(unix)] directory: dir,
        #[cfg(unix)] root_directory,
        #[cfg(unix)] root_path: root,
        #[cfg(windows)] _guard: guard,
        #[cfg(windows)] _directories: directories,
    };
    #[cfg(unix)] prepared.verify()?;
    Ok(prepared)
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
pub(crate) fn windows_parent_path_matches(actual: &Path, expected: &Path) -> bool {
    if actual == expected { return true; }
    fn unc_root(path: &Path) -> Option<(&std::ffi::OsStr, &std::ffi::OsStr)> {
        let Component::Prefix(prefix) = path.components().next()? else { return None; };
        let std::path::Prefix::VerbatimUNC(server, share) = prefix.kind() else { return None; };
        if server.is_empty() || share.is_empty() { return None; }
        let mut root = prefix.as_os_str().to_owned();
        if path.as_os_str() != root { root.push("\\"); if path.as_os_str() != root { return None; } }
        Some((server, share))
    }
    // GetFinalPathName/std canonicalize omit a share root's trailing separator;
    // Path::parent adds it, giving VerbatimUNC a distinct RootDir component.
    // Accept only those two exact root spellings, never another host/share,
    // child, traversal, case fold or general path alias. Object checks remain.
    matches!((unc_root(actual), unc_root(expected)), (Some(left), Some(right)) if left == right)
}

#[cfg(windows)]
fn verify_windows_parent(directory: &Dir, expected: &Path) -> Result<(), AppError> {
    let metadata = directory.dir_metadata().map_err(parent_io_error)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink()
        || !windows_parent_path_matches(&crate::windows_read::final_path(directory)?, expected)
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

    #[test]
    fn verbatim_unc_parent_matches_only_its_exact_share_root_separator_spelling() {
        let root = Path::new(r"\\?\UNC\localhost\NomiFun_GlobalOS_fixture");
        let target = root.join("sample.txt"); let expected = target.parent().unwrap();
        assert_ne!(root, expected, "the real SMB failure has a distinct Rust RootDir component");
        assert!(windows_parent_path_matches(root, expected));
        assert!(windows_parent_path_matches(expected, root));
        assert!(windows_parent_path_matches(root, root));
        for other in [r"\\?\UNC\other-host\NomiFun_GlobalOS_fixture\", r"\\?\UNC\localhost\other-share\",
            r"\\?\UNC\LOCALHOST\NomiFun_GlobalOS_fixture\", r"\\?\UNC\localhost\NOMIFUN_GlobalOS_fixture\",
            r"\\?\UNC\localhost\NomiFun_GlobalOS_fixture\child", r"\\?\UNC\localhost\NomiFun_GlobalOS_fixture\.",
            r"\\?\UNC\localhost\NomiFun_GlobalOS_fixture\..", r"\\?\UNC\localhost\NomiFun_GlobalOS_fixture\\"] {
            assert!(!windows_parent_path_matches(root, Path::new(other)), "{other:?}");
        }
        assert!(!windows_parent_path_matches(Path::new(r"\\?\C:"), Path::new(r"\\?\C:\")));
        assert!(!windows_parent_path_matches(Path::new(r"\\?\C:\"), root));
        let child = root.join("child"); assert!(windows_parent_path_matches(&child, &child));
        assert!(!windows_parent_path_matches(&child, &root.join("other-child")));
    }

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
        let prepared = prepare_global_parent(&parent.join("new.txt")).unwrap();
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

    #[test]
    fn relative_rename_parent_reparse_is_rejected_without_an_external_entry() {
        let fixture = tempfile::tempdir().unwrap(); let parent = fixture.path().join("parent"); let outside = fixture.path().join("outside");
        fs::create_dir(&parent).unwrap(); fs::create_dir(&outside).unwrap();
        let source = fixture.path().join("source.txt"); fs::write(&source, b"retained").unwrap();
        let source_path = fs::canonicalize(&source).unwrap();
        let source_file = crate::windows_directory::open_delete(None, &source_path).unwrap();
        let pinned = prepare_global_rename_parent(&parent.join("target.txt"), false).unwrap();
        assert_eq!(fs::read_dir(&parent).unwrap().count(), 0, "rename creates no temporary guard");
        set_junction(&parent, &outside).unwrap();
        let result = crate::windows_cleanup::rename_relative_no_replace(&source_file, &pinned, std::ffi::OsStr::new("target.txt"));
        assert_eq!(result.unwrap_err().raw_os_error(), Some(1921));
        assert_eq!(fs::read(&source).unwrap(), b"retained"); assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        drop(pinned); drop(source_file); junction::delete(&parent).unwrap();
    }
}
