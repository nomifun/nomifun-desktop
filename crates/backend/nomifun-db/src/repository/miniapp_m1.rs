use std::collections::BTreeMap;
use std::path::Path;

use nomifun_agent_contracts::MINIAPP_RELEASE_PROFILE_VERSION;

use crate::error::DbError;
use crate::models::{
    MiniAppM1Kind, MiniAppM1LibrarySnapshot, MiniAppM1ProjectSourceState,
    MiniAppM1ReleaseOrigin, MiniAppM1ReleaseSourceKind, MiniAppM1Snapshot,
    MiniAppProjectRow, MiniAppReleaseArtifactRow, MiniAppReleaseRow, ProductOperationRow,
    ProductOperationState,
};
use crate::repository::plugin_n1::{
    MAX_PRODUCT_OPERATION_LOG_LINE_CHARS, MAX_PRODUCT_OPERATION_LOG_LINES,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct MiniAppKvRow {
    pub id: i64,
    pub miniapp_id: String,
    pub owner_user_id: String,
    pub namespace: String,
    pub key: String,
    pub value_json: String,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitMiniAppM1PointerStateParams {
    pub owner_user_id: String,
    pub miniapp_id: String,
    pub expected_product_revision: i64,
    pub expected_pointer_revision: i64,
    pub expected_active_release_epoch: i64,
    pub ready_release_id: Option<String>,
    pub ready_release_digest: Option<String>,
    pub active_release_id: Option<String>,
    pub active_release_digest: Option<String>,
    pub previous_release_id: Option<String>,
    pub previous_release_digest: Option<String>,
    pub active_release_epoch: i64,
    pub materialized_catalog_digest: String,
    pub updated_at: i64,
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

    async fn update_project_source_cas(
        &self,
        params: &UpdateMiniAppM1ProjectSourceParams,
    ) -> Result<MiniAppProjectRow, DbError>;

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

    async fn commit_pointer_state_cas(
        &self,
        params: &CommitMiniAppM1PointerStateParams,
    ) -> Result<MiniAppM1Snapshot, DbError>;

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

pub(crate) fn validate_artifact(artifact: &MiniAppReleaseArtifactRow) -> Result<(), DbError> {
    validate_uuid(&artifact.artifact_id, "artifact.artifact_id")?;
    validate_uuid(&artifact.owner_user_id, "artifact.owner_user_id")?;
    validate_digest(&artifact.artifact_digest, "artifact.artifact_digest")?;
    validate_digest(&artifact.manifest_digest, "artifact.manifest_digest")?;
    validate_json_object(&artifact.artifact_record_json, "artifact.artifact_record_json")?;
    validate_relative_path(Some(&artifact.managed_path), "artifact.managed_path")?;
    if artifact.created_at < 0 {
        return Err(conflict("artifact.created_at must be non-negative"));
    }
    Ok(())
}

pub(crate) fn validate_release(release: &MiniAppReleaseRow) -> Result<(), DbError> {
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
    validate_json_object(&release.release_record_json, "release.release_record_json")?;
    if release.created_at < 0 {
        return Err(conflict("release.created_at must be non-negative"));
    }
    Ok(())
}

pub(crate) fn validate_pointer_pair(
    id: Option<&str>,
    digest: Option<&str>,
    label: &str,
) -> Result<(), DbError> {
    if id.is_some() != digest.is_some() {
        return Err(conflict(format!(
            "{label} ID and digest must be written together"
        )));
    }
    if let Some(id) = id {
        validate_uuid(id, &format!("{label}.id"))?;
    }
    validate_optional_digest(digest, &format!("{label}.digest"))
}

pub(crate) fn validate_pointer_state(
    params: &CommitMiniAppM1PointerStateParams,
) -> Result<(), DbError> {
    validate_uuid(&params.owner_user_id, "owner_user_id")?;
    validate_uuid(&params.miniapp_id, "miniapp_id")?;
    validate_digest(
        &params.materialized_catalog_digest,
        "materialized_catalog_digest",
    )?;
    validate_pointer_pair(
        params.ready_release_id.as_deref(),
        params.ready_release_digest.as_deref(),
        "ready_release",
    )?;
    validate_pointer_pair(
        params.active_release_id.as_deref(),
        params.active_release_digest.as_deref(),
        "active_release",
    )?;
    validate_pointer_pair(
        params.previous_release_id.as_deref(),
        params.previous_release_digest.as_deref(),
        "previous_release",
    )?;
    let ids = [
        params.ready_release_id.as_deref(),
        params.active_release_id.as_deref(),
        params.previous_release_id.as_deref(),
    ];
    let populated = ids.iter().flatten().collect::<Vec<_>>();
    let mut unique = populated.clone();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() != populated.len() {
        return Err(conflict("Ready/Active/Previous Release pointers must be distinct"));
    }
    if params.active_release_id.is_none() && params.active_release_epoch != 0 {
        return Err(conflict(
            "active_release_epoch must be zero without an Active Release",
        ));
    }
    if params.active_release_id.is_some() && params.active_release_epoch == 0 {
        return Err(conflict(
            "Active Release requires a positive active_release_epoch",
        ));
    }
    if params.expected_active_release_epoch < 0
        || params.expected_product_revision < 1
        || params.expected_pointer_revision < 1
        || params.updated_at < 0
    {
        return Err(conflict("pointer CAS expectations are invalid"));
    }
    Ok(())
}

pub(crate) fn query_error(error: sqlx::Error) -> DbError {
    match &error {
        sqlx::Error::Database(database_error) => {
            conflict(format!("MiniApp M1 SQLite mutation was rejected: {}", database_error.message()))
        }
        _ => DbError::Query(error),
    }
}
