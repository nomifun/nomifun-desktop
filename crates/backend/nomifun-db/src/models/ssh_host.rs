use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Row mapping for the `ssh_hosts` table — a saved, reusable SSH connection
/// profile owned by one user.
///
/// `auth_type` is stored as TEXT ("password" | "key" | "certificate" | "agent")
/// and converted by the service layer. All `*_encrypted` columns hold
/// AES-256-GCM ciphertext; the repository never encrypts or decrypts — callers
/// pass ciphertext in and receive ciphertext out.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SshHostRow {
    pub id: i64,
    pub ssh_host_id: String,
    pub user_id: String,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    /// One of: "password", "key", "certificate", "agent".
    pub auth_type: String,
    pub password_encrypted: Option<String>,
    pub private_key_encrypted: Option<String>,
    pub passphrase_encrypted: Option<String>,
    pub certificate_encrypted: Option<String>,
    pub sudo_password_encrypted: Option<String>,
    /// SHA256 host-key fingerprint recorded on first connect.
    pub host_fingerprint: Option<String>,
    /// One of: "unknown", "connected", "error".
    pub status: String,
    pub last_connected_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}
