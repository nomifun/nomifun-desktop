//! Host-authorized task output scope. Browser pages and Tool JSON never supply
//! filesystem authority. A native download stays private until terminal proof,
//! content validation and no-clobber publication have all succeeded.
use crate::{run_guard::RunAdmissionError, runtime::WorkspaceError};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

pub const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_TASK_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_TASK_DOWNLOAD_FILES: usize = 256;
pub const MAX_ACTIVE_DOWNLOADS: usize = 4;

#[derive(Default)]
struct Budget {
    active: usize,
    files: usize,
    bytes: u64,
    reserved: u64,
}
pub struct BrowserDownloadScope {
    root: Dir,
    budget: Mutex<Budget>,
}
#[derive(Default)]
struct Charge {
    reserved: u64,
    completed: bool,
}
pub struct PreparedBrowserDownload {
    scope: Arc<BrowserDownloadScope>,
    directory: tempfile::TempDir,
    charge: Mutex<Charge>,
    publication: Mutex<()>,
    residual: Mutex<Option<(Dir, String)>>,
}
#[derive(Clone, Debug, Serialize)]
pub struct BrowserDownloadArtifact {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}
fn cancelled(cancel: &CancellationToken) -> Result<(), WorkspaceError> {
    if cancel.is_cancelled() {
        Err(RunAdmissionError::Cancelled.into())
    } else {
        Ok(())
    }
}
fn filename_allowed(name: &str) -> bool {
    !name.is_empty()
        && name.encode_utf16().count() <= 160
        && !name.ends_with([' ', '.'])
        && !name
            .chars()
            .any(|ch| ch.is_control() || "/\\:*?\"<>|".contains(ch))
        && !matches!(name, "." | "..")
}
impl BrowserDownloadScope {
    pub fn open(root: &Path) -> Result<Self, WorkspaceError> {
        if !root.is_absolute() {
            return Err(WorkspaceError::DownloadDenied);
        }
        Ok(Self {
            root: Dir::open_ambient_dir(root, cap_std::ambient_authority())
                .map_err(|_| WorkspaceError::DownloadDenied)?,
            budget: Mutex::new(Budget::default()),
        })
    }
    pub fn prepare(self: &Arc<Self>) -> Result<Arc<PreparedBrowserDownload>, WorkspaceError> {
        let mut budget = self.budget.lock().unwrap_or_else(|e| e.into_inner());
        if budget.active >= MAX_ACTIVE_DOWNLOADS
            || budget.files + budget.active >= MAX_TASK_DOWNLOAD_FILES
        {
            return Err(WorkspaceError::DownloadLimit);
        }
        let directory = tempfile::Builder::new()
            .prefix("nomifun-agent-download-")
            .tempdir()
            .map_err(|_| WorkspaceError::DownloadDenied)?;
        budget.active += 1;
        Ok(Arc::new(PreparedBrowserDownload {
            scope: self.clone(),
            directory,
            charge: Mutex::new(Charge::default()),
            publication: Mutex::new(()),
            residual: Mutex::new(None),
        }))
    }
}
impl PreparedBrowserDownload {
    /// Only native host code receives this path; never serialize it.
    pub fn native_path(&self) -> PathBuf {
        self.directory.path().join("payload")
    }
    pub fn progress(&self, received: u64, total: Option<u64>) -> Result<(), WorkspaceError> {
        let proposed = received.max(total.unwrap_or(0));
        if proposed > MAX_DOWNLOAD_BYTES {
            return Err(WorkspaceError::DownloadLimit);
        }
        let mut charge = self.charge.lock().unwrap_or_else(|e| e.into_inner());
        if charge.completed {
            return Err(WorkspaceError::DownloadDenied);
        }
        let mut budget = self.scope.budget.lock().unwrap_or_else(|e| e.into_inner());
        let reserved = proposed.max(charge.reserved);
        let next = budget.reserved - charge.reserved + reserved;
        if budget.bytes.saturating_add(next) > MAX_TASK_DOWNLOAD_BYTES {
            return Err(WorkspaceError::DownloadLimit);
        }
        budget.reserved = next;
        charge.reserved = reserved;
        Ok(())
    }
    fn commit(&self, bytes: u64) {
        let mut charge = self.charge.lock().unwrap_or_else(|e| e.into_inner());
        let mut budget = self.scope.budget.lock().unwrap_or_else(|e| e.into_inner());
        budget.reserved -= charge.reserved;
        budget.bytes += bytes;
        budget.files += 1;
        charge.reserved = 0;
        charge.completed = true;
    }
    /// Call only after the native writer has reached terminal and released its
    /// handle. The host supplies the shared executable/content policy, not JSON.
    /// Await this worker even after Stop; publication may never outlive its run.
    pub async fn publish(
        self: &Arc<Self>,
        filename: String,
        validate: fn(&str, &[u8]) -> bool,
        cancel: CancellationToken,
    ) -> Result<BrowserDownloadArtifact, WorkspaceError> {
        let prepared = self.clone();
        tokio::task::spawn_blocking(move || {
            let _publication = prepared
                .publication
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            prepared.publish_blocking(&filename, validate, &cancel)
        })
        .await
        .map_err(|_| WorkspaceError::NativeCommandFailed)?
    }
    fn publish_blocking(
        &self,
        filename: &str,
        validate: fn(&str, &[u8]) -> bool,
        cancel: &CancellationToken,
    ) -> Result<BrowserDownloadArtifact, WorkspaceError> {
        cancelled(cancel)?;
        if !filename_allowed(filename) {
            return Err(WorkspaceError::DownloadDenied);
        }
        let private = Dir::open_ambient_dir(self.directory.path(), cap_std::ambient_authority())
            .map_err(|_| WorkspaceError::DownloadDenied)?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK);
        }
        let mut input = private
            .open_with("payload", &options)
            .map_err(|_| WorkspaceError::DownloadDenied)?;
        let metadata = input
            .metadata()
            .map_err(|_| WorkspaceError::DownloadDenied)?;
        if !metadata.is_file() {
            return Err(WorkspaceError::DownloadDenied);
        }
        self.progress(metadata.len(), Some(metadata.len()))?;
        let mut prefix = [0u8; 512];
        let read = metadata.len().min(prefix.len() as u64) as usize;
        input
            .read_exact(&mut prefix[..read])
            .map_err(|_| WorkspaceError::DownloadDenied)?;
        if !validate(filename, &prefix[..read]) {
            return Err(WorkspaceError::DownloadDenied);
        }
        match self.scope.root.create_dir("downloads") {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(WorkspaceError::DownloadDenied),
        }
        let output_dir = self
            .scope
            .root
            .open_dir_nofollow("downloads")
            .map_err(|_| WorkspaceError::DownloadDenied)?;
        let id = self
            .directory
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(WorkspaceError::DownloadDenied)?;
        let temporary = format!(".{id}.part");
        let published = format!("{id}-{filename}");
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = output_dir
            .open_with(&temporary, &options)
            .map_err(|_| WorkspaceError::DownloadDenied)?;
        let result = (|| {
            let mut hash = Sha256::new();
            hash.update(&prefix[..read]);
            output
                .write_all(&prefix[..read])
                .map_err(|_| WorkspaceError::DownloadDenied)?;
            let mut bytes = read as u64;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                cancelled(cancel)?;
                let read = input
                    .read(&mut buffer)
                    .map_err(|_| WorkspaceError::DownloadDenied)?;
                if read == 0 {
                    break;
                }
                bytes += read as u64;
                self.progress(bytes, None)?;
                output
                    .write_all(&buffer[..read])
                    .map_err(|_| WorkspaceError::DownloadDenied)?;
                hash.update(&buffer[..read]);
            }
            let after = input
                .metadata()
                .map_err(|_| WorkspaceError::DownloadDenied)?;
            if bytes != metadata.len()
                || after.len() != metadata.len()
                || after.modified().ok() != metadata.modified().ok()
            {
                return Err(WorkspaceError::DownloadDenied);
            }
            output
                .sync_all()
                .map_err(|_| WorkspaceError::DownloadDenied)?;
            #[cfg(windows)]
            {
                // Capability-relative ADS, before publication. No arbitrary path
                // is recovered from the page or a symlink-following string join.
                let mut mark = output_dir
                    .open_with(format!("{temporary}:Zone.Identifier"), &options)
                    .map_err(|_| WorkspaceError::DownloadDenied)?;
                mark.write_all(b"[ZoneTransfer]\r\nZoneId=3\r\n")
                    .map_err(|_| WorkspaceError::DownloadDenied)?;
                mark.sync_all()
                    .map_err(|_| WorkspaceError::DownloadDenied)?;
            }
            cancelled(cancel)?;
            // Atomic no-clobber. No check-then-rename fallback on filesystems
            // without hard links: publication must never overwrite user work.
            output_dir
                .hard_link(&temporary, &output_dir, &published)
                .map_err(|_| WorkspaceError::DownloadDenied)?;
            self.commit(bytes);
            Ok(BrowserDownloadArtifact {
                path: format!("downloads/{published}"),
                bytes,
                sha256: format!("{:x}", hash.finalize()),
            })
        })();
        drop(output);
        if output_dir.remove_file(&temporary).is_err() {
            // A residual output still costs quota; never silently release its
            // reservation. Its bounded private source remains owned until close.
            let (completed, reserved) = {
                let charge = self.charge.lock().unwrap_or_else(|e| e.into_inner());
                (charge.completed, charge.reserved)
            };
            if !completed {
                self.commit(reserved);
            }
            *self.residual.lock().unwrap_or_else(|e| e.into_inner()) =
                Some((output_dir, temporary));
        }
        result
    }
    /// The native owner must retain this prepared set until this succeeds.
    pub fn close(&self) -> Result<(), WorkspaceError> {
        let mut residual = self.residual.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((directory, filename)) = residual.as_ref() {
            match directory.remove_file(filename) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(WorkspaceError::NativeCommandFailed),
            }
            residual.take();
        }
        match std::fs::remove_dir_all(self.directory.path()) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(WorkspaceError::NativeCommandFailed),
        }
    }
}
impl Drop for PreparedBrowserDownload {
    fn drop(&mut self) {
        let charge = self.charge.get_mut().unwrap_or_else(|e| e.into_inner());
        let mut budget = self.scope.budget.lock().unwrap_or_else(|e| e.into_inner());
        budget.active -= 1;
        budget.reserved -= charge.reserved;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn concurrent_publications_of_one_download_have_only_one_winner() {
        let root = tempfile::tempdir().unwrap();
        let scope = Arc::new(BrowserDownloadScope::open(root.path()).unwrap());
        let prepared = scope.prepare().unwrap();
        std::fs::write(prepared.native_path(), b"safe").unwrap();
        let (a, b) = tokio::join!(
            prepared.publish("a.txt".into(), allow, CancellationToken::new()),
            prepared.publish("b.txt".into(), allow, CancellationToken::new())
        );
        assert_ne!(a.is_ok(), b.is_ok());
        assert_eq!(scope.budget.lock().unwrap().files, 1);
        assert_eq!(
            std::fs::read_dir(root.path().join("downloads"))
                .unwrap()
                .count(),
            1
        );
    }
    fn allow(_: &str, bytes: &[u8]) -> bool {
        !bytes.starts_with(b"MZ")
    }
    #[tokio::test]
    async fn publication_has_exact_bytes_hash_and_never_overwrites() {
        let root = tempfile::tempdir().unwrap();
        let scope = Arc::new(BrowserDownloadScope::open(root.path()).unwrap());
        let prepared = scope.prepare().unwrap();
        std::fs::write(prepared.native_path(), b"download").unwrap();
        let artifact = prepared
            .publish("中文.txt".into(), allow, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(root.path().join(&artifact.path)).unwrap(),
            b"download"
        );
        assert_eq!(artifact.bytes, 8);
        assert_eq!(
            artifact.sha256,
            format!("{:x}", Sha256::digest(b"download"))
        );
        assert!(
            prepared
                .publish("中文.txt".into(), allow, CancellationToken::new())
                .await
                .is_err()
        );
        #[cfg(windows)]
        assert_eq!(
            std::fs::read_to_string(format!(
                "{}:Zone.Identifier",
                root.path().join(&artifact.path).display()
            ))
            .unwrap(),
            "[ZoneTransfer]\r\nZoneId=3\r\n"
        );
        prepared.close().unwrap();
        assert_eq!(scope.budget.lock().unwrap().files, 1);
    }
    #[tokio::test]
    async fn reject_untrusted_names_content_and_cancel_without_output() {
        let root = tempfile::tempdir().unwrap();
        let scope = Arc::new(BrowserDownloadScope::open(root.path()).unwrap());
        let prepared = scope.prepare().unwrap();
        std::fs::write(prepared.native_path(), b"MZ executable").unwrap();
        for name in [
            "../escape.txt",
            "..\\escape.txt",
            "x:stream",
            "x.",
            "payload.txt",
        ] {
            assert!(
                prepared
                    .publish(name.into(), allow, CancellationToken::new())
                    .await
                    .is_err()
            );
        }
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(
            prepared
                .publish("file.txt".into(), allow, cancel)
                .await
                .is_err()
        );
        assert!(!root.path().join("downloads").exists());
    }
    #[test]
    fn shared_budget_limits_active_count_single_and_aggregate_bytes() {
        let root = tempfile::tempdir().unwrap();
        let scope = Arc::new(BrowserDownloadScope::open(root.path()).unwrap());
        let jobs: Vec<_> = (0..MAX_ACTIVE_DOWNLOADS)
            .map(|_| scope.prepare().unwrap())
            .collect();
        assert!(scope.prepare().is_err());
        assert!(jobs[0].progress(MAX_DOWNLOAD_BYTES + 1, None).is_err());
        jobs[0].progress(MAX_DOWNLOAD_BYTES, None).unwrap();
        jobs[1].progress(MAX_DOWNLOAD_BYTES, None).unwrap();
        assert!(jobs[2].progress(1, None).is_err());
        drop(jobs);
        assert!(scope.prepare().is_ok());
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn workspace_download_junction_is_not_followed() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        junction::create(outside.path(), root.path().join("downloads")).unwrap();
        let scope = Arc::new(BrowserDownloadScope::open(root.path()).unwrap());
        let prepared = scope.prepare().unwrap();
        std::fs::write(prepared.native_path(), b"safe").unwrap();
        assert!(
            prepared
                .publish("file.txt".into(), allow, CancellationToken::new())
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
        junction::delete(root.path().join("downloads")).unwrap();
    }
}
