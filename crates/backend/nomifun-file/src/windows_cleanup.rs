//! Delete publication residues only through a handle to the recorded object.
use std::{fs::{File, OpenOptions}, io, mem, path::Path, sync::{Mutex, MutexGuard}};
use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::{AsRawHandle, FromRawHandle}};
use windows_sys::Win32::{Foundation::INVALID_HANDLE_VALUE, Storage::FileSystem::{
    DELETE, FILE_DISPOSITION_INFO, FILE_DISPOSITION_INFO_EX, FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
    FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
    FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_WRITE_ATTRIBUTES, FILE_READ_ATTRIBUTES, FILE_RENAME_INFO,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_SHARE_DELETE, SYNCHRONIZE,
    FileDispositionInfo, FileDispositionInfoEx, FileIdInfo, FileRenameInfo, GetFileInformationByHandleEx, ReOpenFile, SetFileInformationByHandle,
}};

#[derive(Debug)]
pub(crate) struct OwnedFile {
    // Metadata access keeps the file ID alive without conflicting with
    // ReplaceFile's exclusive opening of the staged source.
    // Arc publication identities share this state. Once deletion is marked,
    // every clone must stop using the cached ID before the last pin closes.
    file: Mutex<Option<File>>,
    volume: u64,
    id: [u8; 16],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileIdentity {
    volume: u64,
    id: [u8; 16],
}

fn identity(file: &impl AsRawHandle) -> io::Result<(u64, [u8; 16])> {
    let mut info = FILE_ID_INFO::default();
    // SAFETY: the live handle and correctly sized writable buffer remain valid.
    let ok = unsafe { GetFileInformationByHandleEx(file.as_raw_handle(), FileIdInfo,
        (&mut info as *mut FILE_ID_INFO).cast(), mem::size_of_val(&info) as u32) };
    if ok == 0 { return Err(io::Error::last_os_error()); }
    Ok((info.VolumeSerialNumber, info.FileId.Identifier))
}

pub(crate) fn durable_identity_token(file: &File) -> io::Result<String> {
    let (volume, id) = identity(file)?;
    Ok(format!("{volume:016x}-{:032x}", u128::from_le_bytes(id)))
}

impl OwnedFile {
    pub(crate) fn capture_named_regular(path: &Path) -> io::Result<Self> {
        use std::os::windows::fs::MetadataExt;
        let file = OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES | SYNCHRONIZE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err(io::Error::other(
                "publication recovery target changed type",
            ));
        }
        let (volume, id) = identity(&file)?;
        Ok(Self {
            file: Mutex::new(Some(file)),
            volume,
            id,
        })
    }

    pub(crate) fn matches_handle(&self, file: &File) -> io::Result<bool> {
        let _pin = self.live_pin()?;
        Ok(identity(file)? == (self.volume, self.id))
    }

    pub(crate) fn identity(&self) -> io::Result<FileIdentity> {
        let _pin = self.live_pin()?;
        Ok(FileIdentity {
            volume: self.volume,
            id: self.id,
        })
    }

    pub(crate) fn matches_identity(&self, identity: FileIdentity) -> io::Result<bool> {
        let _pin = self.live_pin()?;
        Ok((self.volume, self.id) == (identity.volume, identity.id))
    }

    pub(crate) fn capture(file: &File) -> io::Result<Self> {
        // SAFETY: ReOpenFile opens the same file object and returns a new handle.
        let handle = unsafe { ReOpenFile(file.as_raw_handle(), FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, FILE_FLAG_OPEN_REPARSE_POINT) };
        if handle == INVALID_HANDLE_VALUE { return Err(io::Error::last_os_error()); }
        let file = unsafe { File::from_raw_handle(handle) };
        let (volume, id) = identity(&file)?;
        Ok(Self { file: Mutex::new(Some(file)), volume, id })
    }

    pub(crate) fn remove(&self, path: &Path) -> io::Result<()> {
        self.remove_with_hook(path, || {})
    }

    /// Observe a name while the caller still holds its original write/delete
    /// handle. Cleanup separately reacquires a name lock and rechecks identity.
    pub(crate) fn verify_named_identity(&self, path: &Path) -> io::Result<()> {
        let _pin = self.live_pin()?;
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
        let _pin = self.live_pin()?;
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
        self.remove_with_operation(path, after_identity, |file| mark_delete(file, true))
    }

    fn remove_with_operation(&self, path: &Path, after_identity: impl FnOnce(), mark: impl FnOnce(&File) -> io::Result<()>) -> io::Result<()> {
        let mut pin = self.live_pin()?;
        let current = self.open_owned(path, 0)?;
        let name = DeletionName::capture(&current)?;
        after_identity();
        // Match remove_file's residue cleanup: unlink now even when an old
        // reader still holds the original, and allow removal of our staged
        // file after target permissions made it readonly. DELETE access and
        // the existing sharing contract were checked while opening current.
        mark(&current)?;
        // Basic disposition removes the name only after *all* our handles to
        // this object close. Consume the shared pin, including Arc clones.
        // Failure before marking leaves the pin and identity fully usable.
        let metadata_pin = pin.take();
        drop(current);
        drop(metadata_pin);
        name.verify_absent()
    }

    fn live_pin(&self) -> io::Result<MutexGuard<'_, Option<File>>> {
        let pin = self.file.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if pin.is_none() { return Err(io::Error::other("publication identity was consumed by deletion")); }
        Ok(pin)
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

    pub(crate) fn move_no_replace(&self, source: &Path, target: &Path) -> io::Result<()> {
        let _pin = self.live_pin()?;
        let current = self.open_owned(source, 0)?;
        rename_no_replace(&current, target)
    }

    fn restore_with_hook(&self, backup: &Path, target: &Path, after_identity: impl FnOnce()) -> io::Result<()> {
        let _pin = self.live_pin()?;
        let current = self.open_owned(backup, FILE_GENERIC_WRITE)?;
        after_identity();
        rename_no_replace(&current, target)?;
        // A failure after the rename remains uncertain at the caller; never
        // report a confirmed restore before flushing the original handle.
        current.sync_all()
    }
}

/// Mark only the already-owned object. A successful mark is not cleanup proof.
/// Callers must close their handles and then verify the exact name is absent.
pub(crate) fn mark_delete(file: &impl AsRawHandle, ignore_readonly: bool) -> io::Result<()> {
    let info = FILE_DISPOSITION_INFO_EX { Flags: FILE_DISPOSITION_FLAG_DELETE
        | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS
        | if ignore_readonly { FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE } else { 0 } };
    // SAFETY: the caller holds a live handle opened with DELETE access, and
    // the correctly sized disposition buffer is valid for the native call.
    disposition_with_fallback(|| {
        #[cfg(test)]
        if FORCE_BASIC_DISPOSITION.get() { return Err(io::Error::from_raw_os_error(87)); }
        let ok = unsafe { SetFileInformationByHandle(file.as_raw_handle(), FileDispositionInfoEx,
            (&info as *const FILE_DISPOSITION_INFO_EX).cast(), mem::size_of_val(&info) as u32) };
        if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }, || mark_delete_basic(file))
}

fn disposition_with_fallback(ex: impl FnOnce() -> io::Result<()>, basic: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    match ex() {
        Err(error) if matches!(error.raw_os_error(), Some(1 | 50 | 87)) => {
            let result = basic();
            if let Err(basic_error) = &result {
                tracing::debug!(ex_raw_os_error = error.raw_os_error(), basic_raw_os_error = basic_error.raw_os_error(),
                    basic_kind = ?basic_error.kind(), "native basic deletion failed after unsupported extended disposition");
            }
            result
        }
        result => result,
    }
}

fn mark_delete_basic(file: &impl AsRawHandle) -> io::Result<()> {
    let info = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: the same live DELETE handle is used without a path reopen or
    // identity gap. Basic disposition retains native readonly/ACL behavior.
    let ok = unsafe { SetFileInformationByHandle(file.as_raw_handle(), FileDispositionInfo,
        (&info as *const FILE_DISPOSITION_INFO).cast(), mem::size_of_val(&info) as u32) };
    if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

/// Consume our last DELETE handle, then prove the name disappeared. No path
/// deletion is retried, even if another handle keeps basic deletion pending.
pub(crate) fn remove_handle(file: File, ignore_readonly: bool) -> io::Result<()> {
    let name = DeletionName::capture(&file)?;
    mark_delete(&file, ignore_readonly)?;
    drop(file);
    name.verify_absent()
}

/// The proof names an entry in a retained native directory, not an absolute
/// path whose ancestor can disappear or be replaced after marking deletion.
pub(crate) struct DeletionName { parent: cap_std::fs::Dir, name: std::ffi::OsString }

impl DeletionName {
    pub(crate) fn capture(file: &impl AsRawHandle) -> io::Result<Self> {
        let native = nomi_native_files::windows::final_path(file)?;
        let parent_path = native.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "deletion entry has no parent"))?;
        let name = native.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "deletion entry has no name"))?.to_owned();
        // Metadata-only directory access; no listing, child creation, backup
        // privilege, or additional DELETE right is requested.
        let parent = cap_std::fs::Dir::from_std_file(crate::windows_directory::open(None, parent_path)?);
        let metadata = parent.dir_metadata()?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(io::Error::other("deletion parent changed type"));
        }
        let current = crate::windows_directory::open_metadata(Some(&parent), Path::new(&name))?;
        if identity(&current)? != identity(file)?
            || nomi_native_files::windows::join_component(&nomi_native_files::windows::final_path(&parent)?, &name)?
                != nomi_native_files::windows::final_path(file)?
        {
            return Err(io::Error::other("deletion parent or entry changed identity before marking"));
        }
        Ok(Self { parent, name })
    }

    pub(crate) fn verify_absent(&self) -> io::Result<()> {
        match crate::windows_directory::open_metadata(Some(&self.parent), Path::new(&self.name)) {
            // This is one relative native component with no traversed
            // ancestor. PATH_NOT_FOUND, access errors and pending deletion
            // do not prove that the name in the pinned directory is absent.
            Err(error) if error.raw_os_error() == Some(2) => Ok(()),
            Err(error) => Err(error),
            Ok(_) => Err(io::Error::other("marked deletion did not remove the recorded name")),
        }
    }
}

#[cfg(test)]
pub(crate) fn verify_absent(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Err(error) if matches!(error.raw_os_error(), Some(2 | 3)) => Ok(()),
        Err(error) => Err(error),
        Ok(_) => Err(io::Error::other("marked deletion did not remove the recorded name")),
    }
}

pub(crate) fn error_summary(error: &io::Error) -> String {
    format!("raw_os_error={:?},kind={:?}", error.raw_os_error(), error.kind())
}

#[cfg(test)]
thread_local! { static FORCE_BASIC_DISPOSITION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

#[cfg(test)]
pub(crate) fn with_basic_disposition<T>(operation: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset { fn drop(&mut self) { FORCE_BASIC_DISPOSITION.set(self.0); } }
    let _reset = Reset(FORCE_BASIC_DISPOSITION.replace(true));
    operation()
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

/// Rename through the retained directory object. Unlike the Win32 wrapper,
/// NT FileRenameInformation supports RootDirectory with one relative name.
/// No guard child (and therefore no extra child DELETE permission) is needed.
pub(crate) fn rename_relative_no_replace(file: &File, parent: &cap_std::fs::Dir, name: &std::ffi::OsStr) -> io::Result<()> {
    use windows_sys::Wdk::Storage::FileSystem::{NtSetInformationFile, FILE_RENAME_INFORMATION, FileRenameInformation};
    use windows_sys::Win32::{Foundation::RtlNtStatusToDosError, System::IO::IO_STATUS_BLOCK};
    let name: Vec<u16> = name.encode_wide().collect();
    if name.is_empty() || name.iter().any(|unit| matches!(*unit, 0 | 47 | 92)) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "rename requires one native entry name"));
    }
    let offset = mem::offset_of!(FILE_RENAME_INFORMATION, FileName);
    let size = offset + (name.len() + 1) * 2;
    let mut storage = vec![0_usize; size.div_ceil(mem::size_of::<usize>())];
    let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
    let mut status_block = IO_STATUS_BLOCK::default();
    // NtSetInformationFile is synchronous even for asynchronous file objects
    // (IRP_SYNCHRONOUS_API). All borrowed names/handles remain live until the
    // native completion; no FileRenameInformationEx bypass or copy flag is used.
    // SAFETY: aligned bounded storage contains the complete relative name;
    // both the source DELETE handle and destination directory are retained.
    let status = unsafe {
        (*info).Anonymous.ReplaceIfExists = false;
        (*info).RootDirectory = parent.as_raw_handle();
        (*info).FileNameLength = (name.len() * 2) as u32;
        std::ptr::copy_nonoverlapping(name.as_ptr(), storage.as_mut_ptr().cast::<u8>().add(offset).cast(), name.len());
        NtSetInformationFile(file.as_raw_handle(), &mut status_block, info.cast(), size as u32, FileRenameInformation)
    };
    if status < 0 { Err(io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32)) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use crate::windows_test_support::rename_with_posix_semantics;

    #[test]
    fn disposition_falls_back_only_for_native_unsupported_errors() {
        for code in [1, 50, 87, 5, 32, 145] {
            let called = std::cell::Cell::new(false);
            let result = disposition_with_fallback(|| Err(io::Error::from_raw_os_error(code)), || {
                called.set(true);
                Ok(())
            });
            assert_eq!(called.get(), matches!(code, 1 | 50 | 87));
            if called.get() { result.unwrap(); } else { assert_eq!(result.unwrap_err().raw_os_error(), Some(code)); }
        }
        let failure = disposition_with_fallback(|| Err(io::Error::from_raw_os_error(87)),
            || Err(io::Error::from_raw_os_error(5))).unwrap_err();
        assert_eq!(failure.raw_os_error(), Some(5), "retain the final native error without retry or attribute changes");
    }

    #[test]
    fn basic_deletion_closes_shared_publication_pin_and_consumes_all_clones() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("owned");
        fs::write(&path, b"owned").unwrap();
        let owner = std::sync::Arc::new(OwnedFile::capture_named_regular(&path).unwrap());
        let cloned = owner.clone();
        let old_identity = owner.identity().unwrap();
        with_basic_disposition(|| owner.remove(&path)).unwrap();
        verify_absent(&path).unwrap();
        assert!(owner.identity().is_err());
        assert!(cloned.matches_identity(old_identity).is_err());
        fs::write(&path, b"foreign").unwrap();
        assert!(cloned.matches_handle(&File::open(&path).unwrap()).is_err());
        assert!(cloned.remove(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"foreign");
    }

    #[test]
    fn basic_deletion_with_external_pin_stays_unconfirmed_until_it_closes() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("owned");
        fs::write(&path, b"owned").unwrap();
        let owner = OwnedFile::capture_named_regular(&path).unwrap();
        let external_pin = OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).open(&path).unwrap();
        let error = with_basic_disposition(|| owner.remove(&path)).unwrap_err();
        assert!(!matches!(error.raw_os_error(), Some(2 | 3)), "a delete-pending name is not proven absent: {error}");
        assert!(owner.identity().is_err(), "accepted deletion consumes the identity even when absence is unknown");
        drop(external_pin);
        verify_absent(&path).unwrap();
    }

    #[test]
    fn failed_basic_deletion_preserves_readonly_and_the_live_identity() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("owned");
        fs::write(&path, b"owned").unwrap();
        let original = fs::metadata(&path).unwrap().permissions();
        let mut readonly = original.clone(); readonly.set_readonly(true);
        fs::set_permissions(&path, readonly).unwrap();
        let owner = OwnedFile::capture_named_regular(&path).unwrap();
        let error = with_basic_disposition(|| owner.remove(&path)).unwrap_err();
        let unchanged = fs::metadata(&path).unwrap().permissions().readonly();
        fs::set_permissions(&path, original).unwrap();
        assert_eq!(error.raw_os_error(), Some(5));
        assert!(unchanged);
        owner.verify_named_identity(&path).unwrap();
        with_basic_disposition(|| owner.remove(&path)).unwrap();
    }

    fn recovery_fixture(name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let temporary = tempfile::tempdir().unwrap();
        let root = std::env::var_os("NOMIFUN_FILE_PUBLICATION_EVIDENCE")
            .map(std::path::PathBuf::from).map(|root| root.join(name))
            .unwrap_or_else(|| temporary.path().to_path_buf());
        fs::create_dir_all(&root).unwrap();
        (temporary, root)
    }

    #[test]
    fn recovery_window_locks_the_original_backup_name() {
        let (_temporary, root) = recovery_fixture("locked-backup");
        let backup = root.join("backup");
        let target = root.join("target");
        let foreign = root.join("foreign");
        let displaced = root.join("displaced");
        fs::write(&backup, b"original").unwrap();
        fs::write(&foreign, b"foreign").unwrap();
        let owner = OwnedFile::capture(&File::open(&backup).unwrap()).unwrap();
        let mut blocked = Vec::new();
        owner.restore_with_hook(&backup, &target, || {
            for (source, destination, replace) in [(&backup, &displaced, false), (&foreign, &backup, true)] {
                let failure = rename_with_posix_semantics(source, destination, replace).unwrap_err();
                blocked.push(failure.raw_os_error());
                assert_eq!(failure.raw_os_error(), Some(32));
            }
        }).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"original");
        assert_eq!(fs::read(&foreign).unwrap(), b"foreign");
        assert!(!backup.exists() && !displaced.exists());
        drop(owner);
        // The identical remap becomes possible after the recorded operation
        // releases its name guard; the fixture can perform the attempted race.
        fs::write(&backup, b"next original").unwrap();
        rename_with_posix_semantics(&backup, &displaced, false).unwrap();
        rename_with_posix_semantics(&foreign, &backup, true).unwrap();
        assert_eq!(fs::read(&backup).unwrap(), b"foreign");
        assert_eq!(fs::read(&displaced).unwrap(), b"next original");
        fs::write(root.join("assertions.json"), serde_json::to_vec(&serde_json::json!({
            "blocked_native_codes":blocked, "restored_original":true, "foreign_preserved":true,
            "same_race_possible_after_release":true,
        })).unwrap()).unwrap();
    }

    #[test]
    fn recovery_window_preserves_a_concurrent_hardlink_target() {
        let (_temporary, root) = recovery_fixture("concurrent-hardlink");
        let backup = root.join("backup");
        let target = root.join("target");
        let foreign = root.join("foreign");
        fs::write(&backup, b"original").unwrap();
        fs::write(&foreign, b"concurrent").unwrap();
        let owner = OwnedFile::capture(&File::open(&backup).unwrap()).unwrap();
        let failure = owner.restore_with_hook(&backup, &target, || {
            fs::hard_link(&foreign, &target).unwrap();
        }).unwrap_err();
        assert_eq!(failure.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&backup).unwrap(), b"original");
        assert_eq!(fs::read(&target).unwrap(), b"concurrent");
        assert_eq!(fs::read(&foreign).unwrap(), b"concurrent");
        let concurrent_owner = OwnedFile::capture(&File::open(&foreign).unwrap()).unwrap();
        assert!(concurrent_owner.matches_handle(&File::open(&target).unwrap()).unwrap());
        assert!(!owner.matches_handle(&File::open(&target).unwrap()).unwrap());
        fs::write(root.join("assertions.json"), serde_json::to_vec(&serde_json::json!({
            "error_kind":"AlreadyExists", "original_backup_preserved":true,
            "concurrent_hardlink_preserved":true, "foreign_object_identity_unchanged":true,
        })).unwrap()).unwrap();
    }

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
