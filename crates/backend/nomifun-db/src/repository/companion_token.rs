use crate::error::DbError;
use nomifun_common::CompanionId;

/// Data access for `companion_access_token`. Each companion has at most one
/// token; only the SHA-256 hash is stored. Used by the Remote capability front
/// door (`/mcp`, `/mcp-agent`, `/v1`).
#[async_trait::async_trait]
pub trait ICompanionTokenRepository: Send + Sync {
    /// Every `(companion_id, token_hash)` pair, for boot-time validator hydration.
    async fn list_all(&self) -> Result<Vec<(CompanionId, String)>, DbError>;

    /// Insert or rotate the token hash for one companion (keyed on companion_id).
    async fn upsert_for_companion(&self, companion_id: &CompanionId, token_hash: &str) -> Result<(), DbError>;

    /// Revoke a companion's token. Idempotent (no error when absent).
    async fn delete_for_companion(&self, companion_id: &CompanionId) -> Result<(), DbError>;
}
