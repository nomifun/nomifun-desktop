//! Native relative opens and retained cursors shared by both I/O identities.
use std::{ffi::OsString, fs::File, io, mem, path::{Component, Path}, ptr};
use std::os::windows::{ffi::{OsStrExt, OsStringExt}, io::{AsRawHandle, FromRawHandle}};

use cap_std::fs::Dir;
use windows_sys::Wdk::{Foundation::OBJECT_ATTRIBUTES, Storage::FileSystem::{
    NtOpenFile, FILE_DIRECTORY_FILE, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,
}};
use windows_sys::Win32::{Foundation::{ERROR_NO_MORE_FILES, HANDLE, RtlNtStatusToDosError, UNICODE_STRING},
    Storage::FileSystem::{DELETE, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, SYNCHRONIZE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_SHARE_DELETE,
        FILE_ID_BOTH_DIR_INFO, FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo, GetFileInformationByHandleEx},
    System::IO::IO_STATUS_BLOCK};

/// Apply the OS's DOS path normalization once, preserving explicit verbatim
/// filesystem names. GLOBALROOT aliases are accepted when Mount Manager
/// identifies their actual filesystem volume, never by treating every device
/// namespace as a regular file or by enumerating allowed filesystem roots.
pub fn normalize_path(path: &Path) -> io::Result<std::path::PathBuf> {
    use std::path::Prefix;
    if path.as_os_str().encode_wide().any(|unit| unit == 0) { return Err(io::Error::new(io::ErrorKind::InvalidInput, "native file path contains NUL")); }
    let mut normalized = std::path::absolute(path)?;
    let filesystem_path = matches!(normalized.components().next(), Some(Component::Prefix(prefix))
        if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::UNC(_, _) | Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(_, _))
            || matches!(prefix.kind(), Prefix::Verbatim(value) if is_volume_guid(value))
            || matches!(prefix.kind(), Prefix::DeviceNS(value) if is_volume_guid(value) || value.to_str().is_some_and(|value| {
                let bytes = value.as_bytes(); bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
            })));
    if !filesystem_path && !is_filesystem_globalroot(&normalized)? {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "file operation requires a disk or UNC filesystem path"));
    }
    // The explicit data-stream type is an OS alias, including `file::$DATA`
    // for the default stream. Resolve that spelling before an absent stream
    // is staged so a successful publication has one canonical native name.
    if let Some(name) = normalized.file_name().and_then(|name| name.to_str())
        && let Some((before, stream_type)) = name.rsplit_once(':')
        && stream_type.eq_ignore_ascii_case("$DATA") && before.contains(':') {
        let name = before.strip_suffix(':').unwrap_or(before);
        normalized = join_component(normalized.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "stream path has no parent"))?, std::ffi::OsStr::new(name))?;
    }
    Ok(normalized)
}

fn is_filesystem_globalroot(path: &Path) -> io::Result<bool> {
    use std::path::Prefix;
    use windows_sys::Win32::Storage::FileSystem::GetVolumeNameForVolumeMountPointW;
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::Prefix(prefix))
        if matches!(prefix.kind(), Prefix::Verbatim(value) | Prefix::DeviceNS(value) if value.to_str().is_some_and(|value| value.eq_ignore_ascii_case("GLOBALROOT")))) {
        return Ok(false);
    }
    if !matches!(components.next(), Some(Component::RootDir))
        || !matches!(components.next(), Some(Component::Normal(value)) if value.to_str().is_some_and(|value| value.eq_ignore_ascii_case("Device"))) {
        return Ok(false);
    }
    let Some(Component::Normal(device)) = components.next() else { return Ok(false); };
    let root = join_component(Path::new(r"\\?\GLOBALROOT\Device"), device)?;
    let mut root: Vec<u16> = root.as_os_str().encode_wide().collect(); root.extend([92, 0]);
    let mut volume = [0_u16; 128];
    // This asks Mount Manager for this exact alias's registered filesystem
    // volume; it does not open/read a device's content or confer ACL access.
    // SAFETY: the metadata-query input and bounded output buffers remain live.
    if unsafe { GetVolumeNameForVolumeMountPointW(root.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let length = volume.iter().position(|unit| *unit == 0).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid native filesystem volume identity"))?;
    let volume = std::path::PathBuf::from(OsString::from_wide(&volume[..length]));
    Ok(matches!(volume.components().next(), Some(Component::Prefix(prefix))
        if matches!(prefix.kind(), Prefix::Verbatim(value) if is_volume_guid(value))))
}

fn is_volume_guid(value: &std::ffi::OsStr) -> bool {
    let Some(value) = value.to_str() else { return false; };
    let bytes = value.as_bytes();
    if bytes.len() != 44 || !bytes[..7].eq_ignore_ascii_case(b"Volume{") || bytes.last() != Some(&b'}') { return false; }
    bytes[7..43].iter().copied().enumerate().all(|(index, byte)| {
        if matches!(index, 8 | 13 | 18 | 23) { byte == b'-' } else { byte.is_ascii_hexdigit() }
    })
}

/// Append an already separated native name without interpreting `a:stream`
/// as a drive-relative Path. NT relative opens give it stream semantics.
pub fn join_component(parent: &Path, name: &std::ffi::OsStr) -> io::Result<std::path::PathBuf> {
    let value: Vec<u16> = name.encode_wide().collect();
    if value.is_empty() || name == "." || name == ".." || value.iter().any(|unit| matches!(*unit, 0 | 47 | 92)) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "expected a single native file name"));
    }
    let mut joined = parent.as_os_str().to_owned();
    if !joined.encode_wide().last().is_some_and(|unit| matches!(unit, 47 | 92)) { joined.push("\\"); }
    joined.push(name);
    Ok(joined.into())
}

/// Resolve an entry without following its final reparse point. Opening the
/// original normalized DOS spelling preserves Win32 aliases; GetFinalPath
/// then records the on-disk entry name, including an explicit stream suffix.
pub fn canonical_entry(path: &Path) -> io::Result<std::path::PathBuf> {
    use std::os::windows::fs::OpenOptionsExt as _;
    use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_TYPE_DISK, GetFileType};
    let path = normalize_path(path)?;
    let file = std::fs::OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).open(path)?;
    // SAFETY: this metadata-only handle is live and no device data is read.
    if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "entry is not a disk filesystem object"));
    }
    final_path(&file)
}

/// Open one child, or an already canonical absolute root, without following
/// the final reparse point. Callers check its type and resolved handle path.
pub fn open(parent: Option<&Dir>, path: &Path) -> io::Result<File> {
    open_with(parent, path, FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, FILE_DIRECTORY_FILE)
}

/// Open the entry itself for deletion, retaining its name until handle unlink.
pub fn open_delete(parent: Option<&Dir>, path: &Path) -> io::Result<File> {
    open_with(parent, path, DELETE | FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE, 0)
}

pub fn open_metadata(parent: Option<&Dir>, path: &Path) -> io::Result<File> {
    open_with(parent, path, FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, 0)
}

/// Pin an existing native rename parent using only the right needed to add
/// that entry kind. This avoids CreateFile's implicit synchronous access and
/// creates no temporary child requiring extra DELETE access.
pub fn open_rename_parent(path: &Path, directory: bool) -> io::Result<File> {
    use windows_sys::Win32::Storage::FileSystem::{FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY};
    open_with(None, path, FILE_READ_ATTRIBUTES | if directory { FILE_ADD_SUBDIRECTORY } else { FILE_ADD_FILE },
        FILE_SHARE_READ | FILE_SHARE_WRITE, FILE_DIRECTORY_FILE)
}

/// An empty native relative name reopens the retained directory itself. It
/// does not resolve the directory's possibly changed name in its parent.
pub fn open_cursor(directory: &File) -> io::Result<File> {
    open_native(directory.as_raw_handle(), &mut [], FILE_LIST_DIRECTORY | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT)
}

/// Listing access participates in sharing checks: retain the directory name
/// while a read-only caller consumes the cursor.
pub fn open_read_cursor(directory: &File) -> io::Result<File> {
    open_native(directory.as_raw_handle(), &mut [], FILE_LIST_DIRECTORY | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE, FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT)
}

fn open_with(parent: Option<&Dir>, path: &Path, access: u32, sharing: u32, options: u32) -> io::Result<File> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "invalid workspace directory path");
    let mut name: Vec<u16> = path.as_os_str().encode_wide().collect();
    if parent.is_some() {
        if name.is_empty() || path.as_os_str() == "." || path.as_os_str() == ".."
            || name.iter().any(|unit| matches!(*unit, 0 | 47 | 92)) {
            return Err(invalid());
        }
    } else {
        // canonicalize returns the extended DOS spelling. NtOpenFile uses
        // the equivalent native DOS-device prefix, including for UNC roots.
        if !name.starts_with(&[92, 92, 63, 92]) { return Err(invalid()); }
        name[..4].copy_from_slice(&[92, 63, 63, 92]);
    }
    if name.contains(&0) { return Err(invalid()); }
    open_native(parent.map_or(ptr::null_mut(), |directory| directory.as_raw_handle()), &mut name, access, sharing, options)
}

fn open_native(parent: HANDLE, name: &mut [u16], access: u32, sharing: u32, options: u32) -> io::Result<File> {
    let length = u16::try_from(name.len() * 2).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "native directory name is too long"))?;
    let mut unicode = UNICODE_STRING { Length: length, MaximumLength: length, Buffer: name.as_mut_ptr() };
    let attributes = OBJECT_ATTRIBUTES {
        Length: mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent,
        ObjectName: &mut unicode,
        ..Default::default()
    };
    let mut status_block = IO_STATUS_BLOCK::default();
    let mut handle = ptr::null_mut();
    // Unlike CreateFile/cap-std, request only each operation's access mask.
    // Metadata path resolution does not require listing/traverse/SYNCHRONIZE;
    // only the enumeration cursor asks for LIST_DIRECTORY and synchronous IO.
    // No backup privilege is requested or enabled.
    // SAFETY: all structures and the UTF-16 buffer remain live for the open;
    // NtOpenFile returns a new owned handle on success.
    let status = unsafe { NtOpenFile(&mut handle, access, &attributes,
        &mut status_block, sharing, options | FILE_OPEN_REPARSE_POINT) };
    if status < 0 {
        return Err(io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32));
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}

pub struct Entry { pub name: OsString, pub attributes: u32 }

/// A bounded native directory cursor. cap-std's Windows entries() reopens a
/// path; this cursor enumerates the retained directory object itself.
pub struct Entries { file: File, buffer: [u64; 512], offset: Option<usize>, first: bool, done: bool }

impl Entries {
    pub fn new(file: File) -> Self {
        Self { file, buffer: [0;512], offset: None, first: true, done: false }
    }

    fn next_entry(&mut self) -> io::Result<Option<Entry>> {
        loop {
            if self.done { return Ok(None); }
            if self.offset.is_none() {
                self.buffer.fill(0);
                let class = if self.first { FileIdBothDirectoryRestartInfo } else { FileIdBothDirectoryInfo };
                // SAFETY: aligned, initialized storage remains live until this
                // synchronous query completes; its full byte size is provided.
                let ok = unsafe { GetFileInformationByHandleEx(self.file.as_raw_handle(), class,
                    self.buffer.as_mut_ptr().cast(), mem::size_of_val(&self.buffer) as u32) };
                if ok == 0 {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) { self.done = true; return Ok(None); }
                    return Err(error);
                }
                self.first = false;
                self.offset = Some(0);
            }
            let at = self.offset.expect("directory query provided an entry");
            // SAFETY: view exactly the initialized allocation as bytes. All
            // native offsets and variable name lengths are checked below.
            let bytes = unsafe { std::slice::from_raw_parts(self.buffer.as_ptr().cast::<u8>(), mem::size_of_val(&self.buffer)) };
            let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid native directory record");
            let field = |offset: usize| -> io::Result<usize> {
                Ok(u32::from_le_bytes(bytes.get(at + offset..at + offset + 4).ok_or_else(invalid)?.try_into().map_err(|_| invalid())?) as usize)
            };
            let next = field(mem::offset_of!(FILE_ID_BOTH_DIR_INFO, NextEntryOffset))?;
            let length = field(mem::offset_of!(FILE_ID_BOTH_DIR_INFO, FileNameLength))?;
            let start = at + mem::offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
            if length == 0 || length % 2 != 0 || length > bytes.len().saturating_sub(start) { return Err(invalid()); }
            let end = start + length;
            if next != 0 && (next < end - at || next % 8 != 0 || next >= bytes.len() - at) { return Err(invalid()); }
            self.offset = (next != 0).then_some(at + next);
            let name = OsString::from_wide(&bytes[start..end].chunks_exact(2).map(|unit| u16::from_le_bytes([unit[0],unit[1]])).collect::<Vec<_>>());
            if name != "." && name != ".." {
                return Ok(Some(Entry { name, attributes: field(mem::offset_of!(FILE_ID_BOTH_DIR_INFO, FileAttributes))? as u32 }));
            }
        }
    }
}

impl Iterator for Entries {
    type Item = io::Result<Entry>;
    fn next(&mut self) -> Option<Self::Item> {
        match self.next_entry() {
            Ok(name) => name.map(Ok),
            Err(error) => { self.done = true; Some(Err(error)) },
        }
    }
}

/// Retrieve the resolved name from the retained object, never from a second
/// pathname lookup. Stream suffixes and extended DOS/UNC spelling are kept.
pub fn final_path(file: &impl AsRawHandle) -> io::Result<std::path::PathBuf> {
    use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
    let mut buffer = vec![0_u16; 256];
    loop {
        // SAFETY: the borrowed handle and output buffer remain live.
        let length = unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), buffer.as_mut_ptr(), buffer.len() as u32, 0) } as usize;
        if length == 0 { return Err(io::Error::last_os_error()); }
        if length < buffer.len() { return Ok(OsString::from_wide(&buffer[..length]).into()); }
        if length > 32768 { return Err(io::Error::new(io::ErrorKind::InvalidInput, "native path exceeds the Win32 path limit")); }
        buffer.resize(length + 1, 0);
    }
}

fn native_io(error: io::Error) -> crate::NativeFileError { crate::NativeFileError::io("reading native Windows file", error) }
fn changed() -> crate::NativeFileError { crate::NativeFileError::Changed("FILE_CHANGED_DURING_READ: native Windows path changed identity".into()) }

/// Open only a disk file's exact resolved stream. FILE_OPEN_REPARSE_POINT
/// prevents a newly introduced final link from supplying bytes; checking the
/// handle name catches a redirected ancestor before any content is read.
pub fn open_source(path: &Path) -> Result<File, crate::NativeFileError> {
    open_source_with_sharing(path, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
}

/// A temporary snapshot reader denies concurrent write/delete opens for this
/// source stream until it has copied and verified all bytes.
pub fn open_source_locked(path: &Path) -> Result<File, crate::NativeFileError> {
    open_source_with_sharing(path, FILE_SHARE_READ)
}

fn open_source_with_sharing(path: &Path, sharing: u32) -> Result<File, crate::NativeFileError> {
    use std::os::windows::fs::MetadataExt as _;
    use windows_sys::{Wdk::Storage::FileSystem::FILE_NON_DIRECTORY_FILE,
        Win32::Storage::FileSystem::{FILE_READ_DATA, FILE_ATTRIBUTE_REPARSE_POINT, FILE_TYPE_DISK, GetFileType}};
    let file = open_with(None, path, FILE_READ_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        sharing, FILE_NON_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT).map_err(native_io)?;
    let metadata = file.metadata().map_err(native_io)?;
    // SAFETY: file owns a live native file handle. No device I/O is issued.
    if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK || !metadata.is_file()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(crate::NativeFileError::InvalidInput("source is not a regular disk file stream".into()));
    }
    if final_path(&file).map_err(native_io)? != path { return Err(changed()); }
    Ok(file)
}

pub struct NativeWindowsAccess;
impl crate::NativeDirectoryAccess for NativeWindowsAccess {
    fn canonicalize(&self, path: &Path) -> Result<std::path::PathBuf, crate::NativeFileError> {
        std::fs::canonicalize(path).map_err(native_io)
    }
    fn canonicalize_entry(&self, path: &Path) -> Result<std::path::PathBuf, crate::NativeFileError> {
        canonical_entry(path).map_err(native_io)
    }
    fn metadata(&self, path: &Path) -> Result<Option<crate::NativeEntryKind>, crate::NativeFileError> {
        use std::os::windows::fs::MetadataExt as _;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        let file = match open_metadata(None, path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(native_io(error)),
        };
        let metadata = file.metadata().map_err(native_io)?;
        if final_path(&file).map_err(native_io)? != path { return Err(changed()); }
        Ok(Some(if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 { crate::NativeEntryKind::Link }
            else if metadata.is_dir() { crate::NativeEntryKind::Directory }
            else if metadata.is_file() { crate::NativeEntryKind::File }
            else { crate::NativeEntryKind::Other }))
    }
    fn entries(&self, path: &Path) -> Result<Box<dyn Iterator<Item = Result<crate::NativeDirectoryEntry, crate::NativeFileError>>>, crate::NativeFileError> {
        use std::os::windows::fs::MetadataExt as _;
        use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_DIRECTORY};
        let directory = open(None, path).map_err(native_io)?;
        let metadata = directory.metadata().map_err(native_io)?;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 { return Err(changed()); }
        let cursor = open_read_cursor(&directory).map_err(native_io)?;
        if final_path(&directory).map_err(native_io)? != path { return Err(changed()); }
        let path = path.to_owned();
        Ok(Box::new(Entries::new(cursor).map(move |entry| {
            let entry = entry.map_err(native_io)?;
            let kind = if entry.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 { crate::NativeEntryKind::Link }
                else if entry.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 { crate::NativeEntryKind::Directory }
                else { crate::NativeEntryKind::File };
            Ok(crate::NativeDirectoryEntry { path: path.join(entry.name), kind })
        })))
    }
}
