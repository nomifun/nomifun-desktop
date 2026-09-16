//! Workspace-anchored upload snapshots. Neither root authority nor native file
//! paths can be constructed from browser Tool JSON. The native owner retains
//! the prepared set while the browser can still read its selected File objects.
use crate::{run_guard::RunAdmissionError, runtime::WorkspaceError};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use std::{
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

pub const MAX_UPLOAD_FILES: usize = 16;
pub const MAX_UPLOAD_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_RETAINED_UPLOAD_FILES: usize = 128;
pub const MAX_RETAINED_UPLOAD_BYTES: u64 = 256 * 1024 * 1024;

pub struct BrowserUploadScope {
    root: Dir,
}
pub struct PreparedBrowserUpload {
    directory: tempfile::TempDir,
    paths: Vec<PathBuf>,
    total_bytes: u64,
}
impl PreparedBrowserUpload {
    /// Host-only protocol paths. Never include these in Tool output or renderer DTOs.
    pub fn native_paths(&self) -> Result<Vec<String>, WorkspaceError> {
        self.paths
            .iter()
            .map(|path| {
                dunce::simplified(path)
                    .to_str()
                    .map(str::to_owned)
                    .ok_or(WorkspaceError::UploadPathDenied)
            })
            .collect()
    }
    pub fn file_count(&self) -> usize {
        self.paths.len()
    }
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }
    /// Invoke only after native destruction is proven; failed cleanup can retry.
    pub fn close(&self) -> Result<(), WorkspaceError> {
        match std::fs::remove_dir_all(self.directory.path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(WorkspaceError::NativeCommandFailed),
        }
    }
}
fn relative_components(path: &str) -> Result<Vec<String>, WorkspaceError> {
    if path.is_empty() || path.len() > 4096 {
        return Err(WorkspaceError::UploadPathDenied);
    }
    let portable = path.replace('\\', "/");
    let components = Path::new(&portable)
        .components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_owned)
                .ok_or(WorkspaceError::UploadPathDenied),
            _ => Err(WorkspaceError::UploadPathDenied),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if components.is_empty() || components.len() > 64 {
        return Err(WorkspaceError::UploadPathDenied);
    }
    for component in &components {
        if component.ends_with([' ', '.'])
            || component
                .chars()
                .any(|ch| ch.is_control() || ":*?\"<>|".contains(ch))
        {
            return Err(WorkspaceError::UploadPathDenied);
        }
        let stem = component.split('.').next().unwrap_or("").to_uppercase();
        let device = matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
        if device {
            return Err(WorkspaceError::UploadPathDenied);
        }
    }
    Ok(components)
}
fn cancelled(cancel: &CancellationToken) -> Result<(), WorkspaceError> {
    if cancel.is_cancelled() {
        Err(RunAdmissionError::Cancelled.into())
    } else {
        Ok(())
    }
}
impl BrowserUploadScope {
    /// Trusted factory input: the already-authorized physical workspace root.
    pub fn open(root: &Path) -> Result<Self, WorkspaceError> {
        if !root.is_absolute() {
            return Err(WorkspaceError::UploadPathDenied);
        }
        Ok(Self {
            root: Dir::open_ambient_dir(root, cap_std::ambient_authority())
                .map_err(|_| WorkspaceError::UploadPathDenied)?,
        })
    }
    pub async fn prepare(
        self: &Arc<Self>,
        paths: Vec<String>,
        cancel: CancellationToken,
    ) -> Result<Arc<PreparedBrowserUpload>, WorkspaceError> {
        let scope = self.clone();
        // The caller must await this owned worker even when cancellation occurs:
        // no detached copy may later publish files to a replacement Agent turn.
        tokio::task::spawn_blocking(move || scope.prepare_blocking(paths, &cancel))
            .await
            .map_err(|_| WorkspaceError::NativeCommandFailed)?
    }
    fn prepare_blocking(
        &self,
        paths: Vec<String>,
        cancel: &CancellationToken,
    ) -> Result<Arc<PreparedBrowserUpload>, WorkspaceError> {
        cancelled(cancel)?;
        if paths.is_empty() {
            return Err(WorkspaceError::UnsupportedAction);
        }
        if paths.len() > MAX_UPLOAD_FILES {
            return Err(WorkspaceError::UploadLimit);
        }
        let paths = paths
            .iter()
            .map(|path| relative_components(path))
            .collect::<Result<Vec<_>, _>>()?;
        let directory = tempfile::Builder::new()
            .prefix("nomifun-browser-upload-")
            .tempdir()
            .map_err(|_| WorkspaceError::NativeCommandFailed)?;
        let mut prepared = PreparedBrowserUpload {
            directory,
            paths: vec![],
            total_bytes: 0,
        };
        for (index, components) in paths.iter().enumerate() {
            cancelled(cancel)?;
            let (name, parents) = components
                .split_last()
                .ok_or(WorkspaceError::UploadPathDenied)?;
            let mut parent = self
                .root
                .try_clone()
                .map_err(|_| WorkspaceError::UploadPathDenied)?;
            for component in parents {
                parent = parent
                    .open_dir_nofollow(component)
                    .map_err(|_| WorkspaceError::UploadPathDenied)?;
            }
            let mut options = OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            #[cfg(unix)]
            {
                use cap_std::fs::OpenOptionsExt;
                // A raced-in FIFO must not block the worker before type checking.
                options.custom_flags(libc::O_NONBLOCK);
            }
            let mut source = parent
                .open_with(name, &options)
                .map_err(|_| WorkspaceError::UploadPathDenied)?
                .into_std();
            let metadata = source
                .metadata()
                .map_err(|_| WorkspaceError::UploadPathDenied)?;
            if !metadata.is_file() {
                return Err(WorkspaceError::UploadPathDenied);
            }
            if metadata.len() > MAX_UPLOAD_BYTES.saturating_sub(prepared.total_bytes) {
                return Err(WorkspaceError::UploadLimit);
            }
            let staging = prepared.directory.path().join(index.to_string());
            std::fs::create_dir(&staging).map_err(|_| WorkspaceError::NativeCommandFailed)?;
            let destination = staging.join(name);
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&destination)
                .map_err(|_| WorkspaceError::NativeCommandFailed)?;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                cancelled(cancel)?;
                let read = source
                    .read(&mut buffer)
                    .map_err(|_| WorkspaceError::UploadPathDenied)?;
                if read == 0 {
                    break;
                }
                prepared.total_bytes = prepared
                    .total_bytes
                    .checked_add(read as u64)
                    .ok_or(WorkspaceError::UploadLimit)?;
                if prepared.total_bytes > MAX_UPLOAD_BYTES {
                    return Err(WorkspaceError::UploadLimit);
                }
                output
                    .write_all(&buffer[..read])
                    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
            }
            output
                .sync_all()
                .map_err(|_| WorkspaceError::NativeCommandFailed)?;
            let after = source
                .metadata()
                .map_err(|_| WorkspaceError::UploadPathDenied)?;
            if after.len() != metadata.len() || after.modified().ok() != metadata.modified().ok() {
                return Err(WorkspaceError::UploadPathDenied);
            }
            output
                .set_modified(
                    metadata
                        .modified()
                        .map_err(|_| WorkspaceError::UploadPathDenied)?,
                )
                .map_err(|_| WorkspaceError::NativeCommandFailed)?;
            prepared.paths.push(destination);
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
        for path in [
            "../secret",
            "C:/secret",
            "/etc/passwd",
            "file:stream",
            "NUL.txt",
            "COM1",
            "a/../../secret",
            "./file",
            "file.",
            "file ",
        ] {
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
    async fn upload_rejects_symlinks_even_when_the_target_is_inside() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("source"), b"x").unwrap();
        std::os::unix::fs::symlink(root.path().join("source"), root.path().join("link")).unwrap();
        let scope = Arc::new(BrowserUploadScope::open(root.path()).unwrap());
        assert_eq!(
            scope
                .prepare(vec!["link".into()], CancellationToken::new())
                .await
                .err(),
            Some(WorkspaceError::UploadPathDenied)
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn upload_rejects_windows_junction_escape() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), b"not authorized").unwrap();
        junction::create(outside.path(), root.path().join("escape")).unwrap();
        let scope = Arc::new(BrowserUploadScope::open(root.path()).unwrap());
        assert_eq!(
            scope
                .prepare(vec!["escape/secret.txt".into()], CancellationToken::new())
                .await
                .err(),
            Some(WorkspaceError::UploadPathDenied)
        );
        assert_eq!(
            std::fs::read(outside.path().join("secret.txt")).unwrap(),
            b"not authorized"
        );
    }
}
