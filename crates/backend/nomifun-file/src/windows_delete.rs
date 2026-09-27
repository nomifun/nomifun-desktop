//! Confined Windows removal without reopening checked entries by path.
use std::{fs::{File, OpenOptions}, io, mem, path::Path};
use std::os::windows::{fs::{OpenOptionsExt, MetadataExt}, io::AsRawHandle};
use cap_std::fs::Dir;
use nomifun_common::AppError;
use windows_sys::Win32::{Storage::FileSystem::{
    DELETE, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_DISPOSITION_INFO_EX, FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
    FileDispositionInfoEx, SetFileInformationByHandle,
}};

pub(crate) fn remove_entry(path: &Path, root: Option<&crate::windows_read::ReadRoot>, after_open: impl FnOnce()) -> Result<(), AppError> {
    let io_error = |error: io::Error| {
        let message = format!("cannot remove workspace entry '{}': {error}", path.display());
        if error.kind() == io::ErrorKind::NotFound { AppError::NotFound(message) } else { AppError::Internal(message) }
    };
    let file = match root {
        Some(root) => root.open_for_delete(path)?,
        None => OpenOptions::new().access_mode(DELETE | FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).open(path).map_err(io_error)?,
    };
    let metadata = file.metadata().map_err(io_error)?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || (!metadata.is_dir() && !metadata.is_file()) {
        return Err(AppError::Conflict("workspace deletion target changed to a link or unsupported entry".into()));
    }
    after_open();
    if metadata.is_dir() {
        remove_tree(file).map_err(|error| AppError::Internal(format!(
            "{}; recursive removal of '{}' failed: {error}; inspect the remaining tree before retry",
            crate::service::FILE_DELETE_OUTCOME_UNKNOWN, path.display()
        )))
    } else {
        unlink(&file).map_err(io_error)
    }
}

fn unlink(file: &impl AsRawHandle) -> io::Result<()> {
    // Honor readonly attributes. Readers granting delete sharing may keep the
    // old object, but the name must disappear before returning success.
    let info = FILE_DISPOSITION_INFO_EX { Flags: FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS };
    // SAFETY: the DELETE handle and correctly sized buffer stay live. No path
    // is looked up after checking the entry's type and native delete access.
    let ok = unsafe { SetFileInformationByHandle(file.as_raw_handle(), FileDispositionInfoEx,
        (&info as *const FILE_DISPOSITION_INFO_EX).cast(), mem::size_of_val(&info) as u32) };
    if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

struct Frame { directory: Dir, entries: crate::windows_directory::Entries }

impl Frame {
    fn new(file: File) -> io::Result<Self> {
        let entries = crate::windows_directory::Entries::new(crate::windows_directory::open_cursor(&file)?);
        Ok(Self { directory: Dir::from_std_file(file), entries })
    }
}

fn remove_tree(file: File) -> io::Result<()> {
    // Empty-directory deletion does not require LIST_DIRECTORY access.
    match unlink(&file) {
        Ok(()) => return Ok(()),
        Err(error) if error.raw_os_error() == Some(145) => {},
        Err(error) => return Err(error),
    }
    // Explicit frames avoid exhausting the call stack on deep native paths.
    let mut frames = vec![Frame::new(file)?];
    while let Some(frame) = frames.last_mut() {
        match frame.entries.next() {
            Some(name) => {
                let child = crate::windows_directory::open_delete(Some(&frame.directory), Path::new(&name?.name))?;
                let metadata = child.metadata()?;
                // Reparse entries are unlinked themselves, never traversed.
                if metadata.is_dir() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
                    match unlink(&child) {
                        Ok(()) => {},
                        Err(error) if error.raw_os_error() == Some(145) => frames.push(Frame::new(child)?),
                        Err(error) => return Err(error),
                    }
                } else { unlink(&child)?; }
            }
            None => {
                let frame = frames.pop().expect("a directory frame remains");
                drop(frame.entries);
                unlink(&frame.directory)?;
            }
        }
    }
    Ok(())
}
