use std::collections::BTreeMap;
use std::path::Path;

use nomifun_agent_contracts::{
    PluginReadyOrigin, PluginReadyRelease, PluginReleaseArtifactV1,
    PluginReleaseSourceLineage, PluginServiceTestOutcome,
    PLUGIN_RELEASE_PROFILE_VERSION, canonical_json_bytes,
};

use crate::error::DbError;
pub use crate::models::PluginRuntimeKvRow;
use crate::models::{
    PluginRuntimeKind, PluginRuntimeLibrarySnapshot, PluginRuntimeProjectSourceState,
    PluginRuntimeReleaseOrigin, PluginRuntimeReleaseSourceKind, PluginRuntimeSnapshot,
    PluginRuntimeProjectRow, PluginRuntimeReleaseArtifactRow, PluginRuntimeReleaseRow,
    PluginRuntimeSourceMutationIntentRow, PluginRuntimeSurfaceSessionRow, ProductOperationRow,
    ProductOperationState,
};
use crate::repository::plugin_n1::{
    MAX_PRODUCT_OPERATION_LOG_LINE_CHARS, MAX_PRODUCT_OPERATION_LOG_LINES,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreatePluginRuntimeParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub project_id: String,
    pub expected_library_revision: i64,
    pub display_name: String,
    pub description: Option<String>,
    pub icon_asset_id: Option<String>,
    pub kind: PluginRuntimeKind,
    pub materialized_catalog_digest: String,
    pub config_schema_json: String,
    pub config_json: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRuntimeManagedSourceLineage {
    pub managed_source_path: String,
    pub source_head_digest: String,
    pub dependency_lock_digest: String,
    pub build_profile_version: String,
    pub build_generation: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreatePluginRuntimeWithSourceParams {
    pub create: CreatePluginRuntimeParams,
    pub source: PluginRuntimeManagedSourceLineage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginRuntimeImportSource {
    Managed(PluginRuntimeManagedSourceLineage),
    RuntimeOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeginPluginRuntimeImportAsNewParams {
    pub create: CreatePluginRuntimeParams,
    pub operation_id: String,
    pub source: PluginRuntimeImportSource,
    pub bounded_log_tail: Vec<String>,
    pub started_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeginPluginRuntimeImportAsNewResult {
    pub snapshot: PluginRuntimeSnapshot,
    pub operation: ProductOperationRow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishPluginRuntimeImportReadyParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub project_id: String,
    pub operation_id: String,
    pub expected_library_revision: i64,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_project_revision: i64,
    pub artifact: PluginRuntimeReleaseArtifactRow,
    pub release: PluginRuntimeReleaseRow,
    pub bounded_log_tail: Vec<String>,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailPluginRuntimeImportParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub project_id: String,
    pub operation_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_project_revision: i64,
    pub progress_percent: u8,
    pub error_code: String,
    pub bounded_log_tail: Vec<String>,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CancelPluginRuntimeImportParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub project_id: String,
    pub operation_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_project_revision: i64,
    pub bounded_log_tail: Vec<String>,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartPluginRuntimeExportOperationParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub operation_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub bounded_log_tail: Vec<String>,
    pub started_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartPluginRuntimeBackupExportParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub operation_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_config_revision: i64,
    pub expected_credential_bindings_revision: i64,
    pub started_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRuntimeBackupExportSnapshot {
    pub snapshot: PluginRuntimeSnapshot,
    pub releases: Vec<PluginRuntimeReleaseRow>,
    pub artifacts: Vec<PluginRuntimeReleaseArtifactRow>,
    pub kv: Vec<PluginRuntimeKvRow>,
    pub operation: ProductOperationRow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PluginRuntimeBackupReleaseSlot {
    Ready,
    Active,
    Previous,
}

impl PluginRuntimeBackupReleaseSlot {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Active => "active",
            Self::Previous => "previous",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRuntimeBackupImportRelease {
    pub slot: PluginRuntimeBackupReleaseSlot,
    pub artifact: PluginRuntimeReleaseArtifactRow,
    pub release: PluginRuntimeReleaseRow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishPluginRuntimeBackupImportParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub project_id: String,
    pub operation_id: String,
    pub expected_library_revision: i64,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_project_revision: i64,
    pub releases: Vec<PluginRuntimeBackupImportRelease>,
    pub kv: Vec<PluginRuntimeKvRow>,
    pub target_catalog_digest: String,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishPluginRuntimeExportOperationParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub operation_id: String,
    pub bounded_log_tail: Vec<String>,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailPluginRuntimeExportOperationParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub operation_id: String,
    pub progress_percent: u8,
    pub error_code: String,
    pub bounded_log_tail: Vec<String>,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CancelPluginRuntimeExportOperationParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub operation_id: String,
    pub bounded_log_tail: Vec<String>,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdatePluginRuntimeProjectSourceParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub project_id: String,
    pub expected_project_revision: i64,
    pub source_state: PluginRuntimeProjectSourceState,
    pub managed_source_path: Option<String>,
    pub source_head_digest: Option<String>,
    pub dependency_lock_digest: Option<String>,
    pub build_profile_version: Option<String>,
    pub build_generation: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeginPluginSourceMutationParams {
    pub intent_id: String,
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub project_id: String,
    pub expected_product_revision: i64,
    pub expected_project_revision: i64,
    pub expected_build_generation: i64,
    pub expected_source_digest: String,
    pub next_source_digest: String,
    pub next_build_generation: i64,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalizePluginSourceMutationParams {
    pub intent_id: String,
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub project_id: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbortPluginSourceMutationParams {
    pub intent_id: String,
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub project_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartPluginRuntimeBuildOperationParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub project_id: String,
    pub operation_id: String,
    pub expected_project_revision: i64,
    pub expected_source: PluginRuntimeManagedSourceLineage,
    pub bounded_log_tail: Vec<String>,
    pub started_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishPluginRuntimeBuildOperationParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub operation_id: String,
    pub state: ProductOperationState,
    pub progress_percent: u8,
    pub last_error_code: Option<String>,
    pub bounded_log_tail: Vec<String>,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CancelPluginRuntimeBuildOperationParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub operation_id: String,
    pub bounded_log_tail: Vec<String>,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishPluginRuntimeBuildAndRecordReadyParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub project_id: String,
    pub operation_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_project_revision: i64,
    pub expected_build_generation: i64,
    pub artifact: PluginRuntimeReleaseArtifactRow,
    pub release: PluginRuntimeReleaseRow,
    pub bounded_log_tail: Vec<String>,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordPluginRuntimeReadyReleaseParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub project_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_project_revision: i64,
    pub expected_build_generation: i64,
    pub artifact: PluginRuntimeReleaseArtifactRow,
    pub release: PluginRuntimeReleaseRow,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecordPluginRuntimeServiceTestReceiptParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_config_revision: i64,
    pub expected_credential_bindings_revision: i64,
    pub expected_ready_release_id: String,
    pub expected_ready_release_digest: String,
    pub receipt_id: String,
    pub service_run_key: String,
    pub outcome: PluginServiceTestOutcome,
    pub error_code: Option<String>,
    pub receipt_digest: String,
    pub runtime_fingerprint_digest: String,
    pub resolved_test_input_digest: String,
    pub receipt: serde_json::Value,
    pub issued_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct PluginRuntimeServiceTestReceiptRow {
    pub id: i64,
    pub receipt_id: String,
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub release_id: String,
    pub release_digest: String,
    pub service_run_key: String,
    pub outcome: String,
    pub error_code: Option<String>,
    pub receipt_digest: String,
    pub runtime_fingerprint_digest: String,
    pub resolved_test_input_digest: String,
    pub tested_product_revision: i64,
    pub tested_pointer_revision: i64,
    pub tested_config_revision: i64,
    pub tested_credential_bindings_revision: i64,
    pub receipt_json: String,
    pub issued_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRuntimeAutoPublishGuard {
    pub authorization_id: String,
    pub authorization_revision: i64,
    pub project_id: String,
    pub project_revision: i64,
    pub source_head_digest: String,
    pub dependency_lock_digest: String,
    pub build_profile_version: String,
    pub build_generation: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishPluginRuntimeReadyParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_active_release_epoch: i64,
    pub expected_ready_release_id: String,
    pub expected_ready_release_digest: String,
    pub expected_active_release_digest: Option<String>,
    pub target_catalog_digest: String,
    pub auto_publish_guard: Option<PluginRuntimeAutoPublishGuard>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollbackPluginRuntimePreviousParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_active_release_epoch: i64,
    pub expected_current_release_id: String,
    pub expected_current_release_digest: String,
    pub expected_previous_release_id: String,
    pub expected_previous_release_digest: String,
    pub target_catalog_digest: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitPluginRuntimeLifecycleParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_active_release_digest: Option<String>,
    pub enabled: bool,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashPluginRuntimeParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_active_release_digest: Option<String>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestorePluginRuntimeParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_lifecycle: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeginPluginRuntimeDeleteParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_active_release_digest: Option<String>,
    pub operation_id: String,
    pub started_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailPluginRuntimeDeleteParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub operation_id: String,
    pub expected_operation_revision: i64,
    pub error_code: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestartPluginRuntimeDeleteParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub expected_failed_operation_id: String,
    pub new_operation_id: String,
    pub started_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalizePluginRuntimeDeleteParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub operation_id: String,
    pub expected_operation_revision: i64,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetPluginRuntimeAutoPublishParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_authorization_revision: Option<i64>,
    pub authorization_id: String,
    pub enabled: bool,
    pub user_authorized_at_ms: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenPluginRuntimeSurfaceSessionParams {
    pub conversation_id: Option<String>,
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub surface_session_id: String,
    pub capability_digest: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_active_release_id: String,
    pub expected_active_release_digest: String,
    pub expected_active_release_epoch: i64,
    pub issued_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvePluginRuntimeSurfaceSessionParams {
    pub plugin_product_id: String,
    pub capability_digest: String,
    pub expected_active_release_digest: String,
    pub expected_active_release_epoch: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosePluginRuntimeSurfaceSessionParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub surface_session_id: String,
    pub capability_digest: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PluginRuntimeSurfaceKvOperation {
    Get,
    Set { value: serde_json::Value },
    Delete,
    CompareAndSwap {
        expected_revision: Option<i64>,
        value: Option<serde_json::Value>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExecutePluginRuntimeSurfaceKvParams {
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub surface_session_id: String,
    pub expected_surface_generation: i64,
    pub expected_capability_digest: String,
    pub expected_active_release_epoch: i64,
    pub expected_active_release_digest: String,
    pub namespace: String,
    pub key: String,
    pub operation: PluginRuntimeSurfaceKvOperation,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PluginRuntimeSurfaceKvResult {
    Value {
        value: Option<serde_json::Value>,
        revision: Option<i64>,
    },
    Written {
        revision: i64,
    },
    Deleted {
        existed: bool,
    },
    CompareAndSwap {
        applied: bool,
        current_revision: Option<i64>,
    },
}

#[async_trait::async_trait]
pub trait IPluginRuntimeRepository: Send + Sync {
    async fn library(
        &self,
        owner_user_id: &str,
    ) -> Result<PluginRuntimeLibrarySnapshot, DbError>;

    async fn get(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
    ) -> Result<Option<PluginRuntimeSnapshot>, DbError>;

    async fn create(
        &self,
        params: &CreatePluginRuntimeParams,
    ) -> Result<PluginRuntimeSnapshot, DbError>;

    async fn create_with_source(
        &self,
        params: &CreatePluginRuntimeWithSourceParams,
    ) -> Result<PluginRuntimeSnapshot, DbError>;

    async fn begin_import_as_new(
        &self,
        params: &BeginPluginRuntimeImportAsNewParams,
    ) -> Result<BeginPluginRuntimeImportAsNewResult, DbError>;

    async fn finish_import_ready(
        &self,
        params: &FinishPluginRuntimeImportReadyParams,
    ) -> Result<PluginRuntimeSnapshot, DbError>;

    async fn fail_import(
        &self,
        params: &FailPluginRuntimeImportParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn cancel_import(
        &self,
        params: &CancelPluginRuntimeImportParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn start_export_operation(
        &self,
        params: &StartPluginRuntimeExportOperationParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn start_backup_export(
        &self,
        params: &StartPluginRuntimeBackupExportParams,
    ) -> Result<PluginRuntimeBackupExportSnapshot, DbError>;

    async fn finish_backup_import(
        &self,
        params: &FinishPluginRuntimeBackupImportParams,
    ) -> Result<PluginRuntimeSnapshot, DbError>;

    async fn finish_export_operation(
        &self,
        params: &FinishPluginRuntimeExportOperationParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn fail_export_operation(
        &self,
        params: &FailPluginRuntimeExportOperationParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn cancel_export_operation(
        &self,
        params: &CancelPluginRuntimeExportOperationParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn update_project_source_cas(
        &self,
        params: &UpdatePluginRuntimeProjectSourceParams,
    ) -> Result<PluginRuntimeProjectRow, DbError>;

    async fn begin_source_mutation(
        &self,
        params: &BeginPluginSourceMutationParams,
    ) -> Result<PluginRuntimeSourceMutationIntentRow, DbError>;

    async fn finalize_source_mutation(
        &self,
        params: &FinalizePluginSourceMutationParams,
    ) -> Result<PluginRuntimeSnapshot, DbError>;

    async fn abort_source_mutation(
        &self,
        params: &AbortPluginSourceMutationParams,
    ) -> Result<(), DbError>;

    async fn list_source_mutation_intents(
        &self,
    ) -> Result<Vec<PluginRuntimeSourceMutationIntentRow>, DbError>;

    async fn get_source_mutation_intent(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
        project_id: &str,
    ) -> Result<Option<PluginRuntimeSourceMutationIntentRow>, DbError>;

    async fn start_build_operation(
        &self,
        params: &StartPluginRuntimeBuildOperationParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn finish_build_operation(
        &self,
        params: &FinishPluginRuntimeBuildOperationParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn get_build_operation(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
        operation_id: &str,
    ) -> Result<Option<ProductOperationRow>, DbError>;

    async fn list_build_operations(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
    ) -> Result<Vec<ProductOperationRow>, DbError>;

    async fn cancel_build_operation(
        &self,
        params: &CancelPluginRuntimeBuildOperationParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn finish_build_and_record_ready(
        &self,
        params: &FinishPluginRuntimeBuildAndRecordReadyParams,
    ) -> Result<PluginRuntimeSnapshot, DbError>;

    async fn record_ready_release(
        &self,
        params: &RecordPluginRuntimeReadyReleaseParams,
    ) -> Result<PluginRuntimeSnapshot, DbError>;

    async fn record_service_test_receipt_cas(
        &self,
        params: &RecordPluginRuntimeServiceTestReceiptParams,
    ) -> Result<PluginRuntimeSnapshot, DbError>;

    async fn get_ready_service_test_receipt(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
    ) -> Result<Option<PluginRuntimeServiceTestReceiptRow>, DbError>;

    async fn publish_ready_cas(
        &self,
        params: &PublishPluginRuntimeReadyParams,
    ) -> Result<PluginRuntimeSnapshot, DbError>;

    async fn rollback_previous_cas(
        &self,
        params: &RollbackPluginRuntimePreviousParams,
    ) -> Result<PluginRuntimeSnapshot, DbError>;

    async fn commit_lifecycle_cas(
        &self,
        params: &CommitPluginRuntimeLifecycleParams,
    ) -> Result<PluginRuntimeSnapshot, DbError>;

    async fn trash_cas(
        &self,
        params: &TrashPluginRuntimeParams,
    ) -> Result<PluginRuntimeSnapshot, DbError>;

    async fn restore_cas(
        &self,
        params: &RestorePluginRuntimeParams,
    ) -> Result<PluginRuntimeSnapshot, DbError>;

    async fn begin_delete(
        &self,
        params: &BeginPluginRuntimeDeleteParams,
    ) -> Result<PluginRuntimeSnapshot, DbError>;

    async fn fail_delete(
        &self,
        params: &FailPluginRuntimeDeleteParams,
    ) -> Result<PluginRuntimeSnapshot, DbError>;

    async fn restart_delete(
        &self,
        params: &RestartPluginRuntimeDeleteParams,
    ) -> Result<PluginRuntimeSnapshot, DbError>;

    async fn finalize_delete(
        &self,
        params: &FinalizePluginRuntimeDeleteParams,
    ) -> Result<i64, DbError>;

    async fn get_plugin_operation(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
        operation_id: &str,
    ) -> Result<Option<ProductOperationRow>, DbError>;

    async fn list_plugin_operations(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
    ) -> Result<Vec<ProductOperationRow>, DbError>;

    async fn set_auto_publish_cas(
        &self,
        params: &SetPluginRuntimeAutoPublishParams,
    ) -> Result<PluginRuntimeSnapshot, DbError>;

    async fn open_surface_session_cas(
        &self,
        params: &OpenPluginRuntimeSurfaceSessionParams,
    ) -> Result<PluginRuntimeSurfaceSessionRow, DbError>;

    async fn resolve_surface_session(
        &self,
        params: &ResolvePluginRuntimeSurfaceSessionParams,
    ) -> Result<Option<PluginRuntimeSurfaceSessionRow>, DbError>;


    async fn close_surface_session_cas(
        &self,
        params: &ClosePluginRuntimeSurfaceSessionParams,
    ) -> Result<bool, DbError>;

    /// Revoke every Plugin Surface authority scoped to one canonical
    /// AgentSession before that Session's tombstone is finalized.
    async fn revoke_agent_session_surfaces(
        &self,
        owner_user_id: &str,
        agent_session_id: &str,
    ) -> Result<u64, DbError>;

    async fn revoke_all_surface_sessions_on_startup(&self) -> Result<u64, DbError>;

    #[allow(clippy::too_many_arguments)]
    async fn update_config_cas(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
        expected_product_revision: i64,
        expected_pointer_revision: i64,
        expected_config_revision: i64,
        expected_config_schema_json: &str,
        config_json: &str,
        updated_at: i64,
    ) -> Result<PluginRuntimeSnapshot, DbError>;

    #[allow(clippy::too_many_arguments)]
    async fn replace_credential_bindings_cas(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
        expected_product_revision: i64,
        expected_pointer_revision: i64,
        expected_bindings_revision: i64,
        bindings: &BTreeMap<String, String>,
        updated_at: i64,
    ) -> Result<PluginRuntimeSnapshot, DbError>;

    async fn get_kv(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
        namespace: &str,
        key: &str,
    ) -> Result<Option<PluginRuntimeKvRow>, DbError>;

    async fn execute_surface_kv(
        &self,
        params: &ExecutePluginRuntimeSurfaceKvParams,
    ) -> Result<PluginRuntimeSurfaceKvResult, DbError>;

    #[allow(clippy::too_many_arguments)]
    async fn put_kv_cas(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
        namespace: &str,
        key: &str,
        value: &serde_json::Value,
        expected_revision: Option<i64>,
        updated_at: i64,
    ) -> Result<PluginRuntimeKvRow, DbError>;

    #[allow(clippy::too_many_arguments)]
    async fn delete_kv_cas(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
        namespace: &str,
        key: &str,
        expected_revision: i64,
        updated_at: i64,
    ) -> Result<bool, DbError>;
}

pub(crate) fn conflict(message: impl Into<String>) -> DbError {
    DbError::Conflict(message.into())
}

pub(crate) fn validate_uuid(value: &str, label: &str) -> Result<(), DbError> {
    nomifun_common::validate_uuidv7(value)
        .map(|_| ())
        .map_err(|error| conflict(format!("{label} must be canonical UUIDv7: {error}")))
}

pub(crate) fn validate_digest(value: &str, label: &str) -> Result<(), DbError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(conflict(format!("{label} must be a lowercase SHA-256 digest")))
    }
}

pub(crate) fn validate_optional_digest(value: Option<&str>, label: &str) -> Result<(), DbError> {
    if let Some(value) = value {
        validate_digest(value, label)?;
    }
    Ok(())
}

pub(crate) fn validate_json_object(value: &str, label: &str) -> Result<(), DbError> {
    let parsed: serde_json::Value = serde_json::from_str(value)
        .map_err(|error| conflict(format!("{label} is invalid JSON: {error}")))?;
    if !parsed.is_object() {
        return Err(conflict(format!("{label} must be a JSON object")));
    }
    Ok(())
}

pub(crate) fn validate_visible_ascii_key(
    value: &str,
    label: &str,
    maximum_bytes: usize,
) -> Result<(), DbError> {
    if value.is_empty()
        || value.len() > maximum_bytes
        || !value.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(conflict(format!(
            "{label} must contain 1 to {maximum_bytes} visible ASCII bytes"
        )));
    }
    Ok(())
}

pub(crate) fn validate_kv_row(row: &PluginRuntimeKvRow) -> Result<(), DbError> {
    if row.revision < 1
        || row.key_generation < 1
        || row.key_generation > row.revision
        || (row.is_tombstone && row.value_json != "null")
    {
        return Err(conflict(
            "Plugin KV row violates the monotonic tombstone contract",
        ));
    }
    serde_json::from_str::<serde_json::Value>(&row.value_json)
        .map_err(|error| conflict(format!("Plugin KV value is invalid JSON: {error}")))?;
    Ok(())
}

pub(crate) fn validate_managed_source_lineage(
    source: &PluginRuntimeManagedSourceLineage,
) -> Result<(), DbError> {
    validate_project_source(
        PluginRuntimeProjectSourceState::Editable,
        Some(&source.managed_source_path),
        Some(&source.source_head_digest),
        Some(&source.dependency_lock_digest),
        Some(&source.build_profile_version),
        source.build_generation,
    )
}

pub(crate) fn serialize_product_operation_log_tail(
    lines: &[String],
) -> Result<String, DbError> {
    if lines.len() > MAX_PRODUCT_OPERATION_LOG_LINES {
        return Err(conflict(format!(
            "product operation log tail exceeds {MAX_PRODUCT_OPERATION_LOG_LINES} lines"
        )));
    }
    if lines.iter().any(|line| {
        line.chars().count() > MAX_PRODUCT_OPERATION_LOG_LINE_CHARS || line.contains('\0')
    }) {
        return Err(conflict(format!(
            "product operation log line exceeds {MAX_PRODUCT_OPERATION_LOG_LINE_CHARS} characters or contains NUL"
        )));
    }
    serde_json::to_string(lines)
        .map_err(|error| conflict(format!("product operation log cannot be serialized: {error}")))
}

pub(crate) fn validate_product_operation_error_code(
    value: Option<&str>,
) -> Result<(), DbError> {
    if value.is_some_and(|code| {
        code.is_empty()
            || code.chars().count() > 256
            || !code.bytes().all(|byte| byte.is_ascii_graphic())
    }) {
        return Err(conflict(
            "product operation last_error_code must contain 1 to 256 visible ASCII characters",
        ));
    }
    Ok(())
}

pub(crate) fn validate_relative_path(value: Option<&str>, label: &str) -> Result<(), DbError> {
    let Some(value) = value else {
        return Ok(());
    };
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || value.contains('\\')
        || value.contains('\0')
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(conflict(format!(
            "{label} must be a non-empty managed relative path"
        )));
    }
    Ok(())
}

pub(crate) fn validate_project_source(
    source_state: PluginRuntimeProjectSourceState,
    managed_source_path: Option<&str>,
    source_head_digest: Option<&str>,
    dependency_lock_digest: Option<&str>,
    build_profile_version: Option<&str>,
    build_generation: i64,
) -> Result<(), DbError> {
    validate_relative_path(managed_source_path, "managed_source_path")?;
    validate_optional_digest(source_head_digest, "source_head_digest")?;
    validate_optional_digest(dependency_lock_digest, "dependency_lock_digest")?;
    if build_generation < 0 {
        return Err(conflict("build_generation must be non-negative"));
    }
    let valid = match source_state {
        PluginRuntimeProjectSourceState::Empty
        | PluginRuntimeProjectSourceState::RuntimeOnly => {
            managed_source_path.is_none()
                && source_head_digest.is_none()
                && dependency_lock_digest.is_none()
                && build_profile_version.is_none()
                && build_generation == 0
        }
        PluginRuntimeProjectSourceState::Editable => {
            managed_source_path.is_some()
                && source_head_digest.is_some()
                && dependency_lock_digest.is_some()
                && build_profile_version.is_some()
                && build_generation > 0
        }
    };
    if !valid {
        return Err(conflict(
            "Plugin Project source state and lineage fields are inconsistent",
        ));
    }
    if build_profile_version.is_some_and(|value| {
        value.is_empty()
            || value.len() > 64
            || !value.bytes().all(|byte| byte.is_ascii_graphic())
    }) {
        return Err(conflict(
            "build_profile_version must be visible ASCII with at most 64 bytes",
        ));
    }
    if build_profile_version.is_some_and(|value| value != PLUGIN_RELEASE_PROFILE_VERSION) {
        return Err(conflict(
            "Plugin Project must use the canonical release profile version",
        ));
    }
    Ok(())
}

fn canonical_json_string<T: serde::Serialize>(
    value: &T,
    label: &str,
) -> Result<String, DbError> {
    let bytes = canonical_json_bytes(value)
        .map_err(|error| conflict(format!("{label} cannot be canonicalized: {error}")))?;
    String::from_utf8(bytes)
        .map_err(|error| conflict(format!("{label} canonical JSON is not UTF-8: {error}")))
}

fn parse_canonical_json<T>(value: &str, label: &str) -> Result<T, DbError>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let parsed: T = serde_json::from_str(value)
        .map_err(|error| conflict(format!("{label} is invalid: {error}")))?;
    if canonical_json_string(&parsed, label)? != value {
        return Err(conflict(format!("{label} must use canonical JSON")));
    }
    Ok(parsed)
}

fn validate_artifact_row_shape(artifact: &PluginRuntimeReleaseArtifactRow) -> Result<(), DbError> {
    validate_uuid(&artifact.artifact_id, "artifact.artifact_id")?;
    validate_uuid(&artifact.owner_user_id, "artifact.owner_user_id")?;
    validate_digest(&artifact.artifact_digest, "artifact.artifact_digest")?;
    validate_digest(&artifact.manifest_digest, "artifact.manifest_digest")?;
    validate_relative_path(Some(&artifact.managed_path), "artifact.managed_path")?;
    if artifact.created_at < 0 {
        return Err(conflict("artifact.created_at must be non-negative"));
    }
    Ok(())
}

pub(crate) fn normalize_incoming_artifact(
    artifact: &PluginRuntimeReleaseArtifactRow,
) -> Result<(PluginRuntimeReleaseArtifactRow, PluginReleaseArtifactV1), DbError> {
    let payload = validate_artifact(artifact)?;
    let mut normalized = artifact.clone();
    normalized.artifact_record_json = canonical_json_string(
        &payload,
        "artifact.artifact_record_json",
    )?;
    Ok((normalized, payload))
}

pub(crate) fn validate_artifact(
    artifact: &PluginRuntimeReleaseArtifactRow,
) -> Result<PluginReleaseArtifactV1, DbError> {
    validate_artifact_row_shape(artifact)?;
    let payload: PluginReleaseArtifactV1 =
        parse_canonical_json(&artifact.artifact_record_json, "artifact.artifact_record_json")?;
    payload
        .validate()
        .map_err(|error| conflict(format!("artifact contract is invalid: {error}")))?;
    if payload.artifact_id.as_ref() != artifact.artifact_id
        || payload.artifact_digest.as_ref() != artifact.artifact_digest
        || payload.manifest.payload_digest.as_ref() != artifact.manifest_digest
    {
        return Err(conflict(
            "artifact row does not match its typed Artifact payload",
        ));
    }
    Ok(payload)
}

pub(crate) fn validate_product_artifact_contract(
    product_kind: &str,
    artifact: &PluginReleaseArtifactV1,
) -> Result<(), DbError> {
    if product_kind != "plugin" {
        return Err(conflict("Plugin product kind is invalid"));
    }
    artifact.validate().map_err(|error| conflict(format!("Plugin Release contract is invalid: {error}")))
}

pub(crate) fn validate_release(
    release: &PluginRuntimeReleaseRow,
    artifact: &PluginReleaseArtifactV1,
) -> Result<PluginReadyRelease, DbError> {
    validate_uuid(&release.release_id, "release.release_id")?;
    validate_uuid(&release.plugin_product_id, "release.plugin_product_id")?;
    validate_uuid(&release.owner_user_id, "release.owner_user_id")?;
    validate_uuid(&release.artifact_id, "release.artifact_id")?;
    validate_uuid(&release.origin_operation_id, "release.origin_operation_id")?;
    validate_digest(&release.artifact_digest, "release.artifact_digest")?;
    validate_digest(&release.manifest_digest, "release.manifest_digest")?;
    validate_digest(&release.release_digest, "release.release_digest")?;
    validate_optional_digest(
        release.source_snapshot_digest.as_deref(),
        "release.source_snapshot_digest",
    )?;
    validate_optional_digest(
        release.dependency_lock_digest.as_deref(),
        "release.dependency_lock_digest",
    )?;
    if release.project_id.is_some() {
        validate_uuid(
            release.project_id.as_deref().unwrap_or_default(),
            "release.project_id",
        )?;
    }
    let source_kind = match release.source_kind.as_str() {
        "managed" => PluginRuntimeReleaseSourceKind::Managed,
        "runtime_only" => PluginRuntimeReleaseSourceKind::RuntimeOnly,
        _ => return Err(conflict("release.source_kind is invalid")),
    };
    let origin = match release.origin_kind.as_str() {
        "build" => PluginRuntimeReleaseOrigin::Build,
        "import" => PluginRuntimeReleaseOrigin::Import,
        _ => return Err(conflict("release.origin_kind is invalid")),
    };
    match source_kind {
        PluginRuntimeReleaseSourceKind::Managed => {
            if release.project_id.is_none()
                || release.source_snapshot_digest.is_none()
                || release.dependency_lock_digest.is_none()
                || release.build_profile_version.is_none()
                || release.build_generation.is_none()
                || release.build_generation == Some(0)
            {
                return Err(conflict(
                    "managed Release requires complete source lineage",
                ));
            }
            if release.build_profile_version.as_deref()
                != Some(PLUGIN_RELEASE_PROFILE_VERSION)
            {
                return Err(conflict(
                    "managed Release must use the canonical release profile version",
                ));
            }
        }
        PluginRuntimeReleaseSourceKind::RuntimeOnly => {
            if release.project_id.is_some()
                || release.source_snapshot_digest.is_some()
                || release.dependency_lock_digest.is_some()
                || release.build_profile_version.is_some()
                || release.build_generation.is_some()
            {
                return Err(conflict(
                    "runtime-only Release cannot carry authored source lineage",
                ));
            }
        }
    }
    if origin == PluginRuntimeReleaseOrigin::Build
        && source_kind != PluginRuntimeReleaseSourceKind::Managed
    {
        return Err(conflict("built Release must use managed source lineage"));
    }
    if release.created_at < 0 {
        return Err(conflict("release.created_at must be non-negative"));
    }
    let record: PluginReadyRelease =
        parse_canonical_json(&release.release_record_json, "release.release_record_json")?;
    record
        .validate_for_artifact(artifact)
        .map_err(|error| conflict(format!("Release record contract is invalid: {error}")))?;
    let expected_origin = match origin {
        PluginRuntimeReleaseOrigin::Build => PluginReadyOrigin::Build,
        PluginRuntimeReleaseOrigin::Import => PluginReadyOrigin::Import,
    };
    if record.plugin_product_id.as_ref() != release.plugin_product_id
        || record.release.release_id.as_ref() != release.release_id
        || record.release.artifact_id.as_ref() != release.artifact_id
        || record.release.release_digest.as_ref() != release.release_digest
        || record.release.manifest_digest.as_ref() != release.manifest_digest
        || record.origin_operation_id.as_ref() != release.origin_operation_id
        || record.origin != expected_origin
        || record.created_at_ms != release.created_at
    {
        return Err(conflict(
            "Release row identity does not match its typed Release record",
        ));
    }
    match (&record.source_lineage, source_kind) {
        (
            PluginReleaseSourceLineage::Managed {
                project_id,
                source_snapshot_digest,
                dependency_lock_digest,
                build_profile_version,
                build_generation,
            },
            PluginRuntimeReleaseSourceKind::Managed,
        ) if project_id.as_ref() == release.project_id.as_deref().unwrap_or_default()
            && source_snapshot_digest.as_ref()
                == release
                    .source_snapshot_digest
                    .as_deref()
                    .unwrap_or_default()
            && dependency_lock_digest.as_ref()
                == release
                    .dependency_lock_digest
                    .as_deref()
                    .unwrap_or_default()
            && build_profile_version.as_ref()
                == release
                    .build_profile_version
                    .as_deref()
                    .unwrap_or_default()
            && i64::try_from(*build_generation).ok() == release.build_generation => {}
        (PluginReleaseSourceLineage::RuntimeOnly, PluginRuntimeReleaseSourceKind::RuntimeOnly) => {}
        _ => {
            return Err(conflict(
                "Release row lineage does not match its typed Release record",
            ));
        }
    }
    Ok(record)
}

pub(crate) fn query_error(error: sqlx::Error) -> DbError {
    match &error {
        sqlx::Error::Database(database_error) => {
            conflict(format!("Plugin M1 SQLite mutation was rejected: {}", database_error.message()))
        }
        _ => DbError::Query(error),
    }
}
