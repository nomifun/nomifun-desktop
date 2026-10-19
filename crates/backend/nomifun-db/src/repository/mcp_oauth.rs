use crate::{DbError, models::{McpOAuthClientRow, McpOAuthGrantRow}};
use nomifun_common::TimestampMs;

/// The OAuth authority supplies encrypted secret envelopes and coordinates
/// interactive attempts. This repository owns atomic registration/grant writes.
#[async_trait::async_trait]
pub trait IMcpOAuthRepository: Send + Sync {
    async fn get_client(&self, owner_id: &str, issuer: &str, profile_key: &str) -> Result<Option<McpOAuthClientRow>, DbError>;
    async fn get_client_by_ref(&self, owner_id: &str, client_ref: &str) -> Result<Option<McpOAuthClientRow>, DbError>;
    /// A new client_ref adds a registration. Existing registration identity is
    /// immutable; only its encrypted secret and secret expiry can be updated.
    async fn upsert_client(&self, params: McpOAuthClientParams) -> Result<McpOAuthClientRow, DbError>;
    async fn get_grant(&self, owner_id: &str, server_id: &str) -> Result<Option<McpOAuthGrantRow>, DbError>;
    /// Reauthorization replaces the grant with a fresh UUIDv7 and revision 1.
    async fn replace_grant(&self, params: McpOAuthGrantParams) -> Result<McpOAuthGrantRow, DbError>;
    /// Returns None when logout, replacement, or a competing refresh won. Never
    /// inserts a missing grant, so an in-flight refresh cannot undo logout.
    async fn compare_and_swap_grant(&self, params: McpOAuthGrantRefreshParams) -> Result<Option<McpOAuthGrantRow>, DbError>;
    async fn delete_grant(&self, owner_id: &str, server_id: &str) -> Result<(), DbError>;
}

#[derive(Clone)]
pub struct McpOAuthClientParams {
    pub client_ref: String,
    pub owner_id: String,
    pub issuer: String,
    pub profile_key: String,
    pub registration_kind: String,
    pub client_id: String,
    pub token_endpoint_auth_method: String,
    pub redirect_uris_json: String,
    pub client_secret_encrypted: Option<String>,
    pub client_secret_expires_at: Option<TimestampMs>,
}

#[derive(Clone)]
pub struct McpOAuthGrantParams {
    pub grant_id: String,
    pub owner_id: String,
    pub server_id: String,
    pub connection_config_ref: String,
    pub issuer: String,
    pub resource: String,
    pub client_ref: String,
    pub scopes_json: String,
    pub token_encrypted: String,
    pub expires_at: Option<TimestampMs>,
    pub obtained_at: TimestampMs,
}

#[derive(Clone)]
pub struct McpOAuthGrantRefreshParams {
    pub owner_id: String,
    pub grant_id: String,
    pub expected_revision: i64,
    pub scopes_json: String,
    pub token_encrypted: String,
    pub expires_at: Option<TimestampMs>,
    pub obtained_at: TimestampMs,
}
