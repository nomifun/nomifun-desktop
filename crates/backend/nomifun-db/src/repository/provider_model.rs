use crate::error::DbError;
use crate::models::{CreativeStudioTemplateRow, NewProviderModel, ProviderModelRow};
use nomifun_agent_contracts::{AgentPresetRevision, PresetRevisionRef, ResolvedSnapshotEnvelope};

/// A newly compiled current default with only deleted catalog model choices
/// removed. The source revision and its frozen Snapshot are never rewritten.
#[derive(Debug, Clone)]
pub struct ProviderAgentRevisionCleanup {
    pub expected_current: PresetRevisionRef,
    pub revision: AgentPresetRevision,
    pub snapshot: ResolvedSnapshotEnvelope,
}

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

/// One compare-and-swap replacement for a canonical Creative Studio template
/// whose mutable definition referenced the model being deleted.
#[derive(Debug, Clone)]
pub struct ProviderModelTemplateCleanup {
    pub template_id: String,
    pub expected_revision: i64,
    pub replacement: CreativeStudioTemplateRow,
}

/// Prepared current-Agent revisions and Creative Studio CAS replacements.
/// Registered database selections are cleared in the same delete transaction.
#[derive(Debug, Clone, Default)]
pub struct ProviderModelCleanupPlan {
    pub projects: Vec<ProviderModelProjectCleanup>,
    pub templates: Vec<ProviderModelTemplateCleanup>,
    pub agent_revisions: Vec<ProviderAgentRevisionCleanup>,
}

#[derive(Debug, Clone)]
pub struct CoordinatedProviderDelete {
    pub provider_id: String,
    pub expected_config_revision: i64,
    pub cleanup: ProviderModelCleanupPlan,
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

    /// Update only the human-readable label. Display metadata is deliberately
    /// separate from the invocation graph and therefore does not bump the
    /// provider configuration revision.
    async fn set_display_name(
        &self,
        provider_id: &str,
        model: &str,
        display_name: Option<&str>,
    ) -> Result<(), DbError> {
        let _ = (provider_id, model, display_name);
        Ok(())
    }

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
