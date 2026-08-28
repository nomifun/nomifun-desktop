use crate::error::DbError;

/// Data access for the singleton `instance_access_token` row. Only the SHA-256
/// hash is persisted; the plaintext is returned once by the mint endpoint.
#[async_trait::async_trait]
pub trait IInstanceTokenRepository: Send + Sync {
    /// Load the configured installation token hash, if any.
    async fn get(&self) -> Result<Option<String>, DbError>;

    /// Insert or rotate the installation token hash.
    async fn set(&self, token_hash: &str) -> Result<(), DbError>;

    /// Revoke the installation token. Idempotent when no token exists.
    async fn clear(&self) -> Result<(), DbError>;
}
