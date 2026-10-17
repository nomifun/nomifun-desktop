//! Stream directory metadata inside the caller's existing authority.
use std::{ffi::OsString, io, path::{Path, PathBuf}};
use nomifun_common::AppError;
use crate::PathAuthority;

#[derive(Clone, Copy)]
pub(crate) struct EntryKind { directory: bool, file: bool, link: bool }
impl EntryKind {
    pub(crate) fn is_dir(self) -> bool { self.directory }
    pub(crate) fn is_file(self) -> bool { self.file }
    pub(crate) fn is_symlink(self) -> bool { self.link }
    pub(crate) fn from_metadata(metadata: &cap_std::fs::Metadata) -> Self {
        let kind = metadata.file_type();
        Self { directory: kind.is_dir(), file: kind.is_file(), link: kind.is_symlink() }
    }
}

pub(crate) struct Entry { path: PathBuf, name: OsString, kind: Result<EntryKind, io::Error>, hidden: bool }
impl Entry {
    pub(crate) fn path(&self) -> PathBuf { self.path.clone() }
    pub(crate) fn file_name(&self) -> OsString { self.name.clone() }
    pub(crate) fn file_type(&self) -> Result<EntryKind, &io::Error> { self.kind.as_ref().copied() }
    pub(crate) fn is_hidden(&self) -> bool { self.hidden }
}

/// Inspect one entry without following its final link or reading its bytes.
pub(crate) struct MetadataObservation { pub(crate) metadata: cap_std::fs::Metadata, pub(crate) canonical: PathBuf }

pub(crate) fn metadata(path: &Path, authority: &PathAuthority) -> Result<Option<MetadataObservation>, AppError> {
    metadata_with_hooks(path, authority, || {}, || {})
}

fn metadata_with_hooks(path: &Path, authority: &PathAuthority, before_open: impl FnOnce(), after_open: impl FnOnce()) -> Result<Option<MetadataObservation>, AppError> {
    let io_error = |error| crate::native_acquisition::native_read_io_error("inspecting native directory entry", error);
    #[cfg(windows)]
    {
        let root = crate::windows_read::ReadRoot::for_target(path, authority)?;
        before_open();
        let opened = match root {
            Some(root) => root.open_metadata(path),
            None => crate::windows_directory::open_metadata(None, path).map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound { AppError::NotFound("workspace entry is absent".into()) }
                else { io_error(error) }
            }),
        };
        match opened {
            Ok(file) => {
                after_open();
                let metadata = file.metadata().map_err(io_error)?;
                let canonical = crate::windows_read::final_path(&file)?;
                if !crate::windows_read::metadata_name_matches(path, &canonical)? {
                    return Err(AppError::Conflict("workspace metadata entry changed name".into()));
                }
                Ok(Some(MetadataObservation { metadata: cap_std::fs::Metadata::from_just_metadata(metadata), canonical }))
            }
            Err(AppError::NotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }
    #[cfg(unix)]
    {
        let root = UnixReadRoot::for_target(path, authority)?;
        before_open();
        let (parent, name) = match root.parent(path) {
            Ok(value) => value,
            Err(AppError::NotFound(_)) => return Ok(None),
            Err(error) => return Err(error),
        };
        let metadata = match parent.symlink_metadata(&name) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(error)),
        };
        after_open();
        root.verify()?;
        verify_unix_directory(&parent, path.parent().filter(|_| path != root.path).unwrap_or(&root.path))?;
        // Resolve spelling only after obtaining the metadata from the pinned
        // directory. A final symlink is observed as an entry, never followed.
        let canonical = if metadata.file_type().is_symlink() { path.to_path_buf() }
            else { crate::path_safety::validate_native_read_path_authority(&path.to_string_lossy(), authority)? };
        let current = parent.symlink_metadata(&name).map_err(|_| unix_changed())?;
        use cap_std::fs::MetadataExt as _;
        if metadata.dev() != current.dev() || metadata.ino() != current.ino() {
            return Err(unix_changed());
        }
        root.verify()?;
        verify_unix_directory(&parent, path.parent().filter(|_| path != root.path).unwrap_or(&root.path))?;
        Ok(Some(MetadataObservation { metadata, canonical }))
    }
    #[cfg(all(not(windows), not(unix)))]
    {
        before_open();
        after_open();
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                let canonical = if metadata.file_type().is_symlink() { path.to_path_buf() }
                    else { crate::path_safety::validate_native_read_path_authority(&path.to_string_lossy(), authority)? };
                Ok(Some(MetadataObservation { metadata: cap_std::fs::Metadata::from_just_metadata(metadata), canonical }))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_error(error)),
        }
    }
}

pub(crate) struct Entries {
    #[cfg(any(windows, unix))]
    path: PathBuf,
    #[cfg(windows)]
    inner: crate::windows_directory::Entries,
    #[cfg(unix)]
    inner: cap_std::fs::ReadDir,
    #[cfg(unix)]
    root: UnixReadRoot,
    #[cfg(unix)]
    directory: cap_std::fs::Dir,
    #[cfg(unix)]
    failed: bool,
    #[cfg(all(not(windows), not(unix)))]
    inner: std::fs::ReadDir,
}

/// The caller supplies an already resolved canonical directory and the same
/// authority used to resolve it. Native cursors retain the authorized object;
/// Unix also rejects a namespace change before returning more entry names.
pub(crate) fn read_directory(path: &Path, authority: &PathAuthority) -> Result<Entries, AppError> {
    read_directory_with_hook(path, authority, || {})
}

fn read_directory_with_hook(path: &Path, authority: &PathAuthority, after_open: impl FnOnce()) -> Result<Entries, AppError> {
    read_directory_with_hooks(path, authority, || {}, after_open)
}

fn read_directory_with_hooks(path: &Path, authority: &PathAuthority, before_open: impl FnOnce(), after_open: impl FnOnce()) -> Result<Entries, AppError> {
    let io_error = |error| crate::native_acquisition::native_read_io_error("enumerating native directory", error);
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        let root = crate::windows_read::ReadRoot::for_target(path, authority)?;
        before_open();
        let directory = match &root {
            Some(root) => root.open_directory(path)?,
            None => crate::windows_directory::open(None, path).map_err(io_error)?,
        };
        let metadata = directory.metadata().map_err(io_error)?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(AppError::Conflict("workspace directory changed to a link or non-directory".into()));
        }
        let cursor = crate::windows_directory::open_read_cursor(&directory).map_err(io_error)?;
        // The listing handle now pins the directory name and its ancestors.
        // Check the resolved object before returning any native entry names.
        if crate::windows_read::final_path(&directory)? != path {
            return Err(AppError::Conflict("workspace directory moved before enumeration".into()));
        }
        if let Some(root) = root { root.verify()?; }
        after_open();
        Ok(Entries { path: path.to_path_buf(), inner: crate::windows_directory::Entries::new(cursor) })
    }
    #[cfg(unix)]
    {
        let root = UnixReadRoot::for_target(path, authority)?;
        before_open();
        let (parent, name) = root.parent(path)?;
        let directory = unix_readable_directory(&parent, &name).map_err(io_error)?;
        let inner = directory.entries().map_err(io_error)?;
        after_open();
        root.verify()?;
        verify_unix_directory(&directory, path)?;
        Ok(Entries { path: path.to_path_buf(), inner, root, directory, failed: false })
    }
    #[cfg(all(not(windows), not(unix)))]
    {
        let _ = authority;
        before_open();
        let inner = std::fs::read_dir(path).map_err(io_error)?;
        after_open();
        Ok(Entries { inner })
    }
}

impl Iterator for Entries {
    type Item = Result<Entry, io::Error>;
    fn next(&mut self) -> Option<Self::Item> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_HIDDEN};
            self.inner.next().map(|entry| entry.map(|entry| {
                let link = entry.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
                let directory = entry.attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
                let hidden = entry.attributes & FILE_ATTRIBUTE_HIDDEN != 0 || entry.name.as_encoded_bytes().starts_with(b".");
                Entry { path: self.path.join(&entry.name), name: entry.name, hidden,
                    kind: Ok(EntryKind { directory: directory && !link, file: !directory && !link, link }) }
            }))
        }
        #[cfg(unix)]
        {
            if self.failed { return None; }
            let verify = |entries: &Self| {
                entries.root.verify().and_then(|()| verify_unix_directory(&entries.directory, &entries.path))
                    .map_err(|error| io::Error::other(error.to_string()))
            };
            if let Err(error) = verify(self) { self.failed = true; return Some(Err(error)); }
            let entry = self.inner.next()?;
            let result = entry.map(|entry| {
                let name = entry.file_name();
                let hidden = name.as_encoded_bytes().starts_with(b".");
                let kind = entry.file_type().map(|kind| EntryKind {
                    directory: kind.is_dir(), file: kind.is_file(), link: kind.is_symlink(),
                });
                Entry { path: self.path.join(&name), name, hidden, kind }
            });
            if let Err(error) = verify(self) { self.failed = true; return Some(Err(error)); }
            Some(result)
        }
        #[cfg(all(not(windows), not(unix)))]
        {
            self.inner.next().map(|entry| entry.map(|entry| Entry {
                path: entry.path(), name: entry.file_name(), hidden: entry.file_name().as_encoded_bytes().starts_with(b"."), kind: entry.file_type().map(|kind|
                    EntryKind { directory: kind.is_dir(), file: kind.is_file(), link: kind.is_symlink() }),
            }))
        }
    }
}

#[cfg(unix)]
fn unix_changed() -> AppError {
    AppError::Conflict("FILE_CHANGED_DURING_READ: workspace directory namespace changed".into())
}

#[cfg(unix)]
fn verify_unix_directory(directory: &cap_std::fs::Dir, path: &Path) -> Result<(), AppError> {
    crate::unix_directory::verify_named(directory, path).map_err(|_| unix_changed())
}

#[cfg(unix)]
struct UnixReadRoot { directory: cap_std::fs::Dir, path: PathBuf }

#[cfg(unix)]
impl UnixReadRoot {
    // Targets are canonical paths supplied by the existing owner admission.
    // Only the root is opened by ambient spelling; every descendant lookup
    // consumes a search capability and refuses newly introduced links.
    fn for_target(target: &Path, authority: &PathAuthority) -> Result<Self, AppError> {
        let path = match authority {
            PathAuthority::Unrestricted => target.parent().unwrap_or(target).ancestors()
                .find(|parent| parent.is_dir()).ok_or_else(unix_changed)?.to_path_buf(),
            PathAuthority::Workspace(root) => std::fs::canonicalize(root).map_err(|_| unix_changed())?,
            PathAuthority::Confined(roots) => roots.iter().filter_map(|root| std::fs::canonicalize(root).ok())
                .filter(|root| target.starts_with(root)).max_by_key(|root| root.components().count())
                .ok_or_else(|| AppError::Forbidden("workspace read target is outside its authorized roots".into()))?,
        };
        if !target.starts_with(&path) {
            return Err(AppError::Forbidden("workspace read target is outside its bound root".into()));
        }
        if matches!(authority, PathAuthority::Workspace(_)) {
            crate::path_safety::reject_workspace_owner_canonical_path(&path, target)?;
        }
        let directory = crate::unix_directory::open_search(None, &path)
            .map_err(|error| crate::native_acquisition::native_read_io_error("opening native metadata directory", error))?;
        let root = Self { directory, path };
        root.verify()?;
        Ok(root)
    }

    fn verify(&self) -> Result<(), AppError> { verify_unix_directory(&self.directory, &self.path) }

    fn parent(&self, target: &Path) -> Result<(cap_std::fs::Dir, OsString), AppError> {
        self.verify()?;
        let relative = target.strip_prefix(&self.path).map_err(|_| unix_changed())?;
        if relative.as_os_str().is_empty() {
            return Ok((self.directory.try_clone().map_err(|_| unix_changed())?, OsString::from(".")));
        }
        let mut directory = self.directory.try_clone().map_err(|_| unix_changed())?;
        let mut parent = self.path.clone();
        for component in relative.parent().unwrap_or(Path::new("")).components() {
            let std::path::Component::Normal(name) = component else { return Err(unix_changed()); };
            directory = crate::unix_directory::open_search(Some(&directory), Path::new(name))
                .map_err(|error| if error.kind() == io::ErrorKind::NotFound {
                    AppError::NotFound("workspace metadata ancestor is absent".into())
                } else { unix_changed() })?;
            parent.push(name);
            verify_unix_directory(&directory, &parent)?;
        }
        let name = relative.file_name().ok_or_else(unix_changed)?;
        if relative.components().any(|component| !matches!(component, std::path::Component::Normal(_))) {
            return Err(unix_changed());
        }
        self.verify()?;
        Ok((directory, name.to_owned()))
    }
}

#[cfg(unix)]
fn unix_readable_directory(parent: &cap_std::fs::Dir, name: &std::ffi::OsStr) -> io::Result<cap_std::fs::Dir> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "directory name contains NUL"))?;
    // SAFETY: the live parent capability and NUL-terminated single entry name
    // remain borrowed; O_NOFOLLOW rejects a substituted final directory link.
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(),
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    if fd < 0 { return Err(io::Error::last_os_error()); }
    // SAFETY: successful openat gives this owner a new descriptor.
    Ok(cap_std::fs::Dir::from_std_file(unsafe { std::fs::File::from_raw_fd(fd) }))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::{collections::BTreeSet, fs};

    #[test]
    fn listing_keeps_its_directory_and_ancestors_named_until_cursor_drop() {
        for posix in [false, true] {
            let fixture = tempfile::tempdir().unwrap();
            let root = fixture.path().join("workspace");
            let parent = root.join("parent");
            let directory = parent.join("listed");
            let outside = fixture.path().join("outside");
            let moved = outside.join("moved");
            fs::create_dir_all(&directory).unwrap();
            fs::create_dir(&outside).unwrap();
            fs::write(directory.join("inside.txt"), b"inside").unwrap();
            fs::write(outside.join("sentinel"), b"outside").unwrap();
            let canonical = fs::canonicalize(&directory).unwrap();
            let entries = read_directory_with_hook(&canonical, &PathAuthority::Workspace(root), || {
                let rename = if posix { crate::windows_test_support::rename_with_posix_semantics(&parent, &moved, false) }
                    else { fs::rename(&parent, &moved) };
                let error = rename.unwrap_err();
                assert!(matches!(error.raw_os_error(), Some(5 | 32)), "{error}");
            }).unwrap();
            let names = entries.take(1).map(|entry| entry.unwrap().file_name()).collect::<Vec<_>>();
            assert_eq!(names, [OsString::from("inside.txt")]);
            assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"outside");
            crate::windows_test_support::rename_with_posix_semantics(&parent, &moved, false).unwrap();
            assert_eq!(fs::read(moved.join("listed/inside.txt")).unwrap(), b"inside");
        }
    }

    #[test]
    fn listing_pages_preserve_unicode_names_and_link_types() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let mut expected = BTreeSet::new();
        for index in 0..100 {
            let name = format!("{index:03}-空 格🐱-{}.txt", "x".repeat(200));
            fs::write(root.path().join(&name), b"source").unwrap();
            expected.insert(OsString::from(name));
        }
        fs::create_dir(root.path().join("directory")).unwrap();
        junction::create(outside.path(), root.path().join("link")).unwrap();
        let canonical = fs::canonicalize(root.path()).unwrap();
        let entries = read_directory(&canonical, &PathAuthority::Workspace(root.path().to_owned())).unwrap();
        let mut observed = BTreeSet::new();
        let mut directories = Vec::new();
        let mut links = Vec::new();
        for entry in entries {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            if kind.is_symlink() { links.push(entry.file_name()); }
            else if kind.is_dir() { directories.push(entry.file_name()); }
            else { assert!(kind.is_file()); observed.insert(entry.file_name()); }
        }
        assert_eq!(observed, expected);
        assert_eq!(directories, [OsString::from("directory")]);
        assert_eq!(links, [OsString::from("link")]);
    }
}

#[cfg(all(test, unix))]
mod unix_tests {
    use super::*;
    use std::{fs, os::unix::fs::{symlink, PermissionsExt}};

    #[test]
    fn listing_rejects_parent_replacement_before_and_after_open() {
        for after in [false, true] {
            let fixture = tempfile::tempdir().unwrap();
            let root = fixture.path().join("workspace");
            let parent = root.join("parent");
            let directory = parent.join("listed");
            let retained = root.join("retained");
            let outside = fixture.path().join("outside");
            fs::create_dir_all(&directory).unwrap();
            fs::create_dir_all(outside.join("listed")).unwrap();
            fs::write(directory.join("inside.txt"), b"inside").unwrap();
            fs::write(outside.join("listed/outside.txt"), b"outside").unwrap();
            let canonical = fs::canonicalize(&directory).unwrap();
            let replace = || {
                fs::rename(&parent, &retained).unwrap();
                symlink(&outside, &parent).unwrap();
            };
            let result = read_directory_with_hooks(&canonical, &PathAuthority::Workspace(root.clone()),
                || if !after { replace(); }, || if after { replace(); });
            assert!(result.is_err(), "after_open={after}");
            assert_eq!(fs::read(outside.join("listed/outside.txt")).unwrap(), b"outside");
            assert_eq!(fs::read(retained.join("listed/inside.txt")).unwrap(), b"inside");
            fs::remove_file(&parent).unwrap();
            fs::rename(&retained, &parent).unwrap();
            let names = read_directory(&canonical, &PathAuthority::Workspace(root)).unwrap()
                .map(|entry| entry.unwrap().file_name()).collect::<Vec<_>>();
            assert_eq!(names, [OsString::from("inside.txt")]);
        }
    }

    #[test]
    fn listing_cursor_rejects_namespace_changes_between_entries() {
        let fixture = tempfile::tempdir().unwrap();
        let directory = fixture.path().join("workspace");
        let retained = fixture.path().join("retained");
        let outside = fixture.path().join("outside");
        fs::create_dir(&directory).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(directory.join("one.txt"), b"one").unwrap();
        fs::write(directory.join("two.txt"), b"two").unwrap();
        fs::write(outside.join("outside.txt"), b"outside").unwrap();
        let canonical = fs::canonicalize(&directory).unwrap();
        let mut entries = read_directory(&canonical, &PathAuthority::Workspace(directory.clone())).unwrap();
        entries.next().unwrap().unwrap();
        fs::rename(&directory, &retained).unwrap();
        symlink(&outside, &directory).unwrap();
        assert!(entries.next().unwrap().is_err());
        assert!(entries.next().is_none());
        assert_eq!(fs::read(outside.join("outside.txt")).unwrap(), b"outside");
    }

    #[test]
    fn metadata_rejects_parent_replacement_before_and_after_observation() {
        for after in [false, true] {
            let fixture = tempfile::tempdir().unwrap();
            let root = fixture.path().join("workspace");
            let parent = root.join("parent");
            let retained = root.join("retained");
            let outside = fixture.path().join("outside");
            fs::create_dir_all(&parent).unwrap();
            fs::create_dir(&outside).unwrap();
            fs::write(parent.join("value.txt"), b"inside").unwrap();
            fs::write(outside.join("value.txt"), b"different outside bytes").unwrap();
            let target = fs::canonicalize(parent.join("value.txt")).unwrap();
            let replace = || {
                fs::rename(&parent, &retained).unwrap();
                symlink(&outside, &parent).unwrap();
            };
            let result = metadata_with_hooks(&target, &PathAuthority::Workspace(root.clone()),
                || if !after { replace(); }, || if after { replace(); });
            assert!(result.is_err(), "after_observation={after}");
            assert_eq!(fs::read(outside.join("value.txt")).unwrap(), b"different outside bytes");
            fs::remove_file(&parent).unwrap();
            fs::rename(&retained, &parent).unwrap();
            assert_eq!(metadata(&target, &PathAuthority::Workspace(root)).unwrap().unwrap().metadata.len(), 6);
        }
    }

    #[test]
    fn metadata_needs_search_permission_and_never_opens_file_bytes() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("workspace");
        let parent = root.join("parent");
        fs::create_dir_all(&parent).unwrap();
        let target = parent.join("value.txt");
        fs::write(&target, b"metadata only").unwrap();
        symlink("value.txt", parent.join("link")).unwrap();
        let canonical = fs::canonicalize(&target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0)).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o100)).unwrap();
        let result = metadata(&canonical, &PathAuthority::Workspace(root.clone()));
        let link = metadata(&canonical.with_file_name("link"), &PathAuthority::Workspace(root));
        let content_denied = fs::read(&target).is_err();
        let listing_denied = fs::read_dir(&parent).is_err();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(result.unwrap().unwrap().metadata.len(), 13);
        assert!(link.unwrap().unwrap().metadata.file_type().is_symlink());
        if unsafe { libc::geteuid() } != 0 { assert!(content_denied && listing_denied); }
    }
}
