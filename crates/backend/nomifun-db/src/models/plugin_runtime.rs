use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntimeKind {
    Plugin,
}

impl PluginRuntimeKind {
    pub const fn as_str(self) -> &'static str {
        "plugin"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntimeProjectSourceState {
    Empty,
    Editable,
    RuntimeOnly,
}

impl PluginRuntimeProjectSourceState {
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
pub enum PluginRuntimeReleaseOrigin {
    Build,
    Import,
}

impl PluginRuntimeReleaseOrigin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Import => "import",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntimeReleaseSourceKind {
    Managed,
    RuntimeOnly,
}

impl PluginRuntimeReleaseSourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Managed => "managed",
            Self::RuntimeOnly => "runtime_only",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PluginRuntimeLibraryStateRow {
    pub id: i64,
    pub singleton_key: String,
    pub owner_user_id: String,
    pub revision: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PluginRuntimeProductRow {
    pub id: i64,
    pub plugin_product_id: String,
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
pub struct PluginRuntimeProjectRow {
    pub id: i64,
    pub project_id: String,
    pub plugin_product_id: String,
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
pub struct PluginRuntimeSourceMutationIntentRow {
    pub id: i64,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PluginRuntimeKvRow {
    pub id: i64,
    pub plugin_product_id: String,
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
pub struct PluginRuntimeReleaseArtifactRow {
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
pub struct PluginRuntimeReleaseRow {
    pub id: i64,
    pub release_id: String,
    pub plugin_product_id: String,
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
pub struct PluginRuntimeBuildOperationLineageRow {
    pub id: i64,
    pub operation_id: String,
    pub owner_user_id: String,
    pub plugin_product_id: String,
    pub project_id: String,
    pub project_revision: i64,
    pub source_snapshot_digest: String,
    pub dependency_lock_digest: String,
    pub build_profile_version: String,
    pub build_generation: i64,
    pub started_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PluginRuntimePublishAuthorizationRow {
    pub id: i64,
    pub authorization_id: String,
    pub plugin_product_id: String,
    pub owner_user_id: String,
    pub revision: i64,
    pub enabled: bool,
    pub user_authorized_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PluginRuntimeCatalogPublicationRow {
    pub id: i64,
    pub plugin_product_id: String,
    pub owner_user_id: String,
    pub active_release_id: String,
    pub active_release_digest: String,
    pub active_release_epoch: i64,
    pub catalog_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PluginRuntimeSurfaceSessionRow {
    pub id: i64,
    pub surface_session_id: String,
    pub plugin_product_id: String,
    pub owner_user_id: String,
    pub generation: i64,
    pub conversation_id: Option<String>,
    pub capability_digest: String,
    pub active_release_id: String,
    pub active_release_digest: String,
    pub active_release_epoch: i64,
    pub issued_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PluginRuntimeCredentialBindingRow {
    pub id: i64,
    pub plugin_product_id: String,
    pub owner_user_id: String,
    pub slot_key: String,
    pub credential_id: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginRuntimeSnapshot {
    pub library_revision: i64,
    pub product: PluginRuntimeProductRow,
    pub project: PluginRuntimeProjectRow,
    pub ready_release: Option<PluginRuntimeReleaseRow>,
    pub active_release: Option<PluginRuntimeReleaseRow>,
    pub previous_release: Option<PluginRuntimeReleaseRow>,
    pub auto_publish_authorization: Option<PluginRuntimePublishAuthorizationRow>,
    pub catalog_publication: Option<PluginRuntimeCatalogPublicationRow>,
    pub credential_bindings: Vec<PluginRuntimeCredentialBindingRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginRuntimeLibrarySnapshot {
    pub library: PluginRuntimeLibraryStateRow,
    pub products: Vec<PluginRuntimeProductRow>,
}
