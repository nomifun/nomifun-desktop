//! `RemoteFs`: SFTP-backed file operations for the remote host. Every file
//! read/write/edit goes through SFTP rather than shell string-building — the
//! single highest-leverage safety decision, since shell-composed file edits are
//! the source of most published command-injection CVEs in coding agents.
//!
//! Writes are atomic: a temp file in the same directory, permission-preserved,
//! then published with the server's atomic rename operation. Servers without
//! atomic replacement support return an error without deleting the target.
use std::{future::Future, sync::Arc};
use futures_util::{TryStreamExt, stream};
use russh_sftp::protocol::{FileAttributes, OpenFlags, StatusCode};
use tokio::sync::Mutex;

#[path = "fs_session.rs"]
mod session;
use session::{Connector, Session, is_status};

use crate::connection::{SshConnection, SshError};
use crate::limits::{
    MAX_SSH_OUTPUT_BYTES, SSH_OPERATION_TIMEOUT, validate_output_size, validate_path,
    validate_write_payload,
};

/// Non-secret metadata about a remote path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStat {
    pub size: u64,
    /// Modification time (unix seconds); 0 if the server did not report it.
    pub mtime: i64,
    pub is_dir: bool,
}

/// SFTP session bound to one SSH connection. Cheap to hold; open one per
/// connection and reuse.
pub struct RemoteFs {
    session: Mutex<Option<Session>>,
    connect: Connector,
}

impl From<russh_sftp::client::error::Error> for SshError {
    fn from(e: russh_sftp::client::error::Error) -> Self {
        SshError::Protocol(format!("sftp: {e}"))
    }
}

impl SshConnection {
    /// Open the SFTP subsystem on a fresh channel off this connection.
    pub async fn open_sftp(&self) -> Result<RemoteFs, SshError> {
        let handle = Arc::clone(self.handle());
        let connect: Connector = Box::new(move || {
            let handle = Arc::clone(&handle);
            Box::pin(async move {
                let channel = handle.channel_open_session().await?;
                channel.request_subsystem(true, "sftp").await?;
                Session::new(channel.into_stream()).await
            })
        });
        let session = tokio::time::timeout(SSH_OPERATION_TIMEOUT, connect())
            .await.map_err(|_| operation_timeout("SFTP open"))??;
        Ok(RemoteFs { session: Mutex::new(Some(session)), connect })
    }
}

impl RemoteFs {
    /// One deadline includes admission, channel recovery and every request.
    /// Taking the session out of the slot makes dropping this future close the
    /// channel, even if OPEN/CLOSE was sent but its response was never received.
    async fn run<T, F>(&self, operation: &str, action: impl FnOnce(Session) -> F) -> Result<T, SshError>
    where
        F: Future<Output = (Session, Result<T, SshError>)>,
    {
        tokio::time::timeout(SSH_OPERATION_TIMEOUT, async {
            let mut slot = self.session.lock().await;
            let session = match slot.take() {
                Some(session) => session,
                None => (self.connect)().await?,
            };
            let (session, result) = action(session).await;
            if result.is_ok() {
                *slot = Some(session);
            }
            result
        }).await.map_err(|_| operation_timeout(operation))?
    }

    /// Read an entire remote file, within one operation budget and output cap.
    pub async fn read_file(&self, path: &str) -> Result<Vec<u8>, SshError> {
        validate_path_input(path)?;
        self.run("SFTP read", |session| async move {
            let result = async {
                if let Ok(metadata) = session.raw.stat(path).await {
                    let size = usize::try_from(metadata.attrs.size.unwrap_or(0)).unwrap_or(usize::MAX);
                    validate_output_size(size).map_err(limit_error)?;
                }
                let handle = session.raw.open(path, OpenFlags::READ, FileAttributes::empty()).await?.handle;
                let chunk_len = session.chunk_len(&handle, false)?;
                let mut bytes = Vec::new();
                loop {
                    let length = chunk_len.min(MAX_SSH_OUTPUT_BYTES + 1 - bytes.len());
                    match session.raw.read(&handle, bytes.len() as u64, length as u32).await {
                        Ok(data) => {
                            if data.data.is_empty() || data.data.len() > length {
                                return Err(SshError::Protocol("sftp: invalid read response length".into()));
                            }
                            bytes.extend_from_slice(&data.data);
                            validate_output_size(bytes.len()).map_err(limit_error)?;
                        }
                        Err(error) if is_status(&error, StatusCode::Eof) => break,
                        Err(error) => return Err(error.into()),
                    }
                }
                session.raw.close(handle).await?;
                Ok(bytes)
            }.await;
            (session, result)
        }).await
    }

    /// Stat a remote path.
    pub async fn stat(&self, path: &str) -> Result<FileStat, SshError> {
        validate_path_input(path)?;
        self.run("SFTP stat", |session| async move {
            let result = session.raw.stat(path).await.map(|m| FileStat {
                size: m.attrs.size.unwrap_or(0),
                mtime: m.attrs.mtime.map(i64::from).unwrap_or(0),
                is_dir: m.attrs.is_dir(),
            }).map_err(SshError::from);
            (session, result)
        }).await
    }

    /// List entry names, checking the budget before accumulating each batch.
    pub async fn list_dir(&self, path: &str) -> Result<Vec<String>, SshError> {
        validate_path_input(path)?;
        self.run("SFTP directory read", |session| async move {
            let result = async {
                let handle = session.raw.opendir(path).await?.handle;
                let mut total = 0usize;
                let mut entries = Vec::new();
                loop {
                    match session.raw.readdir(&handle).await {
                        Ok(batch) => {
                            if batch.files.is_empty() {
                                return Err(SshError::Protocol("sftp: empty directory response without EOF".into()));
                            }
                            for entry in batch.files {
                                // Account for storage even for empty names or dot entries.
                                total = total.saturating_add(entry.filename.len())
                                    .saturating_add(std::mem::size_of::<String>());
                                validate_output_size(total).map_err(limit_error)?;
                                if entry.filename != "." && entry.filename != ".." {
                                    entries.push(entry.filename);
                                }
                            }
                        }
                        Err(error) if is_status(&error, StatusCode::Eof) => break,
                        Err(error) => return Err(error.into()),
                    }
                }
                session.raw.close(handle).await?;
                Ok(entries)
            }.await;
            (session, result)
        }).await
    }

    /// Resolve a remote path to its canonical absolute form.
    pub async fn canonicalize(&self, path: &str) -> Result<String, SshError> {
        validate_path_input(path)?;
        self.run("SFTP canonicalize", |session| async move {
            let result = async {
                let canonical = session.raw.realpath(path).await?.files.into_iter().next()
                    .ok_or_else(|| SshError::Protocol("sftp: no canonical path returned".into()))?.filename;
                validate_path_input(&canonical)?;
                Ok(canonical)
            }.await;
            (session, result)
        }).await
    }

    /// Atomically publish a sibling temporary file, preserving existing mode
    /// bits. Without server support, replacement fails without deleting the
    /// original. New files start private (0600). Interruption may leave a temp
    /// file; if RENAME was already sent, the publication outcome is uncertain.
    pub async fn write_file_atomic(&self, path: &str, bytes: &[u8]) -> Result<(), SshError> {
        validate_write_input(path, bytes)?;
        self.run("SFTP atomic write", |session| async move {
            let result = write_file_atomic_inner(&session, path, bytes).await;
            (session, result)
        }).await
    }

    /// Remove a remote file (used by callers cleaning up temp artifacts).
    pub async fn remove_file(&self, path: &str) -> Result<(), SshError> {
        validate_path_input(path)?;
        self.run("SFTP remove", |session| async move {
            let result = session.raw.remove(path).await.map(|_| ()).map_err(SshError::from);
            (session, result)
        }).await
    }
}

async fn write_file_atomic_inner(session: &Session, path: &str, bytes: &[u8]) -> Result<(), SshError> {
    let existing_perms = match session.raw.stat(path).await {
        Ok(m) => Some(m.attrs.permissions.ok_or_else(|| {
            SshError::Protocol("sftp: server omitted the existing file permissions".into())
        })? & 0o7777),
        Err(error) if is_status(&error, StatusCode::NoSuchFile) => None,
        Err(error) => return Err(error.into()),
    };
    let (dir, _) = split_parent(path);
    let tmp = format!("{}/.nomi-tmp-{}", dir.trim_end_matches('/'), nonce());
    // EXCLUDE is the dependency's spelling of SFTP EXCL. Never truncate a
    // collision, and never remove a path unless our exclusive OPEN succeeded.
    let handle = session.raw.open(
        &tmp, OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE,
        FileAttributes { permissions: Some(0o600), ..Default::default() },
    ).await.map_err(|e| SshError::Protocol(format!(
        "sftp: cannot create the temporary file {tmp} that an atomic write of {path} needs: {e} — the directory {dir} must exist and be writable (mkdir -p); the temporary path must be unused"
    )))?.handle;
    let result = async {
        let chunk_len = session.chunk_len(&handle, true)?;
        // Keep the high-level client's eight-request write window, but all
        // futures stay owned by this operation and finish before publication.
        let handle = handle.as_str();
        stream::iter(bytes.chunks(chunk_len).enumerate().map(Ok::<_, SshError>))
            .try_for_each_concurrent(8, |(index, chunk)| async move {
                session.raw.write(handle, (index * chunk_len) as u64, chunk.to_vec()).await?;
                Ok(())
            }).await?;
        if let Some(permissions) = existing_perms {
            session.raw.fsetstat(handle, FileAttributes {
                permissions: Some(permissions), ..Default::default()
            }).await?;
        }
        if session.fsync {
            session.raw.fsync(handle).await?;
        }
        session.raw.close(handle).await?;
        session.rename(&tmp, path).await
    }.await;
    if result.is_err() {
        // A failure retires this channel, releasing any still-open handles.
        // Best-effort unlink is only for our confirmed exclusive temporary file.
        if let Err(error) = session.raw.remove(&tmp).await {
            tracing::warn!("sftp: could not clean up temporary file {tmp}: {error}");
        }
    }
    result
}

fn validate_path_input(path: &str) -> Result<(), SshError> {
    validate_path(path).map_err(limit_error)
}

fn validate_write_input(path: &str, bytes: &[u8]) -> Result<(), SshError> {
    validate_path_input(path)?;
    validate_write_payload(bytes).map_err(limit_error)
}

fn limit_error(error: crate::limits::LimitError) -> SshError {
    // Keep admission failures in the transport's typed error channel.
    SshError::InvalidInput(error.to_string())
}

fn operation_timeout(operation: &str) -> SshError {
    SshError::TimedOut(format!(
        "{operation} exceeded {}ms",
        SSH_OPERATION_TIMEOUT.as_millis()
    ))
}

/// Split `path` into (parent_dir, file_name). Remote paths are always POSIX.
fn split_parent(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(0) => ("/", &path[1..]),
        Some(i) => (&path[..i], &path[i + 1..]),
        None => (".", path),
    }
}

/// A per-write nonce for the temp filename. Uses a process-wide counter plus the
/// pid so concurrent writers on one host don't collide; no randomness crate.
fn nonce() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    format!("{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
#[path = "fs_publication_tests.rs"]
mod publication_tests;

#[cfg(test)]
#[path = "fs_stream_tests.rs"]
mod stream_tests;

#[cfg(test)]
mod tests {
    use super::{split_parent, validate_path_input, validate_write_input};
    use crate::connection::SshError;
    use crate::limits::{
        MAX_SSH_OUTPUT_BYTES, MAX_SSH_PATH_BYTES, MAX_SSH_WRITE_BYTES,
    };

    #[test]
    fn split_parent_handles_posix_paths() {
        assert_eq!(split_parent("/srv/www/app.txt"), ("/srv/www", "app.txt"));
        assert_eq!(split_parent("/top.txt"), ("/", "top.txt"));
        assert_eq!(split_parent("bare.txt"), (".", "bare.txt"));
    }

    #[test]
    fn operation_limits_cover_fs_inputs_and_results() {
        assert!(validate_path_input("/tmp/a").is_ok());
        assert!(validate_write_input("/tmp/a", &[]).is_ok());
        assert!(matches!(
            validate_path_input(&"x".repeat(MAX_SSH_PATH_BYTES + 1)),
            Err(SshError::InvalidInput(_))
        ));
        assert!(matches!(
            validate_write_input("/tmp/a", &vec![0; MAX_SSH_WRITE_BYTES + 1]),
            Err(SshError::InvalidInput(_))
        ));
        assert!(crate::limits::validate_output_size(MAX_SSH_OUTPUT_BYTES + 1).is_err());
    }
}
