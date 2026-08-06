//! `SshBackend`: the seam through which the agent's remote tool family reaches a
//! live SSH connection. The trait lives here in `nomi-agent`; the implementation
//! lives in `crates/backend/nomifun-ssh` and is reached through the
//! `nomifun-ai-agent` re-export — so `nomi-agent` never depends on russh or the
//! backend.
//!
//! Shape follows the established Sink pattern: `#[async_trait]`, `&self`, owned
//! `Result<_, String>` returns, no `connect()` on the trait. Connection
//! identity, credential decryption, pooling and keepalive all stay behind the
//! implementation; the model never sees or supplies them.
use std::sync::Arc;

use async_trait::async_trait;

/// Result of running a remote command.
#[derive(Debug, Clone)]
pub struct RemoteCommandOutput {
    pub stdout: String,
    pub exit_code: i32,
    /// True when the command was interrupted by the timeout budget.
    pub timed_out: bool,
}

/// Metadata about a remote path.
#[derive(Debug, Clone)]
pub struct RemoteFileStat {
    pub size: u64,
    pub is_dir: bool,
}

/// Remote operations backing the SSH tool family. Each call operates on the one
/// host bound to the conversation; the binding is baked into the implementation
/// at construction, never passed by the model.
#[async_trait]
pub trait SshBackend: Send + Sync {
    /// Run a shell command on the remote host, capturing combined output, exit
    /// code, and whether it timed out. `timeout_ms` bounds the command.
    async fn run_command(&self, command: &str, timeout_ms: u64)
        -> Result<RemoteCommandOutput, String>;

    /// Read an entire remote file.
    async fn read_file(&self, path: &str) -> Result<Vec<u8>, String>;

    /// Atomically write bytes to a remote file.
    async fn write_file(&self, path: &str, bytes: Vec<u8>) -> Result<(), String>;

    /// Search for `pattern` under `path` on the remote host (ripgrep if present,
    /// else grep), returning matching lines.
    async fn grep(&self, pattern: &str, path: &str) -> Result<String, String>;

    /// List remote entries matching a glob (relative to `cwd`).
    async fn list_files(&self, glob: &str) -> Result<Vec<String>, String>;

    /// Stat a remote path.
    async fn stat(&self, path: &str) -> Result<RemoteFileStat, String>;
}

/// What releasing a session lease could prove about the link behind it. Carried
/// into the runtime's teardown report, where only `Lost` counts as a failure.
#[derive(Debug, Clone)]
pub enum SshLeaseRelease {
    /// The link is still up and was deliberately kept for the conversation.
    Retained { detail: String },
    /// The link was closed and the remote shell was provably reaped.
    Reaped { detail: String },
    /// The link is gone with no proof the remote shell died. A real failure.
    Lost { detail: String },
}

/// One agent runtime's claim on a live remote session.
///
/// `release` **must not close the link.** The runtime that holds a lease is
/// destroyed and rebuilt on every model switch
/// (`AgentKillReason::ConfigurationChanged`); a lease that closed on release
/// would drop the operator's shell — its cwd, its exported environment, its
/// passphrase prompt — each time they touch the model picker. That is exactly
/// what the connection pool exists to prevent, so `release` only *reports* what
/// is already true about the link. Closing is the pool's decision (conversation
/// deleted, host withdrawn, process shutting down), never a lease's.
#[async_trait]
pub trait SshSessionLease: Send + Sync {
    async fn release(&self) -> SshLeaseRelease;
}

/// What a provider hands back: the tools' backend plus the runtime's claim on the
/// session. Kept as one value so a caller cannot take the backend and forget the
/// lease — then nothing would ever report on the link at teardown.
pub struct SshSessionBinding {
    pub backend: Arc<dyn SshBackend>,
    pub lease: Arc<dyn SshSessionLease>,
}

/// Connects a conversation to its bound SSH host and returns a ready
/// `SshBackend`. This is the seam the agent factory calls when a session's
/// `extra` carries an `ssh_host_id`; the implementation (in `nomifun-ssh`)
/// decrypts the stored credential, dials, and opens the shell + SFTP. Kept
/// separate from `SshBackend` so the factory can request a connection without
/// `nomi-agent`/`nomifun-ai-agent` depending on the transport crate (that would
/// be a dependency cycle — the transport crate already depends on the seam).
#[async_trait]
pub trait SshBackendProvider: Send + Sync {
    /// Build a live backend for `ssh_host_id` owned by `user_id`, rooted at
    /// `remote_cwd` on the remote host. `conversation_id` identifies the session
    /// the link belongs to: the implementation pools one link per
    /// (conversation, host), so a rebuilt runtime rejoins the session its
    /// predecessor was using instead of dialling a second time. Errors are
    /// surfaced to the session build.
    async fn connect(
        &self,
        user_id: &str,
        conversation_id: &str,
        ssh_host_id: &str,
        remote_cwd: &str,
    ) -> Result<SshSessionBinding, String>;
}

