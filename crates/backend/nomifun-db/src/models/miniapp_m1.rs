use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MiniAppM1Kind {
    UiOnly,
    Service,
}

impl MiniAppM1Kind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UiOnly => "ui_only",
            Self::Service => "service",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MiniAppM1ProjectSourceState {
    Empty,
    Editable,
    RuntimeOnly,
}

impl MiniAppM1ProjectSourceState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Editable => "editable",
            Self::RuntimeOnly => "runtime_only",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MiniAppM1ReleaseOrigin {
    Build,
    Import,
}

impl MiniAppM1ReleaseOrigin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Import => "import",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MiniAppM1ReleaseSourceKind {
    Managed,
    RuntimeOnly,
}

impl MiniAppM1ReleaseSourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Managed => "managed",
            Self::RuntimeOnly => "runtime_only",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct MiniAppLibraryStateRow {
    pub id: i64,
    pub singleton_key: String,
    pub owner_user_id: String,
    pub revision: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct MiniAppProductRow {
    pub id: i64,
    pub miniapp_id: String,
    pub owner_user_id: String,
    pub product_revision: i64,
    pub display_name: String,
    pub description: Option<String>,
    pub icon_asset_id: Option<String>,
    pub kind: String,
    pub lifecycle: String,
    pub pointer_revision: i64,
    pub active_release_epoch: i64,
    pub ready_release_id: Option<String>,
    pub ready_release_digest: Option<String>,
    pub active_release_id: Option<String>,
    pub active_release_digest: Option<String>,
    pub previous_release_id: Option<String>,
    pub previous_release_digest: Option<String>,
    pub materialized_catalog_digest: String,
    pub config_schema_json: String,
    pub config_json: String,
    pub config_revision: i64,
    pub credential_bindings_revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct MiniAppProjectRow {
    pub id: i64,
    pub project_id: String,
    pub miniapp_id: String,
    pub owner_user_id: String,
    pub project_revision: i64,
    pub source_state: String,
    pub managed_source_path: Option<String>,
    pub source_head_digest: Option<String>,
    pub dependency_lock_digest: Option<String>,
    pub build_profile_version: Option<String>,
    pub build_generation: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct MiniAppSourceMutationIntentRow {
    pub id: i64,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct MiniAppKvRow {
    pub id: i64,
    pub miniapp_id: String,
    pub owner_user_id: String,
    pub namespace: String,
    pub key: String,
    pub value_json: String,
    pub revision: i64,
    pub key_generation: i64,
    pub is_tombstone: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct MiniAppReleaseArtifactRow {
    pub id: i64,
    pub artifact_id: String,
    pub owner_user_id: String,
    pub artifact_digest: String,
    pub manifest_digest: String,
    pub artifact_record_json: String,
    pub managed_path: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct MiniAppReleaseRow {
    pub id: i64,
    pub release_id: String,
    pub miniapp_id: String,
    pub owner_user_id: String,
    pub artifact_id: String,
    pub artifact_digest: String,
    pub manifest_digest: String,
    pub release_digest: String,
    pub origin_kind: String,
    pub origin_operation_id: String,
    pub source_kind: String,
    pub project_id: Option<String>,
    pub source_snapshot_digest: Option<String>,
    pub dependency_lock_digest: Option<String>,
    pub build_profile_version: Option<String>,
    pub build_generation: Option<i64>,
    pub release_record_json: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct MiniAppBuildOperationLineageRow {
    pub id: i64,
    pub operation_id: String,
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub project_id: String,
    pub project_revision: i64,
    pub source_snapshot_digest: String,
    pub dependency_lock_digest: String,
    pub build_profile_version: String,
    pub build_generation: i64,
    pub started_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct MiniAppPublishAuthorizationRow {
    pub id: i64,
    pub authorization_id: String,
    pub miniapp_id: String,
    pub owner_user_id: String,
    pub revision: i64,
    pub enabled: bool,
    pub user_authorized_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct MiniAppCatalogPublicationRow {
    pub id: i64,
    pub miniapp_id: String,
    pub owner_user_id: String,
    pub active_release_id: String,
    pub active_release_digest: String,
    pub active_release_epoch: i64,
    pub catalog_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct MiniAppSurfaceSessionRow {
    pub id: i64,
    pub surface_session_id: String,
    pub miniapp_id: String,
    pub owner_user_id: String,
    pub generation: i64,
    pub capability_digest: String,
    pub active_release_id: String,
    pub active_release_digest: String,
    pub active_release_epoch: i64,
    pub issued_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
#[allow(dead_code)]
pub struct MiniAppDeletionIntentRow {
    pub id: i64,
    pub miniapp_id: String,
    pub owner_user_id: String,
    pub operation_id: String,
    pub started_at_ms: i64,
    pub last_error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct MiniAppCredentialBindingRow {
    pub id: i64,
    pub miniapp_id: String,
    pub owner_user_id: String,
    pub slot_key: String,
    pub credential_id: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MiniAppM1Snapshot {
    pub library_revision: i64,
    pub product: MiniAppProductRow,
    pub project: MiniAppProjectRow,
    pub ready_release: Option<MiniAppReleaseRow>,
    pub active_release: Option<MiniAppReleaseRow>,
    pub previous_release: Option<MiniAppReleaseRow>,
    pub auto_publish_authorization: Option<MiniAppPublishAuthorizationRow>,
    pub catalog_publication: Option<MiniAppCatalogPublicationRow>,
    pub credential_bindings: Vec<MiniAppCredentialBindingRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MiniAppM1LibrarySnapshot {
    pub library: MiniAppLibraryStateRow,
    pub products: Vec<MiniAppProductRow>,
}
