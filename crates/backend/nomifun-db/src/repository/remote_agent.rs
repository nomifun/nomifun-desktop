use nomifun_common::{RemoteAgentId, TimestampMs};

use crate::error::DbError;
use crate::models::RemoteAgentRow;

/// Remote Agent configuration data access abstraction.
///
/// Provides CRUD operations and status management on the `remote_agents` table.
/// Sensitive fields (auth_token, device keys) are stored encrypted; callers
/// handle encryption/decryption before passing data in.
#[async_trait::async_trait]
pub trait IRemoteAgentRepository: Send + Sync {
    /// Returns all remote agents, ordered by creation time ascending.
    async fn list(&self) -> Result<Vec<RemoteAgentRow>, DbError>;

    /// Finds a remote agent by ID, or `None` if not found.
    async fn find_by_id(&self, id: &RemoteAgentId) -> Result<Option<RemoteAgentRow>, DbError>;

    /// Creates a new remote agent and returns the inserted row.
    async fn create(&self, params: CreateRemoteAgentParams<'_>) -> Result<RemoteAgentRow, DbError>;

    /// Updates an existing remote agent. Returns `DbError::NotFound` if the ID doesn't exist.
    async fn update(&self, id: &RemoteAgentId, params: UpdateRemoteAgentParams<'_>) -> Result<RemoteAgentRow, DbError>;

    /// Deletes a remote agent by ID. Returns `DbError::NotFound` if the ID doesn't exist.
    async fn delete(&self, id: &RemoteAgentId) -> Result<(), DbError>;

    /// Updates only the connection status (and optionally last_connected_at).
    /// Returns `DbError::NotFound` if the ID doesn't exist.
    async fn update_status(
        &self,
        id: &RemoteAgentId,
        status: &str,
        last_connected_at: Option<TimestampMs>,
    ) -> Result<(), DbError>;

    /// Stores the encrypted device token issued by an OpenClaw Gateway.
    /// Returns `DbError::NotFound` if the ID doesn't exist.
    async fn update_device_token(&self, id: &RemoteAgentId, device_token: Option<&str>) -> Result<(), DbError>;
}

/// Parameters for creating a new remote agent.
#[derive(Debug)]
pub struct CreateRemoteAgentParams<'a> {
    pub name: &'a str,
    pub protocol: &'a str,
    pub url: &'a str,
    pub auth_type: &'a str,
    pub auth_token: Option<&'a str>,
    pub allow_insecure: bool,
    pub avatar: Option<&'a str>,
    pub description: Option<&'a str>,
    pub device_id: Option<&'a str>,
    pub device_public_key: Option<&'a str>,
    pub device_private_key: Option<&'a str>,
    pub device_token: Option<&'a str>,
}

/// Parameters for updating an existing remote agent.
///
/// All fields are optional; `None` means "keep the current value".
/// For nullable fields, `Some(None)` means "clear the value" and
/// `Some(Some(v))` means "set to v".
#[derive(Debug, Default)]
pub struct UpdateRemoteAgentParams<'a> {
    pub name: Option<&'a str>,
    pub protocol: Option<&'a str>,
    pub url: Option<&'a str>,
    pub auth_type: Option<&'a str>,
    pub auth_token: Option<Option<&'a str>>,
    pub allow_insecure: Option<bool>,
    pub avatar: Option<Option<&'a str>>,
    pub description: Option<Option<&'a str>>,
}
