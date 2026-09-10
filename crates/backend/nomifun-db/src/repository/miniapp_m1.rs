use std::collections::BTreeMap;
use std::path::Path;

use nomifun_agent_contracts::{
    MiniAppReadyOrigin, MiniAppReadyRelease, MiniAppReleaseArtifactV1,
    MiniAppServiceTestOutcome, MiniAppSourceLineage, MINIAPP_RELEASE_PROFILE_VERSION,
    canonical_json_bytes,
};

use crate::error::DbError;
pub use crate::models::MiniAppKvRow;
use crate::models::{
    MiniAppM1Kind, MiniAppM1LibrarySnapshot, MiniAppM1ProjectSourceState,
    MiniAppM1ReleaseOrigin, MiniAppM1ReleaseSourceKind, MiniAppM1Snapshot,
    MiniAppProjectRow, MiniAppReleaseArtifactRow, MiniAppReleaseRow,
    MiniAppSourceMutationIntentRow, MiniAppSurfaceSessionRow, ProductOperationRow,
    ProductOperationState,
};
use crate::repository::plugin_n1::{
    MAX_PRODUCT_OPERATION_LOG_LINE_CHARS, MAX_PRODUCT_OPERATION_LOG_LINES,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateMiniAppM1Params {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub project_id: String,
    pub expected_library_revision: i64,
    pub display_name: String,
    pub description: Option<String>,
    pub icon_asset_id: Option<String>,
    pub kind: MiniAppM1Kind,
    pub materialized_catalog_digest: String,
    pub config_schema_json: String,
    pub config_json: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppM1ManagedSourceLineage {
    pub managed_source_path: String,
    pub source_head_digest: String,
    pub dependency_lock_digest: String,
    pub build_profile_version: String,
    pub build_generation: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateMiniAppM1WithSourceParams {
    pub create: CreateMiniAppM1Params,
    pub source: MiniAppM1ManagedSourceLineage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MiniAppM1ImportSource {
    Managed(MiniAppM1ManagedSourceLineage),
    RuntimeOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeginMiniAppM1ImportAsNewParams {
    pub create: CreateMiniAppM1Params,
    pub operation_id: String,
    pub source: MiniAppM1ImportSource,
    pub bounded_log_tail: Vec<String>,
    pub started_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeginMiniAppM1ImportAsNewResult {
    pub snapshot: MiniAppM1Snapshot,
    pub operation: ProductOperationRow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishMiniAppM1ImportReadyParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub project_id: String,
    pub operation_id: String,
    pub expected_library_revision: i64,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_project_revision: i64,
    pub artifact: MiniAppReleaseArtifactRow,
    pub release: MiniAppReleaseRow,
    pub bounded_log_tail: Vec<String>,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailMiniAppM1ImportParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
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
pub struct CancelMiniAppM1ImportParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub project_id: String,
    pub operation_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_project_revision: i64,
    pub bounded_log_tail: Vec<String>,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartMiniAppM1ExportOperationParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub operation_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub bounded_log_tail: Vec<String>,
    pub started_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartMiniAppM1BackupExportParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub operation_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_config_revision: i64,
    pub expected_credential_bindings_revision: i64,
    pub started_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppM1BackupExportSnapshot {
    pub snapshot: MiniAppM1Snapshot,
    pub releases: Vec<MiniAppReleaseRow>,
    pub artifacts: Vec<MiniAppReleaseArtifactRow>,
    pub kv: Vec<MiniAppKvRow>,
    pub operation: ProductOperationRow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MiniAppM1BackupReleaseSlot {
    Ready,
    Active,
    Previous,
}

impl MiniAppM1BackupReleaseSlot {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Active => "active",
            Self::Previous => "previous",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppM1BackupImportRelease {
    pub slot: MiniAppM1BackupReleaseSlot,
    pub artifact: MiniAppReleaseArtifactRow,
    pub release: MiniAppReleaseRow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishMiniAppM1BackupImportParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub project_id: String,
    pub operation_id: String,
    pub expected_library_revision: i64,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_project_revision: i64,
    pub releases: Vec<MiniAppM1BackupImportRelease>,
    pub kv: Vec<MiniAppKvRow>,
    pub target_catalog_digest: String,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishMiniAppM1ExportOperationParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub operation_id: String,
    pub bounded_log_tail: Vec<String>,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailMiniAppM1ExportOperationParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub operation_id: String,
    pub progress_percent: u8,
    pub error_code: String,
    pub bounded_log_tail: Vec<String>,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CancelMiniAppM1ExportOperationParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub operation_id: String,
    pub bounded_log_tail: Vec<String>,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateMiniAppM1ProjectSourceParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub project_id: String,
    pub expected_project_revision: i64,
    pub source_state: MiniAppM1ProjectSourceState,
    pub managed_source_path: Option<String>,
    pub source_head_digest: Option<String>,
    pub dependency_lock_digest: Option<String>,
    pub build_profile_version: Option<String>,
    pub build_generation: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeginMiniAppSourceMutationParams {
    pub intent_id: String,
    pub owner_user_id: String,
    pub miniapp_id: String,
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
pub struct FinalizeMiniAppSourceMutationParams {
    pub intent_id: String,
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub project_id: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbortMiniAppSourceMutationParams {
    pub intent_id: String,
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub project_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartMiniAppM1BuildOperationParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub project_id: String,
    pub operation_id: String,
    pub expected_project_revision: i64,
    pub expected_source: MiniAppM1ManagedSourceLineage,
    pub bounded_log_tail: Vec<String>,
    pub started_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishMiniAppM1BuildOperationParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub operation_id: String,
    pub state: ProductOperationState,
    pub progress_percent: u8,
    pub last_error_code: Option<String>,
    pub bounded_log_tail: Vec<String>,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CancelMiniAppM1BuildOperationParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub operation_id: String,
    pub bounded_log_tail: Vec<String>,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinishMiniAppM1BuildAndRecordReadyParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub project_id: String,
    pub operation_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_project_revision: i64,
    pub expected_build_generation: i64,
    pub artifact: MiniAppReleaseArtifactRow,
    pub release: MiniAppReleaseRow,
    pub bounded_log_tail: Vec<String>,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordMiniAppM1ReadyReleaseParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub project_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_project_revision: i64,
    pub expected_build_generation: i64,
    pub artifact: MiniAppReleaseArtifactRow,
    pub release: MiniAppReleaseRow,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecordMiniAppM1ServiceTestReceiptParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_config_revision: i64,
    pub expected_credential_bindings_revision: i64,
    pub expected_ready_release_id: String,
    pub expected_ready_release_digest: String,
    pub receipt_id: String,
    pub service_run_key: String,
    pub outcome: MiniAppServiceTestOutcome,
    pub error_code: Option<String>,
    pub receipt_digest: String,
    pub runtime_fingerprint_digest: String,
    pub resolved_test_input_digest: String,
    pub receipt: serde_json::Value,
    pub issued_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct MiniAppServiceTestReceiptRow {
    pub id: i64,
    pub receipt_id: String,
    pub owner_user_id: String,
    pub miniapp_id: String,
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
pub struct MiniAppM1AutoPublishGuard {
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
pub struct PublishMiniAppM1ReadyParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_active_release_epoch: i64,
    pub expected_ready_release_id: String,
    pub expected_ready_release_digest: String,
    pub expected_active_release_digest: Option<String>,
    pub target_catalog_digest: String,
    pub auto_publish_guard: Option<MiniAppM1AutoPublishGuard>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollbackMiniAppM1PreviousParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
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
pub struct CommitMiniAppM1LifecycleParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_active_release_digest: Option<String>,
    pub enabled: bool,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashMiniAppM1Params {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_active_release_digest: Option<String>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoreMiniAppM1Params {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_lifecycle: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeginMiniAppM1DeleteParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_active_release_digest: Option<String>,
    pub operation_id: String,
    pub started_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailMiniAppM1DeleteParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub operation_id: String,
    pub expected_operation_revision: i64,
    pub error_code: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestartMiniAppM1DeleteParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub expected_failed_operation_id: String,
    pub new_operation_id: String,
    pub started_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalizeMiniAppM1DeleteParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub operation_id: String,
    pub expected_operation_revision: i64,
    pub finished_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetMiniAppM1AutoPublishParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_authorization_revision: Option<i64>,
    pub authorization_id: String,
    pub enabled: bool,
    pub user_authorized_at_ms: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenMiniAppM1SurfaceSessionParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
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
pub struct ResolveMiniAppM1SurfaceSessionParams {
    pub miniapp_id: String,
    pub capability_digest: String,
    pub expected_active_release_digest: String,
    pub expected_active_release_epoch: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseMiniAppM1SurfaceSessionParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub surface_session_id: String,
    pub capability_digest: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MiniAppM1SurfaceKvOperation {
    Get,
    Set { value: serde_json::Value },
    Delete,
    CompareAndSwap {
        expected_revision: Option<i64>,
        value: Option<serde_json::Value>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExecuteMiniAppM1SurfaceKvParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub surface_session_id: String,
    pub expected_surface_generation: i64,
    pub expected_capability_digest: String,
    pub expected_active_release_epoch: i64,
    pub expected_active_release_digest: String,
    pub namespace: String,
    pub key: String,
    pub operation: MiniAppM1SurfaceKvOperation,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MiniAppM1SurfaceKvResult {
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
pub trait IMiniAppM1Repository: Send + Sync {
    async fn library(
        &self,
        owner_user_id: &str,
    ) -> Result<MiniAppM1LibrarySnapshot, DbError>;

    async fn get(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
    ) -> Result<Option<MiniAppM1Snapshot>, DbError>;

    async fn create(
        &self,
        params: &CreateMiniAppM1Params,
    ) -> Result<MiniAppM1Snapshot, DbError>;

    async fn create_with_source(
        &self,
        params: &CreateMiniAppM1WithSourceParams,
    ) -> Result<MiniAppM1Snapshot, DbError>;

    async fn begin_import_as_new(
        &self,
        params: &BeginMiniAppM1ImportAsNewParams,
    ) -> Result<BeginMiniAppM1ImportAsNewResult, DbError>;

    async fn finish_import_ready(
        &self,
        params: &FinishMiniAppM1ImportReadyParams,
    ) -> Result<MiniAppM1Snapshot, DbError>;

    async fn fail_import(
        &self,
        params: &FailMiniAppM1ImportParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn cancel_import(
        &self,
        params: &CancelMiniAppM1ImportParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn start_export_operation(
        &self,
        params: &StartMiniAppM1ExportOperationParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn start_backup_export(
        &self,
        params: &StartMiniAppM1BackupExportParams,
    ) -> Result<MiniAppM1BackupExportSnapshot, DbError>;

    async fn finish_backup_import(
        &self,
        params: &FinishMiniAppM1BackupImportParams,
    ) -> Result<MiniAppM1Snapshot, DbError>;

    async fn finish_export_operation(
        &self,
        params: &FinishMiniAppM1ExportOperationParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn fail_export_operation(
        &self,
        params: &FailMiniAppM1ExportOperationParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn cancel_export_operation(
        &self,
        params: &CancelMiniAppM1ExportOperationParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn update_project_source_cas(
        &self,
        params: &UpdateMiniAppM1ProjectSourceParams,
    ) -> Result<MiniAppProjectRow, DbError>;

    async fn begin_source_mutation(
        &self,
        params: &BeginMiniAppSourceMutationParams,
    ) -> Result<MiniAppSourceMutationIntentRow, DbError>;

    async fn finalize_source_mutation(
        &self,
        params: &FinalizeMiniAppSourceMutationParams,
    ) -> Result<MiniAppM1Snapshot, DbError>;

    async fn abort_source_mutation(
        &self,
        params: &AbortMiniAppSourceMutationParams,
    ) -> Result<(), DbError>;

    async fn list_source_mutation_intents(
        &self,
    ) -> Result<Vec<MiniAppSourceMutationIntentRow>, DbError>;

    async fn get_source_mutation_intent(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
        project_id: &str,
    ) -> Result<Option<MiniAppSourceMutationIntentRow>, DbError>;

    async fn start_build_operation(
        &self,
        params: &StartMiniAppM1BuildOperationParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn finish_build_operation(
        &self,
        params: &FinishMiniAppM1BuildOperationParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn get_build_operation(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
        operation_id: &str,
    ) -> Result<Option<ProductOperationRow>, DbError>;

    async fn list_build_operations(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
    ) -> Result<Vec<ProductOperationRow>, DbError>;

    async fn cancel_build_operation(
        &self,
        params: &CancelMiniAppM1BuildOperationParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn finish_build_and_record_ready(
        &self,
        params: &FinishMiniAppM1BuildAndRecordReadyParams,
    ) -> Result<MiniAppM1Snapshot, DbError>;

    async fn record_ready_release(
        &self,
        params: &RecordMiniAppM1ReadyReleaseParams,
    ) -> Result<MiniAppM1Snapshot, DbError>;

    async fn record_service_test_receipt_cas(
        &self,
        params: &RecordMiniAppM1ServiceTestReceiptParams,
    ) -> Result<MiniAppM1Snapshot, DbError>;

    async fn get_ready_service_test_receipt(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
    ) -> Result<Option<MiniAppServiceTestReceiptRow>, DbError>;

    async fn publish_ready_cas(
        &self,
        params: &PublishMiniAppM1ReadyParams,
    ) -> Result<MiniAppM1Snapshot, DbError>;

    async fn rollback_previous_cas(
        &self,
        params: &RollbackMiniAppM1PreviousParams,
    ) -> Result<MiniAppM1Snapshot, DbError>;

    async fn commit_lifecycle_cas(
        &self,
        params: &CommitMiniAppM1LifecycleParams,
    ) -> Result<MiniAppM1Snapshot, DbError>;

    async fn trash_cas(
        &self,
        params: &TrashMiniAppM1Params,
    ) -> Result<MiniAppM1Snapshot, DbError>;

    async fn restore_cas(
        &self,
        params: &RestoreMiniAppM1Params,
    ) -> Result<MiniAppM1Snapshot, DbError>;

    async fn begin_delete(
        &self,
        params: &BeginMiniAppM1DeleteParams,
    ) -> Result<MiniAppM1Snapshot, DbError>;

    async fn fail_delete(
        &self,
        params: &FailMiniAppM1DeleteParams,
    ) -> Result<MiniAppM1Snapshot, DbError>;

    async fn restart_delete(
        &self,
        params: &RestartMiniAppM1DeleteParams,
    ) -> Result<MiniAppM1Snapshot, DbError>;

    async fn finalize_delete(
        &self,
        params: &FinalizeMiniAppM1DeleteParams,
    ) -> Result<i64, DbError>;

    async fn get_miniapp_operation(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
        operation_id: &str,
    ) -> Result<Option<ProductOperationRow>, DbError>;

    async fn list_miniapp_operations(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
    ) -> Result<Vec<ProductOperationRow>, DbError>;

    async fn set_auto_publish_cas(
        &self,
        params: &SetMiniAppM1AutoPublishParams,
    ) -> Result<MiniAppM1Snapshot, DbError>;

    async fn open_surface_session_cas(
        &self,
        params: &OpenMiniAppM1SurfaceSessionParams,
    ) -> Result<MiniAppSurfaceSessionRow, DbError>;

    async fn resolve_surface_session(
        &self,
        params: &ResolveMiniAppM1SurfaceSessionParams,
    ) -> Result<Option<MiniAppSurfaceSessionRow>, DbError>;

    async fn close_surface_session_cas(
        &self,
        params: &CloseMiniAppM1SurfaceSessionParams,
    ) -> Result<bool, DbError>;

    async fn revoke_all_surface_sessions_on_startup(&self) -> Result<u64, DbError>;

    #[allow(clippy::too_many_arguments)]
    async fn update_config_cas(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
        expected_product_revision: i64,
        expected_pointer_revision: i64,
        expected_config_revision: i64,
        expected_config_schema_json: &str,
        config_json: &str,
        updated_at: i64,
    ) -> Result<MiniAppM1Snapshot, DbError>;

    #[allow(clippy::too_many_arguments)]
    async fn replace_credential_bindings_cas(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
        expected_product_revision: i64,
        expected_pointer_revision: i64,
        expected_bindings_revision: i64,
        bindings: &BTreeMap<String, String>,
        updated_at: i64,
    ) -> Result<MiniAppM1Snapshot, DbError>;

    async fn get_kv(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
        namespace: &str,
        key: &str,
    ) -> Result<Option<MiniAppKvRow>, DbError>;

    async fn execute_surface_kv(
        &self,
        params: &ExecuteMiniAppM1SurfaceKvParams,
    ) -> Result<MiniAppM1SurfaceKvResult, DbError>;

    #[allow(clippy::too_many_arguments)]
    async fn put_kv_cas(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
        namespace: &str,
        key: &str,
        value: &serde_json::Value,
        expected_revision: Option<i64>,
        updated_at: i64,
    ) -> Result<MiniAppKvRow, DbError>;

    #[allow(clippy::too_many_arguments)]
    async fn delete_kv_cas(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
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

pub(crate) fn validate_kv_row(row: &MiniAppKvRow) -> Result<(), DbError> {
    if row.revision < 1
        || row.key_generation < 1
        || row.key_generation > row.revision
        || (row.is_tombstone && row.value_json != "null")
    {
        return Err(conflict(
            "MiniApp KV row violates the monotonic tombstone contract",
        ));
    }
    serde_json::from_str::<serde_json::Value>(&row.value_json)
        .map_err(|error| conflict(format!("MiniApp KV value is invalid JSON: {error}")))?;
    Ok(())
}

pub(crate) fn validate_managed_source_lineage(
    source: &MiniAppM1ManagedSourceLineage,
) -> Result<(), DbError> {
    validate_project_source(
        MiniAppM1ProjectSourceState::Editable,
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
    source_state: MiniAppM1ProjectSourceState,
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
        MiniAppM1ProjectSourceState::Empty
        | MiniAppM1ProjectSourceState::RuntimeOnly => {
            managed_source_path.is_none()
                && source_head_digest.is_none()
                && dependency_lock_digest.is_none()
                && build_profile_version.is_none()
                && build_generation == 0
        }
        MiniAppM1ProjectSourceState::Editable => {
            managed_source_path.is_some()
                && source_head_digest.is_some()
                && dependency_lock_digest.is_some()
                && build_profile_version.is_some()
                && build_generation > 0
        }
    };
    if !valid {
        return Err(conflict(
            "MiniApp Project source state and lineage fields are inconsistent",
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
    if build_profile_version.is_some_and(|value| value != MINIAPP_RELEASE_PROFILE_VERSION) {
        return Err(conflict(
            "MiniApp Project must use the canonical release profile version",
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

fn validate_artifact_row_shape(artifact: &MiniAppReleaseArtifactRow) -> Result<(), DbError> {
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
    artifact: &MiniAppReleaseArtifactRow,
) -> Result<(MiniAppReleaseArtifactRow, MiniAppReleaseArtifactV1), DbError> {
    let payload = validate_artifact(artifact)?;
    let mut normalized = artifact.clone();
    normalized.artifact_record_json = canonical_json_string(
        &payload,
        "artifact.artifact_record_json",
    )?;
    Ok((normalized, payload))
}

pub(crate) fn validate_artifact(
    artifact: &MiniAppReleaseArtifactRow,
) -> Result<MiniAppReleaseArtifactV1, DbError> {
    validate_artifact_row_shape(artifact)?;
    let payload: MiniAppReleaseArtifactV1 =
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
    artifact: &MiniAppReleaseArtifactV1,
) -> Result<(), DbError> {
    match product_kind {
        "ui_only" if artifact.manifest.payload.is_ui_only() => Ok(()),
        "service" => {
            if artifact.manifest.payload.service.is_none() {
                return Err(conflict(
                    "Service MiniApp Release must declare service/main.mjs",
                ));
            }
            Ok(())
        }
        "ui_only" => Err(conflict(
            "UI-only MiniApp Release cannot declare a Service",
        )),
        _ => Err(conflict("MiniApp product kind is invalid")),
    }
}

pub(crate) fn validate_release(
    release: &MiniAppReleaseRow,
    artifact: &MiniAppReleaseArtifactV1,
) -> Result<MiniAppReadyRelease, DbError> {
    validate_uuid(&release.release_id, "release.release_id")?;
    validate_uuid(&release.miniapp_id, "release.miniapp_id")?;
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
        "managed" => MiniAppM1ReleaseSourceKind::Managed,
        "runtime_only" => MiniAppM1ReleaseSourceKind::RuntimeOnly,
        _ => return Err(conflict("release.source_kind is invalid")),
    };
    let origin = match release.origin_kind.as_str() {
        "build" => MiniAppM1ReleaseOrigin::Build,
        "import" => MiniAppM1ReleaseOrigin::Import,
        _ => return Err(conflict("release.origin_kind is invalid")),
    };
    match source_kind {
        MiniAppM1ReleaseSourceKind::Managed => {
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
                != Some(MINIAPP_RELEASE_PROFILE_VERSION)
            {
                return Err(conflict(
                    "managed Release must use the canonical release profile version",
                ));
            }
        }
        MiniAppM1ReleaseSourceKind::RuntimeOnly => {
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
    if origin == MiniAppM1ReleaseOrigin::Build
        && source_kind != MiniAppM1ReleaseSourceKind::Managed
    {
        return Err(conflict("built Release must use managed source lineage"));
    }
    if release.created_at < 0 {
        return Err(conflict("release.created_at must be non-negative"));
    }
    let record: MiniAppReadyRelease =
        parse_canonical_json(&release.release_record_json, "release.release_record_json")?;
    record
        .validate_for_artifact(artifact)
        .map_err(|error| conflict(format!("Release record contract is invalid: {error}")))?;
    let expected_origin = match origin {
        MiniAppM1ReleaseOrigin::Build => MiniAppReadyOrigin::Build,
        MiniAppM1ReleaseOrigin::Import => MiniAppReadyOrigin::Import,
    };
    if record.miniapp_id.as_ref() != release.miniapp_id
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
            MiniAppSourceLineage::Managed {
                project_id,
                source_snapshot_digest,
                dependency_lock_digest,
                build_profile_version,
                build_generation,
            },
            MiniAppM1ReleaseSourceKind::Managed,
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
        (MiniAppSourceLineage::RuntimeOnly, MiniAppM1ReleaseSourceKind::RuntimeOnly) => {}
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
            conflict(format!("MiniApp M1 SQLite mutation was rejected: {}", database_error.message()))
        }
        _ => DbError::Query(error),
    }
}
