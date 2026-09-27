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
    pub(crate) fn from_metadata(metadata: &std::fs::Metadata) -> Self {
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
pub(crate) struct MetadataObservation { pub(crate) metadata: std::fs::Metadata, pub(crate) canonical: PathBuf }

pub(crate) fn metadata(path: &Path, authority: &PathAuthority) -> Result<Option<MetadataObservation>, AppError> {
    let io_error = |error| AppError::BadRequest(format!("cannot inspect workspace entry: {error}"));
    #[cfg(windows)]
    {
        let root = crate::windows_read::ReadRoot::for_target(path, authority)?;
        let opened = match root {
            Some(root) => root.open_metadata(path),
            None => crate::windows_directory::open_metadata(None, path).map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound { AppError::NotFound("workspace entry is absent".into()) }
                else { io_error(error) }
            }),
        };
        match opened {
            Ok(file) => {
                let metadata = file.metadata().map_err(io_error)?;
                let canonical = crate::windows_read::final_path(&file)?;
                if !crate::windows_read::metadata_name_matches(path, &canonical)? {
                    return Err(AppError::Conflict("workspace metadata entry changed name".into()));
                }
                Ok(Some(MetadataObservation { metadata, canonical }))
            }
            Err(AppError::NotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = authority;
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                let canonical = if metadata.file_type().is_symlink() { path.to_path_buf() }
                    else { crate::path_safety::validate_path_authority(&path.to_string_lossy(), authority)? };
                Ok(Some(MetadataObservation { metadata, canonical }))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_error(error)),
        }
    }
}

pub(crate) struct Entries {
    #[cfg(windows)]
    path: PathBuf,
    #[cfg(windows)]
    inner: crate::windows_directory::Entries,
    #[cfg(not(windows))]
    inner: std::fs::ReadDir,
}

/// The caller supplies an already resolved canonical directory and the same
/// authority used to resolve it. Windows retains its namespace while reading.
pub(crate) fn read_directory(path: &Path, authority: &PathAuthority) -> Result<Entries, AppError> {
    read_directory_with_hook(path, authority, || {})
}

fn read_directory_with_hook(path: &Path, authority: &PathAuthority, after_open: impl FnOnce()) -> Result<Entries, AppError> {
    let io_error = |error| AppError::BadRequest(format!("cannot enumerate workspace directory: {error}"));
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        let root = crate::windows_read::ReadRoot::for_target(path, authority)?;
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
    #[cfg(not(windows))]
    {
        let _ = authority;
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
        #[cfg(not(windows))]
        {
            self.inner.next().map(|entry| entry.map(|entry| Entry {
                path: entry.path(), name: entry.file_name(), hidden: entry.file_name().as_encoded_bytes().starts_with(b"."), kind: entry.file_type().map(|kind|
                    EntryKind { directory: kind.is_dir(), file: kind.is_file(), link: kind.is_symlink() }),
            }))
        }
    }
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
