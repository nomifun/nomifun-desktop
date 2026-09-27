//! Confined Windows removal without reopening checked entries by path.
use std::{ffi::OsString, fs::{File, OpenOptions}, io, mem, path::Path};
use std::os::windows::{ffi::OsStringExt, fs::{OpenOptionsExt, MetadataExt}, io::AsRawHandle};
use cap_std::fs::Dir;
use nomifun_common::AppError;
use windows_sys::Win32::{Foundation::ERROR_NO_MORE_FILES, Storage::FileSystem::{
    DELETE, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_DISPOSITION_INFO_EX, FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
    FILE_ID_BOTH_DIR_INFO, FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo,
    FileDispositionInfoEx, GetFileInformationByHandleEx, SetFileInformationByHandle,
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

struct Frame { directory: Dir, entries: DirectoryNames }

impl Frame {
    fn new(file: File) -> io::Result<Self> {
        let entries = DirectoryNames::new(&file)?;
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
                let child = crate::windows_directory::open_delete(Some(&frame.directory), Path::new(&name?))?;
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

/// A bounded native directory cursor. cap-std's Windows entries() reopens a
/// path; this cursor enumerates the retained directory object itself.
struct DirectoryNames { file: File, buffer: [u64; 512], offset: Option<usize>, first: bool, done: bool }

impl DirectoryNames {
    fn new(file: &File) -> io::Result<Self> {
        Ok(Self { file: crate::windows_directory::open_cursor(file)?,
            buffer: [0;512], offset: None, first: true, done: false })
    }

    fn next_name(&mut self) -> io::Result<Option<OsString>> {
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
            if name != "." && name != ".." { return Ok(Some(name)); }
        }
    }
}

impl Iterator for DirectoryNames {
    type Item = io::Result<OsString>;
    fn next(&mut self) -> Option<Self::Item> {
        match self.next_name() {
            Ok(name) => name.map(Ok),
            Err(error) => { self.done = true; Some(Err(error)) },
        }
    }
}
