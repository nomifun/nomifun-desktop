//! Metadata-only directory handles used for workspace path resolution.
use std::{fs::File, io, mem, path::{Component, Path}, ptr};
use std::os::windows::{ffi::OsStrExt, io::{AsRawHandle, FromRawHandle}};

use cap_std::fs::Dir;
use windows_sys::Wdk::{Foundation::OBJECT_ATTRIBUTES, Storage::FileSystem::{
    NtOpenFile, FILE_DIRECTORY_FILE, FILE_OPEN_REPARSE_POINT,
}};
use windows_sys::Win32::{Foundation::{RtlNtStatusToDosError, UNICODE_STRING},
    Storage::FileSystem::{FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_SHARE_DELETE},
    System::IO::IO_STATUS_BLOCK};

/// Open one child, or an already canonical absolute root, without following
/// the final reparse point. Callers check its type and resolved handle path.
pub(crate) fn open(parent: Option<&Dir>, path: &Path) -> io::Result<File> {
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
    let length = u16::try_from(name.len() * 2).map_err(|_| invalid())?;
    let mut unicode = UNICODE_STRING { Length: length, MaximumLength: length, Buffer: name.as_mut_ptr() };
    let attributes = OBJECT_ATTRIBUTES {
        Length: mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.map_or(ptr::null_mut(), |directory| directory.as_raw_handle()),
        ObjectName: &mut unicode,
        ..Default::default()
    };
    let mut status_block = IO_STATUS_BLOCK::default();
    let mut handle = ptr::null_mut();
    // Synchronous CreateFile/cap-std opens also request SYNCHRONIZE, which
    // a directory's deny-GENERIC_WRITE ACE can deny. These handles only serve
    // metadata queries and relative lookups; no synchronous data IO, listing,
    // traverse access, or backup privilege is needed.
    // SAFETY: all structures and the UTF-16 buffer remain live for the open;
    // NtOpenFile returns a new owned handle on success.
    let status = unsafe { NtOpenFile(&mut handle, FILE_READ_ATTRIBUTES, &attributes,
        &mut status_block, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT) };
    if status < 0 {
        return Err(io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32));
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}
