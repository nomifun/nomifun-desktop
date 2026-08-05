//! HTTP DTOs for the SSH host book.
//!
//! Request DTOs are `Deserialize`-only with `deny_unknown_fields`. Response
//! DTOs never carry credential plaintext or ciphertext: a stored secret is
//! surfaced only as the masked sentinel `"***"`, which the client uses to know
//! "this secret is set; don't resend it on update". Unlike `remote_agents`'
//! `mask_token`, we deliberately do NOT reveal a last-4 suffix — that would
//! require decrypting every secret just to render a list, a needless exposure.
use nomifun_common::TimestampMs;
use nomifun_db::SshHostRow;
use serde::{Deserialize, Serialize};

/// The masked placeholder for a stored secret. The client omits any credential
/// field still equal to this on update (the value is unchanged).
pub const SECRET_MASK: &str = "***";

/// Owner-visible view of a saved SSH host. No credential material — only
/// whether each secret is set (masked).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshHostResponse {
    pub ssh_host_id: String,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub username: String,
    pub auth_type: String,
    /// `Some("***")` if a password is stored, else `None`.
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub passphrase: Option<String>,
    pub certificate: Option<String>,
    pub sudo_password: Option<String>,
    pub host_fingerprint: Option<String>,
    pub status: String,
    pub last_connected_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

fn mask(stored: &Option<String>) -> Option<String> {
    stored.as_ref().map(|_| SECRET_MASK.to_string())
}

impl From<SshHostRow> for SshHostResponse {
    fn from(r: SshHostRow) -> Self {
        SshHostResponse {
            ssh_host_id: r.ssh_host_id,
            name: r.name,
            host: r.host,
            port: r.port,
            username: r.username,
            auth_type: r.auth_type,
            password: mask(&r.password_encrypted),
            private_key: mask(&r.private_key_encrypted),
            passphrase: mask(&r.passphrase_encrypted),
            certificate: mask(&r.certificate_encrypted),
            sudo_password: mask(&r.sudo_password_encrypted),
            host_fingerprint: r.host_fingerprint,
            status: r.status,
            last_connected_at: r.last_connected_at,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// Create-host request. Credential fields are plaintext here (they are
/// encrypted in the service before storage) and never echoed back.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateSshHostRequest {
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: i64,
    pub username: String,
    /// "password" | "key" | "certificate" | "agent".
    pub auth_type: String,
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub passphrase: Option<String>,
    pub certificate: Option<String>,
    pub sudo_password: Option<String>,
}

/// Update-host request. A `None` field is left unchanged. A credential field
/// equal to the mask (`"***"`) is left unchanged; an empty string clears it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateSshHostRequest {
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<i64>,
    pub username: Option<String>,
    pub auth_type: Option<String>,
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub passphrase: Option<String>,
    pub certificate: Option<String>,
    pub sudo_password: Option<String>,
}

fn default_port() -> i64 {
    22
}
