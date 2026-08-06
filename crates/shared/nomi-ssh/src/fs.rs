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
        Ok(self.sftp.read(path).await?)
    }

    /// Stat a remote path.
    pub async fn stat(&self, path: &str) -> Result<FileStat, SshError> {
        let m = self.sftp.metadata(path).await?;
        Ok(FileStat {
            size: m.size.unwrap_or(0),
            mtime: m.mtime.map(|t| t as i64).unwrap_or(0),
            is_dir: m.is_dir(),
        })
    }

    /// List a remote directory, returning entry names (not full paths).
    pub async fn list_dir(&self, path: &str) -> Result<Vec<String>, SshError> {
        let read_dir = self.sftp.read_dir(path).await?;
        Ok(read_dir.map(|entry| entry.file_name()).collect())
    }

    /// Resolve a remote path to its canonical absolute form.
    pub async fn canonicalize(&self, path: &str) -> Result<String, SshError> {
        Ok(self.sftp.canonicalize(path).await?)
    }

    /// Atomically write `bytes` to `path`: write a sibling temp file, preserve
    /// the target's existing permission bits if it exists, then rename over it.
    /// On an SFTP v3 server that rejects rename-onto-existing, falls back to
    /// remove+rename (a small non-atomic window, logged).
    pub async fn write_file_atomic(&self, path: &str, bytes: &[u8]) -> Result<(), SshError> {
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
        Ok(self.sftp.remove_file(path).await?)
    }
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
    use super::split_parent;

    #[test]
    fn split_parent_handles_posix_paths() {
        assert_eq!(split_parent("/srv/www/app.txt"), ("/srv/www", "app.txt"));
        assert_eq!(split_parent("/top.txt"), ("/", "top.txt"));
        assert_eq!(split_parent("bare.txt"), (".", "bare.txt"));
    }
}
