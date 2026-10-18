//! Local upload snapshots. The Session directory is only the relative-path
//! default; source descriptors and actual OS access controls protect reads.
//! Native snapshots remain owned until the browser is proven closed, because
//! selected File objects may outlive both a Tool call and a page reset.
use crate::{run_guard::RunAdmissionError, runtime::WorkspaceError};
use std::{fs::{File, Metadata}, io::{Read, Seek, SeekFrom, Write}, path::{Path, PathBuf}, sync::Arc};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

pub const MAX_UPLOAD_FILES: usize = 16;
pub const MAX_UPLOAD_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_RETAINED_UPLOAD_FILES: usize = 128;
pub const MAX_RETAINED_UPLOAD_BYTES: u64 = 256 * 1024 * 1024;

pub struct BrowserUploadScope { default_directory: PathBuf }
pub struct PreparedBrowserUpload { directory: tempfile::TempDir, paths: Vec<PathBuf>, total_bytes: u64 }
impl PreparedBrowserUpload {
    /// Host-only paths, never Tool output. Keep native Windows verbatim names
    /// intact so long paths and literal trailing-dot names still identify the
    /// retained snapshot rather than a different Win32 alias.
    pub fn native_paths(&self) -> Result<Vec<String>, WorkspaceError> {
        self.paths.iter().map(|path| path.to_str().map(str::to_owned).ok_or(WorkspaceError::UploadPathDenied)).collect()
    }
    pub fn file_count(&self) -> usize { self.paths.len() }
    pub fn total_bytes(&self) -> u64 { self.total_bytes }
    /// Invoke only after native destruction is proven; failed cleanup can retry.
    pub fn close(&self) -> Result<(), WorkspaceError> {
        match std::fs::remove_dir_all(self.directory.path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(WorkspaceError::NativeCommandFailed),
        }
    }
}

fn cancelled(cancel: &CancellationToken) -> Result<(), WorkspaceError> {
    if cancel.is_cancelled() { Err(RunAdmissionError::Cancelled.into()) } else { Ok(()) }
}

struct Source {
    requested: PathBuf, canonical: PathBuf, file: File, metadata: Metadata,
    identity: same_file::Handle,
    #[cfg(unix)] parent: cap_std::fs::Dir,
}
impl Source {
    fn open(requested: PathBuf, canonical: PathBuf) -> Result<Self, WorkspaceError> {
        #[cfg(windows)]
        let file = nomi_native_files::windows::open_source_locked(&canonical).map_err(|_| WorkspaceError::UploadPathDenied)?;
        #[cfg(unix)]
        let parent = cap_std::fs::Dir::open_ambient_dir(canonical.parent().ok_or(WorkspaceError::UploadPathDenied)?, cap_std::ambient_authority())
            .map_err(|_| WorkspaceError::UploadPathDenied)?;
        #[cfg(unix)]
        let file = open_unix_source(&parent, &canonical)?;
        #[cfg(not(any(windows, unix)))]
        let file = File::open(&canonical).map_err(|_| WorkspaceError::UploadPathDenied)?;
        let metadata = file.metadata().map_err(|_| WorkspaceError::UploadPathDenied)?;
        if !metadata.is_file() { return Err(WorkspaceError::UploadPathDenied); }
        let identity = same_file::Handle::from_file(file.try_clone().map_err(|_| WorkspaceError::UploadPathDenied)?)
            .map_err(|_| WorkspaceError::UploadPathDenied)?;
        let source = Self { requested, canonical, file, metadata, identity, #[cfg(unix)] parent };
        source.verify()?;
        Ok(source)
    }
    fn verify(&self) -> Result<(), WorkspaceError> {
        let changed = || WorkspaceError::UploadSourceChanged;
        if std::fs::canonicalize(&self.requested).map_err(|_| changed())? != self.canonical { return Err(changed()); }
        #[cfg(windows)]
        let current = {
            if nomi_native_files::windows::final_path(&self.file).map_err(|_| changed())? != self.canonical { return Err(changed()); }
            nomi_native_files::windows::open_source(&self.canonical).map_err(|_| changed())?
        };
        #[cfg(unix)]
        let current = {
            let parent_path = self.canonical.parent().ok_or_else(changed)?;
            let named_parent = File::open(parent_path).map_err(|_| changed())?;
            let opened_parent = self.parent.try_clone().map_err(|_| changed())?.into_std_file();
            if std::fs::canonicalize(parent_path).map_err(|_| changed())? != parent_path
                || same_file::Handle::from_file(named_parent).map_err(|_| changed())?
                    != same_file::Handle::from_file(opened_parent).map_err(|_| changed())? { return Err(changed()); }
            open_unix_source(&self.parent, &self.canonical)?
        };
        #[cfg(not(any(windows, unix)))]
        let current = File::open(&self.canonical).map_err(|_| changed())?;
        let after = self.file.metadata().map_err(|_| changed())?;
        if same_file::Handle::from_file(current).map_err(|_| changed())? != self.identity
            || after.len() != self.metadata.len() || after.modified().ok() != self.metadata.modified().ok() {
            return Err(changed());
        }
        Ok(())
    }
}

#[cfg(unix)]
fn open_unix_source(parent: &cap_std::fs::Dir, canonical: &Path) -> Result<File, WorkspaceError> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
    use cap_std::fs::OpenOptionsExt as _;
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No).custom_flags(libc::O_NONBLOCK);
    parent.open_with(canonical.file_name().ok_or(WorkspaceError::UploadPathDenied)?, &options)
        .map(|file| file.into_std()).map_err(|_| WorkspaceError::UploadPathDenied)
}

impl BrowserUploadScope {
    /// Host-resolved default directory only. It need not still exist when an
    /// independent absolute source is acquired later in the same Runtime.
    pub fn open(default_directory: &Path) -> Result<Self, WorkspaceError> {
        if !default_directory.is_absolute() { return Err(WorkspaceError::UploadPathDenied); }
        Ok(Self { default_directory: default_directory.to_owned() })
    }
    fn resolve(&self, path: &str) -> Result<PathBuf, WorkspaceError> {
        if path.is_empty() || path.len() > 4096 || path.contains('\0') { return Err(WorkspaceError::UploadPathDenied); }
        let path = Path::new(path);
        #[cfg(windows)]
        if (path.has_root() || matches!(path.components().next(), Some(std::path::Component::Prefix(_)))) && !path.is_absolute() {
            return Err(WorkspaceError::UploadPathDenied);
        }
        let path = if path.is_absolute() { path.to_owned() } else { self.default_directory.join(path) };
        #[cfg(windows)]
        let path = nomi_native_files::windows::normalize_path(&path).map_err(|_| WorkspaceError::UploadPathDenied)?;
        Ok(path)
    }
    pub async fn prepare(self: &Arc<Self>, paths: Vec<String>, cancel: CancellationToken) -> Result<Arc<PreparedBrowserUpload>, WorkspaceError> {
        let scope = self.clone();
        // Cancellation is consumed inside the owned blocking worker. Always
        // join it before returning, including failed/cancelled preparations.
        tokio::task::spawn_blocking(move || scope.prepare_blocking(paths, &cancel)).await
            .map_err(|_| WorkspaceError::NativeCommandFailed)?
    }
    fn prepare_blocking(&self, paths: Vec<String>, cancel: &CancellationToken) -> Result<Arc<PreparedBrowserUpload>, WorkspaceError> {
        self.prepare_with_hooks(paths, cancel, |_| {}, |_| {})
    }
    fn prepare_with_hooks(&self, paths: Vec<String>, cancel: &CancellationToken,
        mut before_open: impl FnMut(&Path), mut after_copy: impl FnMut(&Path),
    ) -> Result<Arc<PreparedBrowserUpload>, WorkspaceError> {
        cancelled(cancel)?;
        if paths.is_empty() { return Err(WorkspaceError::UnsupportedAction); }
        if paths.len() > MAX_UPLOAD_FILES { return Err(WorkspaceError::UploadLimit); }
        let paths = paths.iter().map(|path| self.resolve(path)).collect::<Result<Vec<_>, _>>()?;
        let directory = tempfile::Builder::new().prefix("nomifun-browser-upload-").tempdir().map_err(|_| WorkspaceError::NativeCommandFailed)?;
        let mut prepared = PreparedBrowserUpload { directory, paths: vec![], total_bytes: 0 };
        for (index, requested) in paths.into_iter().enumerate() {
            cancelled(cancel)?;
            let canonical = std::fs::canonicalize(&requested).map_err(|_| WorkspaceError::UploadPathDenied)?;
            before_open(&canonical);
            let mut source = Source::open(requested, canonical)?;
            if source.metadata.len() > MAX_UPLOAD_BYTES.saturating_sub(prepared.total_bytes) { return Err(WorkspaceError::UploadLimit); }
            let staging = prepared.directory.path().join(index.to_string());
            std::fs::create_dir(&staging).map_err(|_| WorkspaceError::NativeCommandFailed)?;
            let staging = std::fs::canonicalize(staging).map_err(|_| WorkspaceError::NativeCommandFailed)?;
            // The selected name belongs to the caller's native source path;
            // following a legal link must not silently rename the upload to
            // the link target's basename. Identity checks still use canonical.
            let name = source.requested.file_name().ok_or(WorkspaceError::UploadPathDenied)?;
            #[cfg(windows)]
            if name.to_str().is_none_or(|name| name.contains(':')) {
                // An ADS source has real bytes but its colon name would create
                // another stream, not a snapshot directory entry with the same
                // File.name. Do not silently rename it or fake that identity.
                return Err(WorkspaceError::UploadSourceNameUnsupported);
            }
            #[cfg(windows)]
            let destination = nomi_native_files::windows::join_component(&staging, name).map_err(|_| WorkspaceError::UploadPathDenied)?;
            #[cfg(not(windows))]
            let destination = staging.join(name);
            let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(&destination).map_err(|_| WorkspaceError::NativeCommandFailed)?;
            #[cfg(windows)]
            if nomi_native_files::windows::final_path(&output).map_err(|_| WorkspaceError::NativeCommandFailed)? != destination {
                return Err(WorkspaceError::UploadSourceNameUnsupported);
            }
            let mut buffer = [0u8; 64 * 1024];
            let mut copied = 0_u64; let mut digest = Sha256::new();
            loop {
                cancelled(cancel)?;
                let read = source.file.read(&mut buffer).map_err(|_| WorkspaceError::UploadPathDenied)?;
                if read == 0 { break; }
                copied = copied.checked_add(read as u64).ok_or(WorkspaceError::UploadLimit)?;
                if copied > MAX_UPLOAD_BYTES.saturating_sub(prepared.total_bytes) { return Err(WorkspaceError::UploadLimit); }
                digest.update(&buffer[..read]);
                output.write_all(&buffer[..read]).map_err(|_| WorkspaceError::NativeCommandFailed)?;
            }
            after_copy(&source.canonical);
            source.verify()?;
            if copied != source.metadata.len() { return Err(WorkspaceError::UploadSourceChanged); }
            // Metadata alone misses a same-length write with a restored mtime
            // (and some writable mappings). Confirm the retained descriptor's
            // bytes a second time before publishing the immutable snapshot.
            source.file.seek(SeekFrom::Start(0)).map_err(|_| WorkspaceError::UploadPathDenied)?;
            let mut checked = 0_u64; let mut confirmation = Sha256::new();
            loop {
                cancelled(cancel)?;
                let read = source.file.read(&mut buffer).map_err(|_| WorkspaceError::UploadPathDenied)?;
                if read == 0 { break; }
                checked += read as u64;
                if checked > copied { return Err(WorkspaceError::UploadSourceChanged); }
                confirmation.update(&buffer[..read]);
            }
            if checked != copied || digest.finalize() != confirmation.finalize() { return Err(WorkspaceError::UploadSourceChanged); }
            source.verify()?;
            output.sync_all().map_err(|_| WorkspaceError::NativeCommandFailed)?;
            output.set_modified(source.metadata.modified().map_err(|_| WorkspaceError::UploadPathDenied)?).map_err(|_| WorkspaceError::NativeCommandFailed)?;
            prepared.total_bytes += copied; prepared.paths.push(destination);
        }
        cancelled(cancel)?;
        Ok(Arc::new(prepared))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn upload_snapshot_preserves_names_and_bytes_after_source_changes() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("附件.txt"), b"original").unwrap();
        let scope = Arc::new(BrowserUploadScope::open(root.path()).unwrap());
        let upload = scope
            .prepare(vec!["附件.txt".into()], CancellationToken::new())
            .await
            .unwrap();
        std::fs::write(root.path().join("附件.txt"), b"changed").unwrap();
        assert_eq!(upload.file_count(), 1);
        assert_eq!(upload.total_bytes(), 8);
        assert_eq!(upload.paths[0].file_name().unwrap(), "附件.txt");
        assert_eq!(std::fs::read(&upload.paths[0]).unwrap(), b"original");
        upload.close().unwrap();
        assert!(!upload.directory.path().exists());
    }
    #[tokio::test]
    async fn upload_rejects_special_paths_limits_directories_and_cancel() {
        let root = tempfile::tempdir().unwrap();
        let scope = Arc::new(BrowserUploadScope::open(root.path()).unwrap());
        assert_eq!(
            scope.prepare(vec![], CancellationToken::new()).await.err(),
            Some(WorkspaceError::UnsupportedAction)
        );
        for path in ["", "bad\0path", &"x".repeat(4097)] {
            assert_eq!(
                scope
                    .prepare(vec![path.into()], CancellationToken::new())
                    .await
                    .err(),
                Some(WorkspaceError::UploadPathDenied),
                "{path}"
            );
        }
        assert_eq!(
            scope
                .prepare(
                    vec!["a".into(); MAX_UPLOAD_FILES + 1],
                    CancellationToken::new()
                )
                .await
                .err(),
            Some(WorkspaceError::UploadLimit)
        );
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            scope.prepare(vec![], cancel).await,
            Err(WorkspaceError::Admission(RunAdmissionError::Cancelled))
        ));
        std::fs::create_dir(root.path().join("directory")).unwrap();
        assert_eq!(
            scope
                .prepare(vec!["directory".into()], CancellationToken::new())
                .await
                .err(),
            Some(WorkspaceError::UploadPathDenied)
        );
        let large = std::fs::File::create(root.path().join("large.bin")).unwrap();
        large.set_len(MAX_UPLOAD_BYTES + 1).unwrap();
        drop(large);
        assert_eq!(
            scope
                .prepare(vec!["large.bin".into()], CancellationToken::new())
                .await
                .err(),
            Some(WorkspaceError::UploadLimit)
        );
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn upload_accepts_explicit_symlink_sources() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("source"), b"x").unwrap();
        std::os::unix::fs::symlink(root.path().join("source"), root.path().join("link")).unwrap();
        let scope = Arc::new(BrowserUploadScope::open(root.path()).unwrap());
        let upload = scope.prepare(vec!["link".into()], CancellationToken::new()).await.unwrap();
        assert_eq!(std::fs::read(&upload.paths[0]).unwrap(), b"x");
        assert_eq!(upload.paths[0].file_name().unwrap(), "link");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn upload_accepts_windows_junction_sources_outside_the_default_directory() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), b"global source").unwrap();
        junction::create(outside.path(), root.path().join("escape")).unwrap();
        let scope = Arc::new(BrowserUploadScope::open(root.path()).unwrap());
        let upload = scope.prepare(vec!["escape/secret.txt".into()], CancellationToken::new()).await.unwrap();
        assert_eq!(std::fs::read(&upload.paths[0]).unwrap(), b"global source");
        assert_eq!(
            std::fs::read(outside.path().join("secret.txt")).unwrap(),
            b"global source"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn native_alias_names_are_exact_and_ads_sources_are_not_fake_snapshot_entries() {
        let root = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(root.path()).unwrap();
        std::fs::write(root.path().join("source.txt"), b"main").unwrap();
        std::fs::write(root.path().join("source.txt:private"), b"stream").unwrap();
        let literal = nomi_native_files::windows::join_component(&canonical, "literal.txt.".as_ref()).unwrap();
        std::fs::write(&literal, b"literal").unwrap();
        let scope = Arc::new(BrowserUploadScope::open(root.path()).unwrap());
        for name in ["source.txt.".to_owned(), "source.txt ".to_owned(), format!(r"\\.\{}", root.path().join("source.txt").display())] {
            let upload = scope.prepare(vec![name.into()], CancellationToken::new()).await.unwrap();
            assert_eq!(upload.paths[0].file_name().unwrap(), "source.txt");
            assert_eq!(std::fs::read(&upload.paths[0]).unwrap(), b"main"); upload.close().unwrap();
        }
        let upload = scope.prepare(vec![literal.to_str().unwrap().into()], CancellationToken::new()).await.unwrap();
        assert_eq!(upload.paths[0].file_name().unwrap(), "literal.txt.");
        assert_eq!(std::fs::read(&upload.paths[0]).unwrap(), b"literal");
        assert!(upload.native_paths().unwrap()[0].ends_with("\\literal.txt.")); upload.close().unwrap();
        assert!(matches!(scope.prepare(vec!["source.txt:private".into()], CancellationToken::new()).await,
            Err(WorkspaceError::UploadSourceNameUnsupported)));
        assert_eq!(std::fs::read(root.path().join("source.txt")).unwrap(), b"main");
        assert_eq!(std::fs::read(root.path().join("source.txt:private")).unwrap(), b"stream");
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires a real login token with WRITE_DAC for the unique ACL fixture"]
    fn native_acl_denial_does_not_prepare_an_upload_or_become_absence() {
        let root = tempfile::tempdir().unwrap(); let source = root.path().join("private.txt");
        std::fs::write(&source, b"preserved").unwrap();
        struct Restore(PathBuf);
        impl Drop for Restore { fn drop(&mut self) {
            let status = std::process::Command::new("icacls.exe").arg(&self.0).args(["/remove:d", "*S-1-1-0"]).status().unwrap();
            assert!(status.success());
        } }
        let restore = Restore(source.clone());
        let status = std::process::Command::new("icacls.exe").arg(&source).args(["/deny", "*S-1-1-0:(RD)"]).status().unwrap();
        assert!(status.success());
        let native = std::fs::read(&source).unwrap_err();
        let scope = BrowserUploadScope::open(root.path()).unwrap();
        let result = scope.prepare_blocking(vec!["private.txt".into()], &CancellationToken::new());
        drop(restore);
        assert_eq!(native.raw_os_error(), Some(5));
        assert!(matches!(result, Err(WorkspaceError::UploadPathDenied)));
        assert_eq!(std::fs::read(&source).unwrap(), b"preserved");
    }

    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "requires NOMIFUN_TEST_OTHER_VOLUME naming an explicit writable directory on another filesystem"]
    async fn upload_snapshots_an_explicit_other_volume_source() {
        let directory = std::env::var_os("NOMIFUN_TEST_OTHER_VOLUME").expect("explicit second-volume fixture directory");
        let root = tempfile::tempdir().unwrap(); let outside = tempfile::tempdir_in(directory).unwrap();
        let source = outside.path().join("other-volume.txt"); std::fs::write(&source, b"other volume source").unwrap();
        assert_ne!(root.path().components().next(), source.components().next(), "the configured source must use another drive");
        let scope = Arc::new(BrowserUploadScope::open(root.path()).unwrap());
        let upload = scope.prepare(vec![source.to_str().unwrap().into()], CancellationToken::new()).await.unwrap();
        std::fs::remove_file(&source).unwrap();
        assert_eq!(std::fs::read(&upload.paths[0]).unwrap(), b"other volume source");
        assert_eq!(upload.paths[0].file_name().unwrap(), "other-volume.txt"); upload.close().unwrap();
    }

    #[tokio::test]
    async fn upload_accepts_absolute_parent_and_native_aliases_with_exact_snapshots() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("workspace"); let outside = fixture.path().join("outside");
        std::fs::create_dir(&workspace).unwrap(); std::fs::create_dir(&outside).unwrap();
        let source = outside.join("附件 source.txt"); std::fs::write(&source, b"external exact bytes").unwrap();
        let modified = std::fs::metadata(&source).unwrap().modified().unwrap();
        let scope = Arc::new(BrowserUploadScope::open(&workspace).unwrap());
        let paths = vec![source.to_str().unwrap().to_owned(), "../outside/附件 source.txt".into()];
        let upload = scope.prepare(paths, CancellationToken::new()).await.unwrap();
        for path in &upload.paths {
            assert_eq!(std::fs::read(path).unwrap(), b"external exact bytes");
            assert_eq!(path.file_name().unwrap(), "附件 source.txt");
            assert_eq!(std::fs::metadata(path).unwrap().modified().unwrap(), modified);
        }
        std::fs::write(&source, b"later source").unwrap();
        assert_eq!(std::fs::read(&upload.paths[0]).unwrap(), b"external exact bytes");
        std::fs::remove_dir(&workspace).unwrap();
        let later = scope.prepare(vec![source.to_str().unwrap().to_owned()], CancellationToken::new()).await.unwrap();
        assert_eq!(std::fs::read(&later.paths[0]).unwrap(), b"later source");
        upload.close().unwrap(); later.close().unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn upload_rejects_an_ancestor_junction_introduced_between_resolution_and_open() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("inside"); let outside = fixture.path().join("outside");
        std::fs::create_dir(&root).unwrap(); std::fs::create_dir(&outside).unwrap();
        std::fs::write(root.join("value.txt"), b"inside").unwrap(); std::fs::write(outside.join("value.txt"), b"outside").unwrap();
        let scope = BrowserUploadScope::open(&root).unwrap();
        let result = scope.prepare_with_hooks(vec!["value.txt".into()], &CancellationToken::new(), |_| {
            std::fs::rename(&root, fixture.path().join("retained")).unwrap(); junction::create(&outside, &root).unwrap();
        }, |_| panic!("outside bytes must not be copied"));
        junction::delete(&root).unwrap(); std::fs::rename(fixture.path().join("retained"), &root).unwrap();
        assert!(matches!(result, Err(WorkspaceError::UploadPathDenied | WorkspaceError::UploadSourceChanged)));
        assert_eq!(std::fs::read(root.join("value.txt")).unwrap(), b"inside");
    }

    #[cfg(unix)]
    #[test]
    fn same_length_source_mutation_with_restored_mtime_does_not_publish_a_mixed_snapshot() {
        let root = tempfile::tempdir().unwrap(); let source = root.path().join("value.txt");
        std::fs::write(&source, b"original").unwrap(); let modified = std::fs::metadata(&source).unwrap().modified().unwrap();
        let scope = BrowserUploadScope::open(root.path()).unwrap();
        let result = scope.prepare_with_hooks(vec!["value.txt".into()], &CancellationToken::new(), |_| {}, |_| {
            std::fs::write(&source, b"modified").unwrap(); File::options().write(true).open(&source).unwrap().set_modified(modified).unwrap();
        });
        assert!(matches!(result, Err(WorkspaceError::UploadSourceChanged)));
    }

    #[cfg(windows)]
    #[test]
    fn a_live_writable_mapping_cannot_supply_an_unverified_snapshot() {
        use std::os::windows::{fs::OpenOptionsExt as _, io::AsRawHandle as _};
        use std::ffi::c_void;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CreateFileMappingW(file: *mut c_void, attributes: *const c_void, protect: u32, high: u32, low: u32, name: *const u16) -> *mut c_void;
            fn MapViewOfFile(mapping: *mut c_void, access: u32, high: u32, low: u32, bytes: usize) -> *mut c_void;
            fn UnmapViewOfFile(base: *const c_void) -> i32;
            fn CloseHandle(handle: *mut c_void) -> i32;
        }
        struct Mapping { handle: *mut c_void, address: *mut c_void }
        impl Drop for Mapping { fn drop(&mut self) { unsafe { UnmapViewOfFile(self.address); CloseHandle(self.handle); } } }
        let root = tempfile::tempdir().unwrap(); let source = root.path().join("value.txt");
        std::fs::write(&source, b"original").unwrap();
        let file = File::options().read(true).write(true).share_mode(7).open(&source).unwrap();
        // SAFETY: the fixture is exactly eight initialized bytes. PAGE_READWRITE
        // and FILE_MAP_WRITE create a mapping retained until after the worker.
        let mapping = unsafe {
            let handle = CreateFileMappingW(file.as_raw_handle(), std::ptr::null(), 4, 0, 0, std::ptr::null());
            assert!(!handle.is_null());
            let address = MapViewOfFile(handle, 2, 0, 0, 8); assert!(!address.is_null());
            Mapping { handle, address }
        };
        drop(file);
        let scope = BrowserUploadScope::open(root.path()).unwrap();
        let mut changed = false;
        let result = scope.prepare_with_hooks(vec!["value.txt".into()], &CancellationToken::new(), |_| {}, |_| {
            // Mapped writes can change bytes without a new writable file open
            // or a modified-time update. Metadata-only verification misses it.
            unsafe { std::ptr::copy_nonoverlapping(b"modified".as_ptr(), mapping.address.cast::<u8>(), 8); }
            changed = true;
        });
        if changed {
            assert!(matches!(result, Err(WorkspaceError::UploadSourceChanged)));
        } else {
            // This Windows build keeps the mapping's writable share active
            // even after its file handle closes. Prove the independent OS
            // refusal rather than claiming the hash branch executed.
            let denied = nomi_native_files::windows::open_source_locked(&std::fs::canonicalize(&source).unwrap()).unwrap_err();
            assert_eq!(denied.observation().raw_os_error, Some(32));
            assert!(matches!(result, Err(WorkspaceError::UploadPathDenied)));
        }
        drop(mapping);
    }

    #[test]
    fn cancellation_during_copy_is_joined_before_any_snapshot_is_returned() {
        let root = tempfile::tempdir().unwrap(); std::fs::write(root.path().join("value.txt"), b"source").unwrap();
        let scope = BrowserUploadScope::open(root.path()).unwrap(); let cancel = CancellationToken::new();
        let result = scope.prepare_with_hooks(vec!["value.txt".into()], &cancel, |_| {}, |_| cancel.cancel());
        assert!(matches!(result, Err(WorkspaceError::Admission(RunAdmissionError::Cancelled))));
    }
}
