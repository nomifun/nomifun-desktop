//! Delete a regular workspace file through its authorized native handle.
use std::{fs::OpenOptions, mem, os::windows::{fs::OpenOptionsExt, io::AsRawHandle}, path::Path};
use nomifun_common::AppError;
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_DISPOSITION_INFO_EX, FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
    FileDispositionInfoEx, SetFileInformationByHandle,
};

pub(crate) fn remove_regular_file(path: &Path, after_open: impl FnOnce()) -> Result<(), AppError> {
    let io_error = |error| AppError::Internal(format!("cannot remove file '{}': {error}", path.display()));
    // DELETE access participates in sharing checks. Keep this handle without
    // delete sharing through removal so the checked name cannot be replaced.
    // Metadata access is sufficient: deleting does not require reading bytes.
    let file = OpenOptions::new().access_mode(DELETE | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(path).map_err(io_error)?;
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(AppError::Conflict("workspace deletion target changed to a non-regular file".into()));
    }
    after_open();
    // Unlike cleanup of an owned stage, user-file deletion must honor the
    // readonly attribute. POSIX unlink makes the name disappear even while
    // readers that granted delete sharing retain the old object.
    let info = FILE_DISPOSITION_INFO_EX { Flags: FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS };
    // SAFETY: the checked file handle and correctly sized info stay live; the
    // operation never reopens the path after the access/type check.
    let ok = unsafe { SetFileInformationByHandle(file.as_raw_handle(), FileDispositionInfoEx,
        (&info as *const FILE_DISPOSITION_INFO_EX).cast(), mem::size_of_val(&info) as u32) };
    if ok == 0 { Err(io_error(std::io::Error::last_os_error())) } else { Ok(()) }
}
