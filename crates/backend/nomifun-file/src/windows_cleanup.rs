//! Delete publication residues only through a handle to the recorded object.
use std::{fs::{File, OpenOptions}, io, mem, path::Path};
use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::{AsRawHandle, FromRawHandle}};
use windows_sys::Win32::{Foundation::INVALID_HANDLE_VALUE, Storage::FileSystem::{
    DELETE, FILE_DISPOSITION_INFO_EX, FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
    FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
    FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_WRITE_ATTRIBUTES, FILE_READ_ATTRIBUTES, FILE_RENAME_INFO,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_SHARE_DELETE, SYNCHRONIZE,
    FileDispositionInfoEx, FileIdInfo, FileRenameInfo, GetFileInformationByHandleEx, ReOpenFile, SetFileInformationByHandle,
}};

pub(crate) struct OwnedFile {
    // Metadata access keeps the file ID alive without conflicting with
    // ReplaceFile's exclusive opening of the staged source.
    _file: File,
    volume: u64,
    id: [u8; 16],
}

fn identity(file: &File) -> io::Result<(u64, [u8; 16])> {
    let mut info = FILE_ID_INFO::default();
    // SAFETY: the live handle and correctly sized writable buffer remain valid.
    let ok = unsafe { GetFileInformationByHandleEx(file.as_raw_handle(), FileIdInfo,
        (&mut info as *mut FILE_ID_INFO).cast(), mem::size_of_val(&info) as u32) };
    if ok == 0 { return Err(io::Error::last_os_error()); }
    Ok((info.VolumeSerialNumber, info.FileId.Identifier))
}

impl OwnedFile {
    pub(crate) fn capture(file: &File) -> io::Result<Self> {
        // SAFETY: ReOpenFile opens the same file object and returns a new handle.
        let handle = unsafe { ReOpenFile(file.as_raw_handle(), FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, FILE_FLAG_OPEN_REPARSE_POINT) };
        if handle == INVALID_HANDLE_VALUE { return Err(io::Error::last_os_error()); }
        let file = unsafe { File::from_raw_handle(handle) };
        let (volume, id) = identity(&file)?;
        Ok(Self { _file: file, volume, id })
    }

    pub(crate) fn remove(&self, path: &Path) -> io::Result<()> {
        self.remove_with_hook(path, || {})
    }

    /// Observe a name while the caller still holds its original write/delete
    /// handle. Cleanup separately reacquires a name lock and rechecks identity.
    pub(crate) fn verify_named_identity(&self, path: &Path) -> io::Result<()> {
        use std::os::windows::fs::MetadataExt;
        let current = OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES | SYNCHRONIZE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(path)?;
        let metadata = current.metadata()?;
        if identity(&current)? != (self.volume, self.id) || !metadata.is_file()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err(io::Error::other("replacement backup changed identity or type"));
        }
        Ok(())
    }

    /// Read only the recorded object, retaining its name and bytes for the
    /// observation. WRITE_ATTRIBUTES preserves target permissions by handle.
    pub(crate) fn open_for_verification(&self, path: &Path) -> io::Result<File> {
        use std::os::windows::fs::MetadataExt;
        let file = OpenOptions::new().access_mode(FILE_GENERIC_READ | FILE_WRITE_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(path)?;
        let metadata = file.metadata()?;
        if identity(&file)? != (self.volume, self.id) || !metadata.is_file()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err(io::Error::other("publication source changed identity or type"));
        }
        Ok(file)
    }

    fn remove_with_hook(&self, path: &Path, after_identity: impl FnOnce()) -> io::Result<()> {
        let current = self.open_owned(path, 0)?;
        after_identity();
        // Match remove_file's residue cleanup: unlink now even when an old
        // reader still holds the original, and allow removal of our staged
        // file after target permissions made it readonly. DELETE access and
        // the existing sharing contract were checked while opening current.
        remove_handle(&current)
    }

    fn open_owned(&self, path: &Path, access: u32) -> io::Result<File> {
        let current = OpenOptions::new().access_mode(access | DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(path)?;
        if identity(&current)? != (self.volume, self.id) {
            return Err(io::Error::other("publication cleanup path changed file identity"));
        }
        Ok(current)
    }

    pub(crate) fn restore(&self, backup: &Path, target: &Path) -> io::Result<()> {
        self.restore_with_hook(backup, target, || {})
    }

    fn restore_with_hook(&self, backup: &Path, target: &Path, after_identity: impl FnOnce()) -> io::Result<()> {
        let current = self.open_owned(backup, FILE_GENERIC_WRITE)?;
        after_identity();
        rename_no_replace(&current, target)?;
        // A failure after the rename remains uncertain at the caller; never
        // report a confirmed restore before flushing the original handle.
        current.sync_all()
    }
}

/// Unlink an already-owned handle without looking up its name again.
pub(crate) fn remove_handle(file: &File) -> io::Result<()> {
    let info = FILE_DISPOSITION_INFO_EX { Flags: FILE_DISPOSITION_FLAG_DELETE
        | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE };
    // SAFETY: the caller holds a live handle opened with DELETE access, and
    // the correctly sized disposition buffer is valid for the native call.
    let ok = unsafe { SetFileInformationByHandle(file.as_raw_handle(), FileDispositionInfoEx,
        (&info as *const FILE_DISPOSITION_INFO_EX).cast(), mem::size_of_val(&info) as u32) };
    if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

pub(crate) fn rename_no_replace(file: &File, target: &Path) -> io::Result<()> {
    let name: Vec<u16> = target.as_os_str().encode_wide().collect();
    if name.contains(&0) { return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid publication target")); }
    let offset = mem::offset_of!(FILE_RENAME_INFO, FileName);
    let size = offset + (name.len() + 1) * 2;
    let mut storage = vec![0_usize; size.div_ceil(mem::size_of::<usize>())];
    let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // SAFETY: the aligned buffer contains the complete NUL-terminated name.
    // ReplaceIfExists is false: a concurrent target cannot be overwritten.
    let ok = unsafe {
        (*info).Anonymous.ReplaceIfExists = false;
        (*info).FileNameLength = (name.len() * 2) as u32;
        std::ptr::copy_nonoverlapping(name.as_ptr(), storage.as_mut_ptr().cast::<u8>().add(offset).cast(), name.len());
        SetFileInformationByHandle(file.as_raw_handle(), FileRenameInfo, info.cast(), size as u32)
    };
    if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use crate::windows_test_support::rename_with_posix_semantics;

    #[test]
    fn cleanup_locks_the_named_object_until_handle_deletion_finishes() {
        let fixture = tempfile::tempdir().unwrap();
        let temporary = fixture.path().join("temporary");
        let foreign = fixture.path().join("foreign");
        fs::write(&temporary, b"owned").unwrap();
        fs::write(&foreign, b"foreign").unwrap();
        let owner = OwnedFile::capture(&File::open(&temporary).unwrap()).unwrap();
        let result = owner.remove_with_hook(&temporary, || {
            let error = rename_with_posix_semantics(&foreign, &temporary, true).unwrap_err();
            assert_eq!(error.raw_os_error(), Some(32));
        });
        result.unwrap();
        drop(owner);
        assert!(!temporary.exists());
        assert_eq!(fs::read(&foreign).unwrap(), b"foreign");
        assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
        fs::write(&temporary, b"new owned file").unwrap();
        rename_with_posix_semantics(&foreign, &temporary, true).unwrap();
        assert_eq!(fs::read(&temporary).unwrap(), b"foreign");
    }

    #[test]
    fn restore_keeps_a_target_created_after_the_backup_identity_check() {
        let fixture = tempfile::tempdir().unwrap();
        let backup = fixture.path().join("backup");
        let target = fixture.path().join("target");
        fs::write(&backup, b"original").unwrap();
        let owner = OwnedFile::capture(&File::open(&backup).unwrap()).unwrap();
        let result = owner.restore_with_hook(&backup, &target, || {
            fs::write(&target, b"concurrent target").unwrap();
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&target).unwrap(), b"concurrent target");
        assert_eq!(fs::read(&backup).unwrap(), b"original");
        fs::remove_file(&target).unwrap();
        owner.restore(&backup, &target).unwrap();
        assert!(!backup.exists());
        assert_eq!(fs::read(&target).unwrap(), b"original");
    }
}
