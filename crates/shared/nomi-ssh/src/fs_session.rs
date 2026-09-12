//! A single SFTP channel, taken out of its reusable slot for each operation.
//! Failure or cancellation drops the channel (including any remote handles);
//! only a fully successful operation returns it for reuse. The next operation
//! reopens SFTP on the same authenticated SSH connection, not a second socket.
use std::{future::Future, pin::Pin};

use russh_sftp::{
    client::{RawSftpSession, error::Error, rawsession::Limits},
    extensions,
    protocol::{Packet, StatusCode},
};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::connection::SshError;

#[path = "fs_stream.rs"]
mod stream;

pub(super) type Connector =
    Box<dyn Fn() -> Pin<Box<dyn Future<Output = Result<Session, SshError>> + Send>> + Send + Sync>;

pub(super) struct Session {
    pub raw: RawSftpSession,
    // Drop after raw queues its close frame; cancellation also wakes a writer
    // unable to reach that queued frame and releases the actual channel stream.
    _stream_guard: tokio_util::sync::DropGuard,
    pub fsync: bool,
    posix_rename: bool,
    limits: Limits,
}

impl Session {
    pub async fn new<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
        stream: S,
    ) -> Result<Self, SshError> {
        let (stream, guard) = stream::bounded_stream(stream);
        let mut raw = RawSftpSession::new(stream);
        let version = raw.init().await?;
        let has = |name| version.extensions.get(name).is_some_and(|v| v == "1");
        let limits = if has(extensions::LIMITS) {
            let limits = Limits::from(raw.limits().await?);
            raw.set_limits(limits);
            limits
        } else {
            Limits::default()
        };
        Ok(Self {
            raw,
            _stream_guard: guard,
            fsync: has(extensions::FSYNC),
            posix_rename: has("posix-rename@openssh.com"),
            limits,
        })
    }

    pub fn chunk_len(&self, handle: &str, writing: bool) -> Result<usize, SshError> {
        // Leave room for packet framing, request id, handle and offset. Cap
        // locally even when the server advertises very large/unlimited reads.
        let payload = self
            .limits
            .packet_len
            .unwrap_or(32 * 1024 + 64)
            .saturating_sub(64 + handle.len() as u64);
        let limit = if writing {
            self.limits.write_len
        } else {
            self.limits.read_len
        };
        let chunk = payload.min(limit.unwrap_or(u64::MAX)).min(32 * 1024) as usize;
        if chunk == 0 {
            return Err(SshError::Protocol(
                "sftp: server packet limit cannot hold file data".into(),
            ));
        }
        Ok(chunk)
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<(), SshError> {
        if !self.posix_rename {
            // SFTP v3 may create a new destination but must not remove an
            // existing one. Unsupported atomic replacement is an error.
            self.raw.rename(from, to).await?;
            return Ok(());
        }
        // posix-rename has exactly two SFTP strings (u32 byte length + bytes).
        let mut data = Vec::with_capacity(8 + from.len() + to.len());
        for path in [from, to] {
            data.extend_from_slice(&(path.len() as u32).to_be_bytes());
            data.extend_from_slice(path.as_bytes());
        }
        match self.raw.extended("posix-rename@openssh.com", data).await? {
            Packet::Status(status) if status.status_code == StatusCode::Ok => Ok(()),
            Packet::Status(status) => Err(Error::Status(status).into()),
            _ => Err(Error::UnexpectedPacket.into()),
        }
    }
}

pub(super) fn is_status(error: &Error, expected: StatusCode) -> bool {
    matches!(error, Error::Status(status) if status.status_code == expected)
}
