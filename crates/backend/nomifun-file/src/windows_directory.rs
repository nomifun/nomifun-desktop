//! Native relative opens for workspace directories, deletion and enumeration.
use std::{fs::File, io, mem, path::{Component, Path}, ptr};
use std::os::windows::{ffi::OsStrExt, io::{AsRawHandle, FromRawHandle}};

use cap_std::fs::Dir;
use windows_sys::Wdk::{Foundation::OBJECT_ATTRIBUTES, Storage::FileSystem::{
    NtOpenFile, FILE_DIRECTORY_FILE, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,
}};
use windows_sys::Win32::{Foundation::{HANDLE, RtlNtStatusToDosError, UNICODE_STRING},
    Storage::FileSystem::{DELETE, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, SYNCHRONIZE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_SHARE_DELETE},
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

/// An empty native relative name reopens the retained directory itself. It
/// does not resolve the directory's possibly changed name in its parent.
pub(crate) fn open_cursor(directory: &File) -> io::Result<File> {
    open_native(directory.as_raw_handle(), &mut [], FILE_LIST_DIRECTORY | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT)
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
