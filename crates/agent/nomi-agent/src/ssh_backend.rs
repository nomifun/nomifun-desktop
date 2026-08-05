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
