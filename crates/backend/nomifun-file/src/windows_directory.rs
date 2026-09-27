//! Native relative opens for workspace directories, deletion and enumeration.
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

/// Open one child, or an already canonical absolute root, without following
/// the final reparse point. Callers check its type and resolved handle path.
pub(crate) fn open(parent: Option<&Dir>, path: &Path) -> io::Result<File> {
    open_with(parent, path, FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, FILE_DIRECTORY_FILE)
}

/// Open the entry itself for deletion, retaining its name until handle unlink.
pub(crate) fn open_delete(parent: Option<&Dir>, path: &Path) -> io::Result<File> {
    open_with(parent, path, DELETE | FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE, 0)
}

pub(crate) fn open_metadata(parent: Option<&Dir>, path: &Path) -> io::Result<File> {
    open_with(parent, path, FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, 0)
}

/// An empty native relative name reopens the retained directory itself. It
/// does not resolve the directory's possibly changed name in its parent.
pub(crate) fn open_cursor(directory: &File) -> io::Result<File> {
    open_native(directory.as_raw_handle(), &mut [], FILE_LIST_DIRECTORY | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT)
}

/// Listing access participates in sharing checks: retain the directory name
/// while a read-only caller consumes the cursor.
pub(crate) fn open_read_cursor(directory: &File) -> io::Result<File> {
    open_native(directory.as_raw_handle(), &mut [], FILE_LIST_DIRECTORY | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE, FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT)
}

fn open_with(parent: Option<&Dir>, path: &Path, access: u32, sharing: u32, options: u32) -> io::Result<File> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "invalid workspace directory path");
    let mut name: Vec<u16> = path.as_os_str().encode_wide().collect();
    if parent.is_some() {
        let mut components = path.components();
        if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
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

pub(crate) struct Entry { pub(crate) name: OsString, pub(crate) attributes: u32 }

/// A bounded native directory cursor. cap-std's Windows entries() reopens a
/// path; this cursor enumerates the retained directory object itself.
pub(crate) struct Entries { file: File, buffer: [u64; 512], offset: Option<usize>, first: bool, done: bool }

impl Entries {
    pub(crate) fn new(file: File) -> Self {
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
