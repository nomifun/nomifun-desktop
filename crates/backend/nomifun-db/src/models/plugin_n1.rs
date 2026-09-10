use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PluginArtifactRow {
    pub id: i64,
    pub artifact_id: String,
    pub artifact_digest: String,
    pub package_id: String,
    pub package_version: String,
    pub manifest_digest: String,
    pub manifest_json: String,
    pub managed_path: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PluginProjectRow {
    pub id: i64,
    pub project_id: String,
    pub owner_user_id: String,
    pub package_id: String,
    pub display_name: String,
    pub description: String,
    pub apply_mode: String,
    pub auto_apply_mount_id: Option<String>,
    pub auto_apply_authorization_revision: i64,
    pub auto_apply_authorized_at: Option<i64>,
    pub managed_source_path: Option<String>,
    pub source_head_digest: Option<String>,
    pub dependency_lock_digest: Option<String>,
    pub build_generation: i64,
    pub linked_mount_id: Option<String>,
    pub ready_candidate_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PluginDependencyMutationIntentRow {
    pub id: i64,
    pub intent_id: String,
    pub project_id: String,
    pub owner_user_id: String,
    pub expected_project_updated_at: i64,
    pub expected_build_generation: i64,
    pub expected_source_digest: String,
    pub expected_lock_digest: String,
    pub next_source_digest: String,
    pub next_lock_digest: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PluginReadyCandidateRow {
    pub id: i64,
    pub candidate_id: String,
    pub project_id: String,
    pub candidate_digest: String,
    pub origin_kind: String,
    pub artifact_id: String,
    pub artifact_digest: String,
    pub target_package_id: String,
    pub target_package_version: String,
    pub target_manifest_digest: String,
    pub base_target_digest: Option<String>,
    pub source_snapshot_digest: Option<String>,
    pub dependency_lock_digest: Option<String>,
    pub contract_diff_json: String,
    pub origin_operation_id: String,
    pub build_generation: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PluginCandidateTestReceiptRow {
    pub id: i64,
    pub receipt_id: String,
    pub candidate_id: String,
    pub candidate_digest: String,
    pub artifact_id: String,
    pub artifact_digest: String,
    pub receipt_digest: String,
    pub runtime_fingerprint_digest: String,
    pub receipt_json: String,
    pub tested_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PluginMountRow {
    pub id: i64,
    pub mount_id: String,
    pub package_id: String,
    pub current_artifact_digest: Option<String>,
    pub previous_artifact_digest: Option<String>,
    pub current_revision_id: Option<String>,
    pub previous_revision_id: Option<String>,
    pub enabled: bool,
    pub retained: bool,
    pub delete_pending: bool,
    pub revision: i64,
    pub config_json: String,
    pub config_schema_digest: Option<String>,
    pub config_revision: i64,
    pub credential_bindings_revision: i64,
    pub data_dir_path: String,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginMountRuntimeState {
    pub mount: PluginMountRow,
    pub credential_bindings: Vec<PluginCredentialBindingRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PluginMountRevisionRow {
    pub id: i64,
    pub mount_revision_id: String,
    pub mount_id: String,
    pub revision: i64,
    pub artifact_id: String,
    pub artifact_digest: String,
    pub candidate_key: String,
    pub candidate_digest: String,
    pub base_target_digest: Option<String>,
    pub apply_authorization_kind: String,
    pub auto_apply_authorization_revision: Option<i64>,
    pub applied_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PluginCredentialBindingRow {
    pub id: i64,
    pub mount_id: String,
    pub slot: String,
    pub credential_id: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCredentialBindingInput {
    pub slot: String,
    pub credential_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCredentialBindingSnapshot {
    pub mount_id: String,
    pub mount_revision: i64,
    pub current_artifact_digest: Option<String>,
    pub bindings_revision: i64,
    pub bindings: Vec<PluginCredentialBindingRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PluginKvRow {
    pub id: i64,
    pub mount_id: String,
    pub namespace: String,
    pub key: String,
    pub value_json: String,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductOperationKind {
    Build,
    Import,
    Export,
    MiniappPermanentDelete,
}

impl ProductOperationKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Import => "import",
            Self::Export => "export",
            Self::MiniappPermanentDelete => "miniapp_permanent_delete",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginCandidateOrigin {
    Build,
    Import,
}

impl PluginCandidateOrigin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Import => "import",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductOperationState {
    Running,
    Succeeded,
    Failed,
    Canceled,
}

impl ProductOperationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct ProductOperationRow {
    pub id: i64,
    pub operation_id: String,
    pub kind: String,
    pub owner_kind: String,
    pub owner_id: String,
    pub state: String,
    pub progress_percent: Option<i64>,
    pub last_error_code: Option<String>,
    pub bounded_log_tail_json: String,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
}
