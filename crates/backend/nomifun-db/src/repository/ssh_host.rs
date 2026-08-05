use nomifun_common::{SshHostId, TimestampMs};

use crate::error::DbError;
use crate::models::SshHostRow;

/// SSH host connection-profile data access. Every method takes `user_id` first
/// and filters by it, so a cross-owner id is indistinguishable from NotFound.
/// Credential columns are stored/returned as ciphertext; the service layer
/// handles encryption/decryption.
#[async_trait::async_trait]
pub trait ISshHostRepository: Send + Sync {
    /// All hosts owned by `user_id`, newest first.
    async fn list(&self, user_id: &str) -> Result<Vec<SshHostRow>, DbError>;

    /// One host by id, scoped to `user_id`; `None` if absent or owned by another.
    async fn find(&self, user_id: &str, id: &SshHostId) -> Result<Option<SshHostRow>, DbError>;

    /// Create a host owned by `user_id`; returns the inserted row.
    async fn create(&self, user_id: &str, params: CreateSshHostParams<'_>) -> Result<SshHostRow, DbError>;

    /// Update an owned host. `DbError::NotFound` if absent or not owned.
    async fn update(
        &self,
        user_id: &str,
        id: &SshHostId,
        params: UpdateSshHostParams<'_>,
    ) -> Result<SshHostRow, DbError>;

    /// Delete an owned host. `DbError::NotFound` if absent or not owned.
    async fn delete(&self, user_id: &str, id: &SshHostId) -> Result<(), DbError>;

    /// Update connection status and (optionally) last_connected_at + fingerprint.
    async fn update_status(
        &self,
        user_id: &str,
        id: &SshHostId,
        status: &str,
        last_connected_at: Option<TimestampMs>,
        host_fingerprint: Option<&str>,
    ) -> Result<(), DbError>;
}

/// Parameters for creating a host. Credential fields are pre-encrypted ciphertext.
#[derive(Debug, Default)]
pub struct CreateSshHostParams<'a> {
    pub name: &'a str,
    pub host: &'a str,
    pub port: i64,
    pub username: &'a str,
    pub auth_type: &'a str,
    pub password_encrypted: Option<&'a str>,
    pub private_key_encrypted: Option<&'a str>,
    pub passphrase_encrypted: Option<&'a str>,
    pub certificate_encrypted: Option<&'a str>,
    pub sudo_password_encrypted: Option<&'a str>,
}

/// Parameters for updating a host. `None` fields are left unchanged; credential
/// fields wrapped in `Some(None)` explicitly clear the stored secret.
#[derive(Debug, Default)]
pub struct UpdateSshHostParams<'a> {
    pub name: Option<&'a str>,
    pub host: Option<&'a str>,
    pub port: Option<i64>,
    pub username: Option<&'a str>,
    pub auth_type: Option<&'a str>,
    pub password_encrypted: Option<Option<&'a str>>,
    pub private_key_encrypted: Option<Option<&'a str>>,
    pub passphrase_encrypted: Option<Option<&'a str>>,
    pub certificate_encrypted: Option<Option<&'a str>>,
    pub sudo_password_encrypted: Option<Option<&'a str>>,
}
