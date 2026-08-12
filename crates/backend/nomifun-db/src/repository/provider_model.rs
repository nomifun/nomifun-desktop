use crate::error::DbError;
use crate::models::{NewProviderModel, ProviderModelRow};

#[async_trait::async_trait]
pub trait IProviderModelRepository: Send + Sync {
    async fn list(&self) -> Result<Vec<ProviderModelRow>, DbError>;
    async fn list_for_provider(&self, provider_id: &str) -> Result<Vec<ProviderModelRow>, DbError>;
    async fn get(
        &self,
        provider_id: &str,
        model: &str,
    ) -> Result<Option<ProviderModelRow>, DbError>;

    /// Upsert full model metadata and atomically replace its complete non-empty
    /// capability configuration. This is the only public configuration write.
    async fn save(
        &self,
        provider_id: &str,
        expected_config_revision: i64,
        model: &NewProviderModel<'_>,
    ) -> Result<ProviderModelRow, DbError>;

    async fn delete(&self, provider_id: &str, model: &str) -> Result<bool, DbError>;
}
