use std::{fs, io, path::Path, ptr};
use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};

/// A writable section whose creating file handle is already closed.
pub(crate) struct WritableMapping {
    handle: windows_sys::Win32::Foundation::HANDLE,
    view: windows_sys::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS,
    length: usize,
}

impl WritableMapping {
    pub(crate) fn open(path: &Path, length: usize) -> io::Result<Self> {
        use windows_sys::Win32::System::Memory::{CreateFileMappingW, MapViewOfFile, PAGE_READWRITE, FILE_MAP_WRITE};
        let file = fs::OpenOptions::new().read(true).write(true).open(path)?;
        let handle = unsafe { CreateFileMappingW(file.as_raw_handle(), ptr::null(), PAGE_READWRITE, 0, 0, ptr::null()) };
        if handle.is_null() { return Err(io::Error::last_os_error()); }
        let view = unsafe { MapViewOfFile(handle, FILE_MAP_WRITE, 0, 0, length) };
        if view.Value.is_null() {
            let error = io::Error::last_os_error();
            unsafe { windows_sys::Win32::Foundation::CloseHandle(handle); }
            return Err(error);
        }
        Ok(Self { handle, view, length })
    }

    pub(crate) fn write(&self, bytes: &[u8]) -> io::Result<()> {
        assert_eq!(bytes.len(), self.length);
        // SAFETY: this test owns a writable view of exactly length bytes.
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), self.view.Value.cast(), bytes.len()); }
        if unsafe { windows_sys::Win32::System::Memory::FlushViewOfFile(self.view.Value, self.length) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for WritableMapping {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Memory::UnmapViewOfFile(self.view);
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

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
