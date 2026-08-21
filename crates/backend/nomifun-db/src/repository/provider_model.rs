use crate::error::DbError;
use crate::models::{CreativeStudioWorkflowRow, NewProviderModel, ProviderModelRow};

/// One compare-and-swap replacement for a canonical Creative Studio project
/// whose mutable document referenced the model being deleted.
#[derive(Debug, Clone)]
pub struct ProviderModelProjectCleanup {
    pub project_id: String,
    pub expected_revision: i64,
    pub document_json: String,
    pub node_count: i64,
    pub connection_count: i64,
    pub updated_at: i64,
}

/// One compare-and-swap replacement for a canonical Creative Studio workflow
/// whose mutable definition referenced the model being deleted.
#[derive(Debug, Clone)]
pub struct ProviderModelWorkflowCleanup {
    pub workflow_id: String,
    pub expected_revision: i64,
    pub replacement: CreativeStudioWorkflowRow,
}

/// All mutable Creative Studio references that must be removed together with
/// one exact provider/model row.
#[derive(Debug, Clone, Default)]
pub struct ProviderModelCleanupPlan {
    pub projects: Vec<ProviderModelProjectCleanup>,
    pub workflows: Vec<ProviderModelWorkflowCleanup>,
}

/// Strongly typed authority for deleting one model and its precomputed
/// Creative Studio cleanup patches in a single database transaction.
#[derive(Debug, Clone)]
pub struct CoordinatedProviderModelDelete {
    pub provider_id: String,
    pub model: String,
    pub expected_config_revision: i64,
    pub cleanup: ProviderModelCleanupPlan,
}

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

    async fn delete_coordinated(
        &self,
        plan: &CoordinatedProviderModelDelete,
    ) -> Result<bool, DbError> {
        let _ = plan;
        Err(DbError::Init(
            "coordinated provider model deletion is unavailable in this repository".into(),
        ))
    }
}
