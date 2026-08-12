use crate::error::DbError;
use crate::models::{ProviderConnectionRow, UpsertProviderConnectionParams};

/// CRUD for non-default per-role provider connection profiles, keyed by
/// `(provider_id, role)`.
#[async_trait::async_trait]
pub trait IProviderConnectionRepository: Send + Sync {
    /// Connection rows for one provider, ordered by `role`.
    async fn list_for_provider(
        &self,
        provider_id: &str,
    ) -> Result<Vec<ProviderConnectionRow>, DbError>;
    /// A single connection row, if present.
    async fn get(
        &self,
        provider_id: &str,
        role: &str,
    ) -> Result<Option<ProviderConnectionRow>, DbError>;
    /// Insert or update the connection for `(provider_id, role)`. The stable
    /// `connection_id` is minted on first insert and never changes on update.
    async fn upsert(
        &self,
        provider_id: &str,
        expected_config_revision: i64,
        params: &UpsertProviderConnectionParams<'_>,
    ) -> Result<ProviderConnectionRow, DbError>;
    /// Delete one connection row; returns whether a row was removed.
    /// Implementations must reject deletion while a task capability still
    /// references the role.
    async fn delete(&self, provider_id: &str, role: &str) -> Result<bool, DbError>;
}
