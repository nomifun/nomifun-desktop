use serde_json::Value;

use crate::DbError;
use crate::models::{
    PluginArtifactRow, PluginCandidateOrigin, PluginCandidateTestReceiptRow,
    PluginCredentialBindingInput, PluginCredentialBindingSnapshot, PluginKvRow, PluginMountRow,
    PluginMountRuntimeState, PluginProjectRow, PluginReadyCandidateRow, ProductOperationKind,
    ProductOperationRow, ProductOperationState,
};

pub const MAX_PRODUCT_OPERATION_LOG_LINES: usize = 200;
pub const MAX_PRODUCT_OPERATION_LOG_LINE_CHARS: usize = 4096;

#[derive(Debug, Clone)]
pub struct CreatePluginArtifactParams {
    pub artifact_id: String,
    pub artifact_digest: String,
    pub package_id: String,
    pub package_version: String,
    pub manifest_digest: String,
    pub manifest: Value,
    pub managed_path: String,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct CreatePluginProjectParams {
    pub project_id: String,
    pub owner_user_id: String,
    pub package_id: String,
    pub display_name: String,
    pub description: String,
    pub managed_source_path: Option<String>,
    pub source_head_digest: Option<String>,
    pub dependency_lock_digest: Option<String>,
    pub initial_build_generation: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct UpdatePluginProjectSourceParams {
    pub project_id: String,
    pub expected_generation: i64,
    pub source_head_digest: String,
    pub dependency_lock_digest: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct DeletePluginProjectParams {
    pub project_id: String,
    pub owner_user_id: String,
    pub expected_updated_at: i64,
    pub expected_generation: i64,
    pub expected_ready_candidate_id: Option<String>,
    pub expected_ready_candidate_digest: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DiscardPluginCandidateParams {
    pub project_id: String,
    pub owner_user_id: String,
    pub expected_updated_at: i64,
    pub expected_generation: i64,
    pub candidate_id: String,
    pub expected_candidate_digest: String,
}

#[derive(Debug, Clone)]
pub struct StartProductOperationParams {
    pub operation_id: String,
    pub kind: ProductOperationKind,
    pub owner_kind: String,
    pub owner_id: String,
    pub progress_percent: Option<u8>,
    pub bounded_log_tail: Vec<String>,
    pub started_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct FinishProductOperationParams {
    pub operation_id: String,
    pub state: ProductOperationState,
    pub progress_percent: Option<u8>,
    pub last_error_code: Option<String>,
    pub bounded_log_tail: Vec<String>,
    pub finished_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct RecordPluginReadyCandidateParams {
    pub candidate_id: String,
    pub project_id: String,
    pub candidate_digest: String,
    pub origin: PluginCandidateOrigin,
    pub artifact_id: String,
    pub artifact_digest: String,
    pub base_target_digest: Option<String>,
    pub source_snapshot_digest: Option<String>,
    pub dependency_lock_digest: Option<String>,
    pub contract_diff: Value,
    pub origin_operation_id: String,
    pub expected_generation: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct RecordPluginCandidateTestReceiptParams {
    pub receipt_id: String,
    pub candidate_id: String,
    pub candidate_digest: String,
    pub artifact_id: String,
    pub artifact_digest: String,
    pub receipt_digest: String,
    pub runtime_fingerprint_digest: String,
    pub receipt: Value,
    pub tested_at: i64,
}

#[derive(Debug, Clone)]
pub struct ApplyPluginCandidateParams {
    pub project_id: String,
    pub candidate_id: String,
    pub expected_project_generation: i64,
    pub expected_mount_revision: Option<i64>,
    pub expected_current_artifact_digest: Option<String>,
    pub new_mount_id: Option<String>,
    pub new_data_dir_path: Option<String>,
    pub config_schema_digest: String,
    pub initial_config: Value,
    pub applied_at: i64,
}

#[derive(Debug, Clone)]
pub struct RestorePluginMountParams {
    pub mount_id: String,
    pub expected_revision: i64,
    pub expected_current_artifact_digest: String,
    pub expected_previous_artifact_digest: String,
    pub restored_at: i64,
}

#[derive(Debug, Clone)]
pub struct UninstallPluginMountParams {
    pub mount_id: String,
    pub expected_revision: i64,
    pub expected_current_artifact_digest: String,
    pub uninstalled_at: i64,
}

#[derive(Debug, Clone)]
pub struct PutPluginKvParams {
    pub mount_id: String,
    pub expected_mount_revision: i64,
    pub expected_current_artifact_digest: Option<String>,
    pub namespace: String,
    pub key: String,
    pub value: Value,
    pub expected_revision: Option<i64>,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct GetPluginKvParams {
    pub mount_id: String,
    pub expected_mount_revision: i64,
    pub expected_current_artifact_digest: Option<String>,
    pub namespace: String,
    pub key: String,
}

#[derive(Debug, Clone)]
pub struct DeletePluginKvParams {
    pub mount_id: String,
    pub expected_mount_revision: i64,
    pub expected_current_artifact_digest: Option<String>,
    pub namespace: String,
    pub key: String,
    pub expected_revision: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct UpdatePluginMountConfigParams {
    pub mount_id: String,
    pub expected_mount_revision: i64,
    pub expected_current_artifact_digest: Option<String>,
    pub expected_config_revision: i64,
    pub expected_config_schema_digest: Option<String>,
    pub config_schema_digest: String,
    pub config: Value,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct ListPluginCredentialBindingsParams {
    pub mount_id: String,
    pub expected_mount_revision: i64,
    pub expected_current_artifact_digest: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReplacePluginCredentialBindingsParams {
    pub mount_id: String,
    pub expected_mount_revision: i64,
    pub expected_current_artifact_digest: Option<String>,
    pub expected_bindings_revision: i64,
    pub bindings: Vec<PluginCredentialBindingInput>,
    pub updated_at: i64,
}

#[async_trait::async_trait]
pub trait IPluginN1Repository: Send + Sync {
    async fn put_artifact(
        &self,
        params: &CreatePluginArtifactParams,
    ) -> Result<PluginArtifactRow, DbError>;

    async fn create_project(
        &self,
        params: &CreatePluginProjectParams,
    ) -> Result<PluginProjectRow, DbError>;

    async fn update_project_source_cas(
        &self,
        params: &UpdatePluginProjectSourceParams,
    ) -> Result<PluginProjectRow, DbError>;

    async fn delete_project_cas(
        &self,
        params: &DeletePluginProjectParams,
    ) -> Result<bool, DbError>;

    async fn discard_candidate(
        &self,
        params: &DiscardPluginCandidateParams,
    ) -> Result<bool, DbError>;

    async fn start_operation(
        &self,
        params: &StartProductOperationParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn finish_operation(
        &self,
        params: &FinishProductOperationParams,
    ) -> Result<ProductOperationRow, DbError>;

    async fn record_ready_candidate(
        &self,
        params: &RecordPluginReadyCandidateParams,
    ) -> Result<PluginReadyCandidateRow, DbError>;

    async fn record_candidate_test_receipt(
        &self,
        params: &RecordPluginCandidateTestReceiptParams,
    ) -> Result<PluginCandidateTestReceiptRow, DbError>;

    async fn apply_candidate(
        &self,
        params: &ApplyPluginCandidateParams,
    ) -> Result<PluginMountRow, DbError>;

    async fn restore_previous(
        &self,
        params: &RestorePluginMountParams,
    ) -> Result<PluginMountRow, DbError>;

    async fn uninstall_retain_data(
        &self,
        params: &UninstallPluginMountParams,
    ) -> Result<PluginMountRow, DbError>;

    async fn mark_mount_delete_pending(
        &self,
        mount_id: &str,
        expected_revision: i64,
        updated_at: i64,
    ) -> Result<PluginMountRow, DbError>;

    async fn complete_mount_data_delete(&self, mount_id: &str) -> Result<bool, DbError>;

    async fn update_mount_config_cas(
        &self,
        params: &UpdatePluginMountConfigParams,
    ) -> Result<PluginMountRow, DbError>;

    async fn replace_credential_bindings(
        &self,
        params: &ReplacePluginCredentialBindingsParams,
    ) -> Result<PluginCredentialBindingSnapshot, DbError>;

    async fn list_credential_bindings(
        &self,
        params: &ListPluginCredentialBindingsParams,
    ) -> Result<PluginCredentialBindingSnapshot, DbError>;

    async fn put_kv_cas(&self, params: &PutPluginKvParams) -> Result<PluginKvRow, DbError>;

    async fn get_kv(&self, params: &GetPluginKvParams)
        -> Result<Option<PluginKvRow>, DbError>;

    async fn delete_kv_cas(&self, params: &DeletePluginKvParams) -> Result<bool, DbError>;

    async fn get_project(&self, project_id: &str) -> Result<Option<PluginProjectRow>, DbError>;

    async fn get_ready_candidate(
        &self,
        project_id: &str,
    ) -> Result<Option<PluginReadyCandidateRow>, DbError>;

    async fn get_mount(&self, mount_id: &str) -> Result<Option<PluginMountRow>, DbError>;

    async fn get_mount_runtime_state(
        &self,
        params: &ListPluginCredentialBindingsParams,
    ) -> Result<Option<PluginMountRuntimeState>, DbError>;
}
