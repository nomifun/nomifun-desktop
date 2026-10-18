//! Search-only native metadata and retained directory cursors. Extracted from
//! the file owner's existing Unix namespace checks; no ambient recursive walk.
use std::{ffi::{CString, OsString}, fs::File, io, os::{fd::{AsRawFd, FromRawFd}, unix::{ffi::OsStrExt, fs::MetadataExt as _}}, path::{Component, Path, PathBuf}};
use cap_std::fs::{Dir, MetadataExt as _};
use crate::{NativeFileError, NativeDirectoryAccess, NativeDirectoryEntry, NativeEntryKind};

pub(crate) struct NativeUnixAccess;
fn changed() -> NativeFileError { NativeFileError::Changed("FILE_CHANGED_DURING_READ: native directory namespace changed".into()) }
fn io_error(error: io::Error) -> NativeFileError { NativeFileError::io("reading native directory metadata", error) }

fn open_search(parent: Option<&Dir>, path: &Path, readable: bool) -> io::Result<Dir> {
    let name = CString::new(path.as_os_str().as_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "native directory name contains NUL"))?;
    #[cfg(target_os = "macos")] let search = libc::O_SEARCH;
    #[cfg(target_os = "linux")] let search = libc::O_PATH;
    #[cfg(not(any(target_os = "macos", target_os = "linux")))] let search = libc::O_RDONLY;
    let access = if readable { libc::O_RDONLY } else { search };
    // SAFETY: the directory capability and NUL-terminated native name stay
    // live; a successful openat returns one exclusively owned descriptor.
    let fd = unsafe { libc::openat(parent.map_or(libc::AT_FDCWD, AsRawFd::as_raw_fd), name.as_ptr(), access | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    if fd < 0 { return Err(io::Error::last_os_error()); }
    Ok(Dir::from_std_file(unsafe { File::from_raw_fd(fd) }))
}
fn verify(directory: &Dir, path: &Path) -> Result<(), NativeFileError> {
    let opened = directory.dir_metadata().map_err(io_error)?;
    let named = std::fs::metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::PermissionDenied { io_error(error) } else { changed() }
    })?;
    if std::fs::canonicalize(path).map_err(io_error)? != path || opened.dev() != named.dev() || opened.ino() != named.ino() { return Err(changed()); }
    Ok(())
}
struct ReadRoot { directory: Dir, path: PathBuf }
impl ReadRoot {
    fn for_target(target: &Path) -> Result<Self, NativeFileError> {
        let mut path = target.parent().unwrap_or(target);
        loop {
            match std::fs::symlink_metadata(path) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => break,
                Ok(_) => return Err(changed()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => path = path.parent().ok_or_else(changed)?,
                Err(error) => return Err(io_error(error)),
            }
        }
        let root = Self { directory: open_search(None, path, false).map_err(io_error)?, path: path.to_owned() };
        root.verify()?;
        Ok(root)
    }
    fn verify(&self) -> Result<(), NativeFileError> { verify(&self.directory, &self.path) }
    fn parent(&self, target: &Path) -> Result<(Dir, OsString, PathBuf), NativeFileError> {
        self.verify()?;
        let relative = target.strip_prefix(&self.path).map_err(|_| changed())?;
        if relative.as_os_str().is_empty() { return Ok((self.directory.try_clone().map_err(io_error)?, OsString::from("."), self.path.clone())); }
        let mut directory = self.directory.try_clone().map_err(io_error)?;
        let mut parent = self.path.clone();
        for part in relative.parent().unwrap_or(Path::new("")).components() {
            let Component::Normal(name) = part else { return Err(changed()); };
            directory = open_search(Some(&directory), Path::new(name), false).map_err(io_error)?;
            parent.push(name); verify(&directory, &parent)?;
        }
        let name = relative.file_name().ok_or_else(changed)?.to_owned();
        self.verify()?;
        Ok((directory, name, parent))
    }
}
fn kind(metadata: &cap_std::fs::Metadata) -> NativeEntryKind {
    let kind = metadata.file_type();
    if kind.is_symlink() { NativeEntryKind::Link } else if kind.is_dir() { NativeEntryKind::Directory } else if kind.is_file() { NativeEntryKind::File } else { NativeEntryKind::Other }
}
impl NativeDirectoryAccess for NativeUnixAccess {
    fn canonicalize(&self, path: &Path) -> Result<PathBuf, NativeFileError> { std::fs::canonicalize(path).map_err(|error| NativeFileError::io("resolving native instruction path", error)) }
    fn canonicalize_entry(&self, path: &Path) -> Result<PathBuf, NativeFileError> {
        let Some(name) = path.file_name() else { return self.canonicalize(path); };
        let parent = self.canonicalize(path.parent().ok_or_else(changed)?)?;
        Ok(parent.join(name))
    }
    fn metadata(&self, path: &Path) -> Result<Option<NativeEntryKind>, NativeFileError> {
        let root = ReadRoot::for_target(path)?;
        let (parent, name, parent_path) = match root.parent(path) {
            Ok(value) => value,
            Err(NativeFileError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let metadata = match parent.symlink_metadata(&name) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(io_error(error)),
        };
        root.verify()?; verify(&parent, &parent_path)?;
        let current = parent.symlink_metadata(&name).map_err(|_| changed())?;
        if (metadata.dev(), metadata.ino()) != (current.dev(), current.ino()) { return Err(changed()); }
        root.verify()?; verify(&parent, &parent_path)?;
        Ok(Some(kind(&metadata)))
    }
    fn entries(&self, path: &Path) -> Result<Box<dyn Iterator<Item = Result<NativeDirectoryEntry, NativeFileError>>>, NativeFileError> {
        let root = ReadRoot::for_target(path)?;
        let (parent, name, _) = root.parent(path)?;
        let directory = open_search(Some(&parent), Path::new(&name), true).map_err(io_error)?;
        let inner = directory.entries().map_err(io_error)?;
        root.verify()?; verify(&directory, path)?;
        Ok(Box::new(Entries { path: path.to_owned(), root, directory, inner, failed: false }))
    }
}
struct Entries { path: PathBuf, root: ReadRoot, directory: Dir, inner: cap_std::fs::ReadDir, failed: bool }
impl Entries {
    fn verify(&self) -> Result<(), NativeFileError> { self.root.verify().and_then(|()| verify(&self.directory, &self.path)) }
}
impl Iterator for Entries {
    type Item = Result<NativeDirectoryEntry, NativeFileError>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed { return None; }
        if let Err(error) = self.verify() { self.failed = true; return Some(Err(error)); }
        let entry = self.inner.next()?;
        let result = entry.map_err(io_error).and_then(|entry| {
            let file_type = entry.file_type().map_err(io_error)?;
            let kind = if file_type.is_symlink() { NativeEntryKind::Link } else if file_type.is_dir() { NativeEntryKind::Directory } else if file_type.is_file() { NativeEntryKind::File } else { NativeEntryKind::Other };
            Ok(NativeDirectoryEntry { path: self.path.join(entry.file_name()), kind })
        });
        if let Err(error) = self.verify() { self.failed = true; return Some(Err(error)); }
        Some(result)
    }
}
