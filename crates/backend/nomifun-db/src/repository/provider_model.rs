use crate::error::DbError;
use crate::models::{NewProviderModel, ProviderModelRow, ProviderModelUpdate};

/// CRUD for the authoritative per-model entity rows, keyed by
/// `(provider_id, model)`.
#[async_trait::async_trait]
pub trait IProviderModelRepository: Send + Sync {
    /// All model rows across all providers.
    async fn list(&self) -> Result<Vec<ProviderModelRow>, DbError>;
    /// Model rows for one provider, ordered by `sort_order`.
    async fn list_for_provider(&self, provider_id: &str) -> Result<Vec<ProviderModelRow>, DbError>;
    /// A single model row, if present.
    async fn get(&self, provider_id: &str, model: &str) -> Result<Option<ProviderModelRow>, DbError>;
    /// Insert a new model row; `DbError::Conflict` when the composite key
    /// already exists (or the parent provider does not).
    async fn create(&self, provider_id: &str, row: &NewProviderModel<'_>) -> Result<ProviderModelRow, DbError>;
    /// Insert only when (provider_id, model) absent; returns whether inserted.
    ///
    /// This is the safe primitive for background catalog reconciliation: a
    /// concurrent user write must never be overwritten by a stale observation.
    async fn insert_if_absent(&self, provider_id: &str, row: &NewProviderModel<'_>) -> Result<bool, DbError>;
    /// Partially update one model row; fields left `None` are kept.
    async fn update(&self, provider_id: &str, model: &str, update: &ProviderModelUpdate<'_>) -> Result<ProviderModelRow, DbError>;
    /// Server-side health write (probe outcome). No-op returning false when the row is absent.
    async fn set_health(&self, provider_id: &str, model: &str, health_json: Option<&str>) -> Result<bool, DbError>;
    /// Delete one model row; returns whether a row was removed.
    async fn delete(&self, provider_id: &str, model: &str) -> Result<bool, DbError>;
}
