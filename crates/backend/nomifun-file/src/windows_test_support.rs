use std::{fs, io, path::Path, ptr};
use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};

pub(crate) fn rename_with_posix_semantics(path: &Path, target: &Path, replace: bool) -> io::Result<()> {
    use windows_sys::Wdk::Storage::FileSystem::{FILE_RENAME_POSIX_SEMANTICS, FILE_RENAME_REPLACE_IF_EXISTS};
    use windows_sys::Win32::Storage::FileSystem::{DELETE, FILE_RENAME_INFO,
        FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_SHARE_DELETE, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FileRenameInfoEx, SetFileInformationByHandle};
    let file = fs::OpenOptions::new().access_mode(DELETE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).open(path)?;
    let name: Vec<u16> = target.as_os_str().encode_wide().collect();
    let offset = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
    // Keep a NUL for Win32 path conversion in addition to the explicit length.
    let size = offset + (name.len() + 1) * 2;
    let mut storage = vec![0_usize; size.div_ceil(std::mem::size_of::<usize>())];
    let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // SAFETY: storage has the header's alignment and space for the complete
    // variable-length name. Both it and file remain live through the call.
    let ok = unsafe {
        (*info).Anonymous.Flags = FILE_RENAME_POSIX_SEMANTICS
            | if replace { FILE_RENAME_REPLACE_IF_EXISTS } else { 0 };
        (*info).FileNameLength = (name.len() * 2) as u32;
        ptr::copy_nonoverlapping(name.as_ptr(), storage.as_mut_ptr().cast::<u8>().add(offset).cast(), name.len());
        SetFileInformationByHandle(file.as_raw_handle(), FileRenameInfoEx, info.cast(), size as u32)
    };
    if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}
