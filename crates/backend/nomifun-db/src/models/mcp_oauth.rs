use nomifun_common::TimestampMs;

/// An authorization-server registration, independent of any user's grant.
/// Secrets are ciphertext authored by the OAuth authority, never by this repository.
#[derive(Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct McpOAuthClientRow {
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
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl std::fmt::Debug for McpOAuthClientRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpOAuthClientRow")
            .field("client_ref", &self.client_ref)
            .field("owner_id", &self.owner_id)
            .field("issuer", &self.issuer)
            .finish_non_exhaustive()
    }
}

/// The current grant for one configured MCP server. Token rotation changes
/// only `revision`; interactive authorization replaces `grant_id`.
#[derive(Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct McpOAuthGrantRow {
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
    pub revision: i64,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl std::fmt::Debug for McpOAuthGrantRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpOAuthGrantRow")
            .field("grant_id", &self.grant_id)
            .field("owner_id", &self.owner_id)
            .field("server_id", &self.server_id)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}
