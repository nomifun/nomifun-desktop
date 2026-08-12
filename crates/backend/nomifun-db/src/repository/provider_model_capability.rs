use crate::error::DbError;
use crate::models::ProviderModelCapabilityRow;

/// Task-scoped invocation configuration repository.
#[async_trait::async_trait]
pub trait IProviderModelCapabilityRepository: Send + Sync {
    async fn list(&self) -> Result<Vec<ProviderModelCapabilityRow>, DbError>;

    async fn list_for_provider(
        &self,
        provider_id: &str,
    ) -> Result<Vec<ProviderModelCapabilityRow>, DbError>;

    async fn list_for_model(
        &self,
        provider_id: &str,
        model: &str,
    ) -> Result<Vec<ProviderModelCapabilityRow>, DbError>;

    async fn get(
        &self,
        provider_id: &str,
        model: &str,
        task: &str,
    ) -> Result<Option<ProviderModelCapabilityRow>, DbError>;

    /// Persist or clear one task-specific probe observation only while the
    /// provider invocation graph still has `expected_config_revision`.
    /// Returns `false` for a stale revision or missing capability so late
    /// network results can be discarded without overwriting newer config.
    async fn set_health(
        &self,
        provider_id: &str,
        expected_config_revision: i64,
        model: &str,
        task: &str,
        health_json: Option<&str>,
    ) -> Result<bool, DbError>;
}
