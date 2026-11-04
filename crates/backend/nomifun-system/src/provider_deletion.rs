//! Cross-subsystem model-reference cleanup, planned before catalog deletion.

use nomifun_common::{AppError, ProviderLifecycleBarrier, ProviderUsage};
use nomifun_db::ProviderModelCleanupPlan;
use std::sync::Arc;

#[async_trait::async_trait]
pub trait ProviderDeletionCoordinator: Send + Sync {
    /// Read-only diagnostics. A usage is never a deletion precondition.
    async fn usages(&self, provider_id: &str) -> Result<Vec<ProviderUsage>, AppError>;

    /// Validate and plan mutable SQLite references for a complete provider.
    async fn prepare_provider_cleanup(&self, _provider_id: &str) -> Result<ProviderModelCleanupPlan, AppError> {
        Ok(ProviderModelCleanupPlan::default())
    }

    /// Clear side-store selections while the lifecycle write guard is held.
    /// Immutable Session/Turn/history records are never cleanup targets.
    async fn cleanup_external_references(&self, _provider_id: &str, _model: Option<&str>) -> Result<(), AppError> {
        Ok(())
    }

    fn catalog_deleted(&self, _provider_id: &str, _model: Option<&str>) {}

    /// Build validated soft-reference replacements for one exact
    /// provider/model pair. The model repository applies this plan together
    /// with the catalog delete in one SQLite transaction.
    async fn prepare_soft_model_cleanup(
        &self,
        provider_id: &str,
        model: &str,
    ) -> Result<ProviderModelCleanupPlan, AppError>;

    /// Process-local barrier shared with side-store writers. The Provider
    /// service takes its write guard across cleanup planning and deletion.
    fn provider_lifecycle_barrier(&self) -> Option<Arc<ProviderLifecycleBarrier>> {
        None
    }
}

pub type SharedProviderDeletionCoordinator = Arc<dyn ProviderDeletionCoordinator>;
