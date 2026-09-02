//! `RemoteFs`: SFTP-backed file operations for the remote host. Every file
//! read/write/edit goes through SFTP rather than shell string-building — the
//! single highest-leverage safety decision, since shell-composed file edits are
//! the source of most published command-injection CVEs in coding agents.
//!
//! Writes are atomic: a temp file in the same directory, permission-preserved,
//! then renamed over the target (with a remove+rename fallback for SFTP v3
//! servers that reject rename-onto-existing).
use russh_sftp::client::SftpSession;

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
    sftp: SftpSession,
}

impl From<russh_sftp::client::error::Error> for SshError {
    fn from(e: russh_sftp::client::error::Error) -> Self {
        SshError::Protocol(format!("sftp: {e}"))
    }
}

impl SshConnection {
    /// Open the SFTP subsystem on a fresh channel off this connection.
    pub async fn open_sftp(&self) -> Result<RemoteFs, SshError> {
        let channel = self.handle().channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        let sftp = SftpSession::new(channel.into_stream()).await?;
        Ok(RemoteFs { sftp })
    }
}

impl RemoteFs {
    /// Read an entire remote file.
    pub async fn read_file(&self, path: &str) -> Result<Vec<u8>, SshError> {
        validate_path_input(path)?;
        match tokio::time::timeout(SSH_OPERATION_TIMEOUT, self.sftp.metadata(path)).await {
            Ok(Ok(metadata)) => {
                let size = usize::try_from(metadata.size.unwrap_or(0)).unwrap_or(usize::MAX);
                validate_output_size(size).map_err(limit_error)?;
            }
            Ok(Err(_)) => {}
            Err(_) => return Err(operation_timeout("SFTP read metadata")),
        }
        let read = async {
            use tokio::io::AsyncReadExt;

            let mut file = self.sftp.open(path).await?;
            let mut bytes = Vec::new();
            (&mut file)
                .take((MAX_SSH_OUTPUT_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .await
                .map_err(|e| SshError::Protocol(format!("sftp read: {e}")))?;
            file.close()
                .await
                .map_err(|e| SshError::Protocol(format!("sftp close after read: {e}")))?;
            Ok::<Vec<u8>, SshError>(bytes)
        };
        let bytes = tokio::time::timeout(SSH_OPERATION_TIMEOUT, read)
            .await
            .map_err(|_| {
                SshError::TimedOut(format!(
                    "SFTP read exceeded {}ms",
                    SSH_OPERATION_TIMEOUT.as_millis()
                ))
            })??;
        validate_output_size(bytes.len()).map_err(limit_error)?;
        Ok(bytes)
    }

    /// Stat a remote path.
    pub async fn stat(&self, path: &str) -> Result<FileStat, SshError> {
        validate_path_input(path)?;
        let m = tokio::time::timeout(SSH_OPERATION_TIMEOUT, self.sftp.metadata(path))
            .await
            .map_err(|_| operation_timeout("SFTP stat"))??;
        Ok(FileStat {
            size: m.size.unwrap_or(0),
            mtime: m.mtime.map(|t| t as i64).unwrap_or(0),
            is_dir: m.is_dir(),
        })
    }

    /// List a remote directory, returning entry names (not full paths).
    pub async fn list_dir(&self, path: &str) -> Result<Vec<String>, SshError> {
        validate_path_input(path)?;
        let read_dir = tokio::time::timeout(SSH_OPERATION_TIMEOUT, self.sftp.read_dir(path))
            .await
            .map_err(|_| operation_timeout("SFTP directory read"))??;
        let mut total = 0usize;
        let mut entries = Vec::new();
        for entry in read_dir {
            let name = entry.file_name();
            total = total.saturating_add(name.len());
            validate_output_size(total).map_err(limit_error)?;
            entries.push(name);
        }
        Ok(entries)
    }

    /// Resolve a remote path to its canonical absolute form.
    pub async fn canonicalize(&self, path: &str) -> Result<String, SshError> {
        validate_path_input(path)?;
        let canonical =
            tokio::time::timeout(SSH_OPERATION_TIMEOUT, self.sftp.canonicalize(path))
                .await
                .map_err(|_| operation_timeout("SFTP canonicalize"))??;
        validate_path_input(&canonical)?;
        Ok(canonical)
    }

    /// Atomically write `bytes` to `path`: write a sibling temp file, preserve
    /// the target's existing permission bits if it exists, then rename over it.
    /// On an SFTP v3 server that rejects rename-onto-existing, falls back to
    /// remove+rename (a small non-atomic window, logged).
    pub async fn write_file_atomic(&self, path: &str, bytes: &[u8]) -> Result<(), SshError> {
        validate_write_input(path, bytes)?;
        tokio::time::timeout(SSH_OPERATION_TIMEOUT, self.write_file_atomic_inner(path, bytes))
            .await
            .map_err(|_| {
                SshError::TimedOut(format!(
                    "SFTP atomic write exceeded {}ms",
                    SSH_OPERATION_TIMEOUT.as_millis()
                ))
            })?
    }

    async fn write_file_atomic_inner(&self, path: &str, bytes: &[u8]) -> Result<(), SshError> {
        // Preserve existing permissions if the target already exists.
        let existing_perms = match self.sftp.metadata(path).await {
            Ok(m) => m.permissions,
            Err(_) => None,
        };

        let (dir, _file) = split_parent(path);
        let tmp = format!("{dir}/.nomi-tmp-{}", nonce());

        // `SftpSession::write` opens with WRITE only (no CREATE) and fails on a
        // new path; create + write_all + fsync the temp file explicitly.
        {
            use tokio::io::AsyncWriteExt;
            // This step fails on the *temp* path, so a missing parent directory
            // surfaces as a bare "no such file" about a file the caller never
            // named — which reads as "the target file does not exist yet" and
            // invites exactly the wrong fix. Say where the failure happened and
            // what has to be true of that directory.
            let mut file = self.sftp.create(&tmp).await.map_err(|e| {
                SshError::Protocol(format!(
                    "sftp: cannot create the temporary file {tmp} that an atomic write of {path} \
                     needs: {e} — the directory {dir} has to exist already and be writable by \
                     this user (create it with `mkdir -p {dir}`); the target file itself does not \
                     have to exist"
                ))
            })?;
            file.write_all(bytes).await.map_err(|e| SshError::Protocol(format!("sftp write: {e}")))?;
            file.sync_all().await?;
            file.shutdown().await.ok();
        }

        if let Some(perms) = existing_perms {
            let attrs = russh_sftp::protocol::FileAttributes {
                permissions: Some(perms),
                ..Default::default()
            };
            // Best-effort: a server may reject setstat; the write already succeeded.
            let _ = self.sftp.set_metadata(&tmp, attrs).await;
        }

        match self.sftp.rename(&tmp, path).await {
            Ok(()) => Ok(()),
            Err(_) => {
                // SFTP v3 rename fails if the destination exists; remove and retry.
                tracing::warn!(
                    "sftp rename onto existing failed; using non-atomic remove+rename for {path}"
                );
                let _ = self.sftp.remove_file(path).await;
                self.sftp.rename(&tmp, path).await?;
                Ok(())
            }
        }
    }

    /// Remove a remote file (used by callers cleaning up temp artifacts).
    pub async fn remove_file(&self, path: &str) -> Result<(), SshError> {
        validate_path_input(path)?;
        tokio::time::timeout(SSH_OPERATION_TIMEOUT, self.sftp.remove_file(path))
            .await
            .map_err(|_| operation_timeout("SFTP remove"))??;
        Ok(())
    }
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
