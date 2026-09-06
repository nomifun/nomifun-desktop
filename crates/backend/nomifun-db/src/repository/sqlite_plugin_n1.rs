use serde_json::Value;
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::DbError;
use crate::models::{
    PluginArtifactRow, PluginCandidateOrigin, PluginCandidateTestReceiptRow,
    PluginCredentialBindingInput, PluginCredentialBindingRow, PluginCredentialBindingSnapshot,
    PluginKvRow, PluginMountRow, PluginMountRuntimeState, PluginProjectRow,
    PluginReadyCandidateRow, ProductOperationKind, ProductOperationRow, ProductOperationState,
};
use crate::repository::plugin_n1::{
    ApplyPluginCandidateParams, CreatePluginArtifactParams, CreatePluginProjectParams,
    DeletePluginKvParams, FinishProductOperationParams, GetPluginKvParams,
    IPluginN1Repository, ListPluginCredentialBindingsParams, PutPluginKvParams,
    RecordPluginCandidateTestReceiptParams, RecordPluginReadyCandidateParams,
    ReplacePluginCredentialBindingsParams, RestorePluginMountParams, StartProductOperationParams,
    UninstallPluginMountParams, UpdatePluginMountConfigParams, UpdatePluginProjectSourceParams,
    MAX_PRODUCT_OPERATION_LOG_LINES,
    MAX_PRODUCT_OPERATION_LOG_LINE_CHARS,
};

const MAX_ERROR_CODE_CHARS: usize = 256;
const READY_CANDIDATE_SELECT: &str = "\
    SELECT candidate.*, \
           artifact.package_id AS target_package_id, \
           artifact.package_version AS target_package_version, \
           artifact.manifest_digest AS target_manifest_digest \
    FROM plugin_ready_candidates candidate \
    JOIN plugin_artifacts artifact \
      ON artifact.artifact_id = candidate.artifact_id \
     AND artifact.artifact_digest = candidate.artifact_digest";

#[derive(Clone, Debug)]
pub struct SqlitePluginN1Repository {
    pool: SqlitePool,
}

impl SqlitePluginN1Repository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn conflict(message: impl Into<String>) -> DbError {
    DbError::Conflict(message.into())
}

fn query_error(error: sqlx::Error) -> DbError {
    match &error {
        sqlx::Error::Database(database_error) => conflict(format!(
            "Plugin N1 mutation was rejected by SQLite: {}",
            database_error.message()
        )),
        _ => DbError::Query(error),
    }
}

fn validate_uuid(value: &str, label: &str) -> Result<(), DbError> {
    nomifun_common::validate_uuidv7(value)
        .map(|_| ())
        .map_err(|error| conflict(format!("{label} must be a canonical UUIDv7: {error}")))
}

fn validate_digest(value: &str, label: &str) -> Result<(), DbError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(conflict(format!(
            "{label} must be a lowercase SHA-256 hex digest"
        )))
    }
}

fn validate_optional_digest(value: Option<&str>, label: &str) -> Result<(), DbError> {
    if let Some(value) = value {
        validate_digest(value, label)?;
    }
    Ok(())
}

fn validate_timestamp(value: i64, label: &str) -> Result<(), DbError> {
    if value < 0 {
        return Err(conflict(format!("{label} must be non-negative")));
    }
    Ok(())
}

fn json_object(value: &Value, label: &str) -> Result<String, DbError> {
    if !value.is_object() {
        return Err(conflict(format!("{label} must be a JSON object")));
    }
    serde_json::to_string(value)
        .map_err(|error| conflict(format!("{label} cannot be serialized: {error}")))
}

fn json_value(value: &Value, label: &str) -> Result<String, DbError> {
    serde_json::to_string(value)
        .map_err(|error| conflict(format!("{label} cannot be serialized: {error}")))
}

fn validate_log_tail(lines: &[String]) -> Result<String, DbError> {
    if lines.len() > MAX_PRODUCT_OPERATION_LOG_LINES {
        return Err(conflict(format!(
            "product operation log tail exceeds {MAX_PRODUCT_OPERATION_LOG_LINES} lines"
        )));
    }
    if lines
        .iter()
        .any(|line| line.chars().count() > MAX_PRODUCT_OPERATION_LOG_LINE_CHARS || line.contains('\0'))
    {
        return Err(conflict(format!(
            "product operation log line exceeds {MAX_PRODUCT_OPERATION_LOG_LINE_CHARS} characters or contains NUL"
        )));
    }
    serde_json::to_string(lines)
        .map_err(|error| conflict(format!("product operation log cannot be serialized: {error}")))
}

fn validate_error_code(value: Option<&str>) -> Result<(), DbError> {
    if value.is_some_and(|code| {
        code.is_empty()
            || code.chars().count() > MAX_ERROR_CODE_CHARS
            || !code.bytes().all(|byte| byte.is_ascii_graphic())
    }) {
        return Err(conflict(
            "product operation last_error_code must contain 1 to 256 visible ASCII characters",
        ));
    }
    Ok(())
}

async fn fetch_candidate_by_id(
    tx: &mut Transaction<'_, Sqlite>,
    candidate_id: &str,
) -> Result<PluginReadyCandidateRow, DbError> {
    sqlx::query_as::<_, PluginReadyCandidateRow>(&format!(
        "{READY_CANDIDATE_SELECT} WHERE candidate.candidate_id = ?"
    ))
    .bind(candidate_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(DbError::Query)
}

async fn lock_project(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
) -> Result<PluginProjectRow, DbError> {
    let locked =
        sqlx::query("UPDATE plugin_projects SET updated_at = updated_at WHERE project_id = ?")
            .bind(project_id)
            .execute(&mut **tx)
            .await
            .map_err(query_error)?;
    if locked.rows_affected() != 1 {
        return Err(DbError::NotFound(format!("plugin project {project_id}")));
    }
    sqlx::query_as("SELECT * FROM plugin_projects WHERE project_id = ?")
        .bind(project_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(DbError::Query)
}

async fn lock_mount(
    tx: &mut Transaction<'_, Sqlite>,
    mount_id: &str,
) -> Result<PluginMountRow, DbError> {
    let locked = sqlx::query("UPDATE plugin_mounts SET updated_at = updated_at WHERE mount_id = ?")
        .bind(mount_id)
        .execute(&mut **tx)
        .await
        .map_err(query_error)?;
    if locked.rows_affected() != 1 {
        return Err(DbError::NotFound(format!("plugin mount {mount_id}")));
    }
    sqlx::query_as("SELECT * FROM plugin_mounts WHERE mount_id = ?")
        .bind(mount_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(DbError::Query)
}

fn require_project_generation(
    project: &PluginProjectRow,
    expected_generation: i64,
) -> Result<(), DbError> {
    if project.build_generation != expected_generation {
        return Err(conflict(format!(
            "plugin project {} generation changed from expected {} to {}",
            project.project_id, expected_generation, project.build_generation
        )));
    }
    Ok(())
}

fn require_mount_cas(
    mount: &PluginMountRow,
    expected_revision: i64,
    expected_current: Option<&str>,
) -> Result<(), DbError> {
    if mount.revision != expected_revision
        || mount.current_artifact_digest.as_deref() != expected_current
    {
        return Err(conflict(format!(
            "plugin mount {} changed before the requested transition",
            mount.mount_id
        )));
    }
    Ok(())
}

fn require_mount_runtime_cas(
    mount: &PluginMountRow,
    expected_revision: i64,
    expected_current: Option<&str>,
) -> Result<(), DbError> {
    require_mount_cas(mount, expected_revision, expected_current)?;
    if mount.delete_pending {
        return Err(conflict("plugin mount data is pending deletion"));
    }
    Ok(())
}

fn validate_binding_input(
    binding: &PluginCredentialBindingInput,
) -> Result<(), DbError> {
    if binding.slot.is_empty()
        || binding.slot.len() > 128
        || !binding.slot.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(conflict("plugin credential binding slot must be visible ASCII"));
    }
    if binding.credential_id.is_empty()
        || binding.credential_id.len() > 512
        || !binding
            .credential_id
            .bytes()
            .all(|byte| byte.is_ascii_graphic())
    {
        return Err(conflict(
            "plugin credential binding credential_id must be visible ASCII",
        ));
    }
    Ok(())
}

async fn fetch_bindings(
    executor: &mut sqlx::SqliteConnection,
    mount_id: &str,
) -> Result<Vec<PluginCredentialBindingRow>, DbError> {
    sqlx::query_as(
        "SELECT * FROM plugin_credential_bindings
         WHERE mount_id = ? ORDER BY slot ASC",
    )
    .bind(mount_id)
    .fetch_all(&mut *executor)
    .await
    .map_err(DbError::Query)
}

fn validate_operation_owner(kind: ProductOperationKind, owner_kind: &str) -> Result<(), DbError> {
    let valid = match kind {
        ProductOperationKind::MiniappPermanentDelete => owner_kind == "miniapp",
        ProductOperationKind::Build => matches!(owner_kind, "plugin_project" | "miniapp"),
        ProductOperationKind::Import | ProductOperationKind::Export => {
            matches!(owner_kind, "plugin_project" | "plugin_mount" | "miniapp")
        }
    };
    if valid {
        Ok(())
    } else {
        Err(conflict("product operation kind and owner kind are incompatible"))
    }
}

#[async_trait::async_trait]
impl IPluginN1Repository for SqlitePluginN1Repository {
    async fn put_artifact(
        &self,
        params: &CreatePluginArtifactParams,
    ) -> Result<PluginArtifactRow, DbError> {
        validate_uuid(&params.artifact_id, "artifact_id")?;
        validate_digest(&params.artifact_digest, "artifact_digest")?;
        validate_digest(&params.manifest_digest, "manifest_digest")?;
        validate_timestamp(params.created_at, "created_at")?;
        let manifest_json = json_object(&params.manifest, "plugin manifest")?;
        sqlx::query(
            "INSERT INTO plugin_artifacts (
                artifact_id, artifact_digest, package_id, package_version,
                manifest_digest, manifest_json, managed_path, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(artifact_digest) DO NOTHING",
        )
        .bind(&params.artifact_id)
        .bind(&params.artifact_digest)
        .bind(&params.package_id)
        .bind(&params.package_version)
        .bind(&params.manifest_digest)
        .bind(&manifest_json)
        .bind(&params.managed_path)
        .bind(params.created_at)
        .execute(&self.pool)
        .await
        .map_err(query_error)?;
        let artifact: PluginArtifactRow =
            sqlx::query_as("SELECT * FROM plugin_artifacts WHERE artifact_digest = ?")
                .bind(&params.artifact_digest)
                .fetch_one(&self.pool)
                .await
                .map_err(DbError::Query)?;
        if artifact.artifact_id != params.artifact_id
            || artifact.package_id != params.package_id
            || artifact.package_version != params.package_version
            || artifact.manifest_digest != params.manifest_digest
            || artifact.manifest_json != manifest_json
            || artifact.managed_path != params.managed_path
        {
            return Err(conflict(format!(
                "artifact digest {} is already bound to different immutable content",
                params.artifact_digest
            )));
        }
        Ok(artifact)
    }

    async fn create_project(
        &self,
        params: &CreatePluginProjectParams,
    ) -> Result<PluginProjectRow, DbError> {
        validate_uuid(&params.project_id, "project_id")?;
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_optional_digest(params.source_head_digest.as_deref(), "source_head_digest")?;
        validate_optional_digest(
            params.dependency_lock_digest.as_deref(),
            "dependency_lock_digest",
        )?;
        validate_timestamp(params.created_at, "created_at")?;
        sqlx::query(
            "INSERT INTO plugin_projects (
                project_id, owner_user_id, package_id, managed_source_path,
                source_head_digest, dependency_lock_digest, created_at, updated_at
             )
             SELECT ?, ?, ?, ?, ?, ?, ?, ?
             WHERE EXISTS (SELECT 1 FROM users WHERE user_id = ?)",
        )
        .bind(&params.project_id)
        .bind(&params.owner_user_id)
        .bind(&params.package_id)
        .bind(&params.managed_source_path)
        .bind(&params.source_head_digest)
        .bind(&params.dependency_lock_digest)
        .bind(params.created_at)
        .bind(params.created_at)
        .bind(&params.owner_user_id)
        .execute(&self.pool)
        .await
        .map_err(query_error)?;
        self.get_project(&params.project_id)
            .await?
            .ok_or_else(|| conflict("plugin project owner does not exist"))
    }

    async fn update_project_source_cas(
        &self,
        params: &UpdatePluginProjectSourceParams,
    ) -> Result<PluginProjectRow, DbError> {
        validate_uuid(&params.project_id, "project_id")?;
        validate_digest(&params.source_head_digest, "source_head_digest")?;
        validate_optional_digest(
            params.dependency_lock_digest.as_deref(),
            "dependency_lock_digest",
        )?;
        validate_timestamp(params.updated_at, "updated_at")?;
        let result = sqlx::query(
            "UPDATE plugin_projects
             SET source_head_digest = ?, dependency_lock_digest = ?,
                 build_generation = build_generation + 1, updated_at = ?
             WHERE project_id = ? AND build_generation = ?
               AND managed_source_path IS NOT NULL
               AND updated_at <= ?",
        )
        .bind(&params.source_head_digest)
        .bind(&params.dependency_lock_digest)
        .bind(params.updated_at)
        .bind(&params.project_id)
        .bind(params.expected_generation)
        .bind(params.updated_at)
        .execute(&self.pool)
        .await
        .map_err(query_error)?;
        if result.rows_affected() != 1 {
            return Err(conflict(format!(
                "plugin project {} source generation CAS failed",
                params.project_id
            )));
        }
        self.get_project(&params.project_id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("plugin project {}", params.project_id)))
    }

    async fn start_operation(
        &self,
        params: &StartProductOperationParams,
    ) -> Result<ProductOperationRow, DbError> {
        validate_uuid(&params.operation_id, "operation_id")?;
        validate_uuid(&params.owner_id, "owner_id")?;
        if params.started_at_ms <= 0 {
            return Err(conflict("product operation started_at_ms must be positive"));
        }
        validate_operation_owner(params.kind, &params.owner_kind)?;
        if params.kind == ProductOperationKind::MiniappPermanentDelete
            && params.progress_percent.is_some()
        {
            return Err(conflict(
                "miniapp permanent delete does not persist progress_percent",
            ));
        }
        let bounded_log_tail_json = validate_log_tail(&params.bounded_log_tail)?;
        let mut tx = self.pool.begin().await?;
        let owner_exists = match params.owner_kind.as_str() {
            "plugin_project" => {
                let project: Option<(Option<String>, Option<String>, Option<String>, i64)> =
                    sqlx::query_as(
                        "SELECT managed_source_path, source_head_digest,
                                dependency_lock_digest, build_generation
                         FROM plugin_projects WHERE project_id = ?",
                    )
                .bind(&params.owner_id)
                .fetch_optional(&mut *tx)
                .await?;
                if params.kind == ProductOperationKind::Build
                    && project.as_ref().is_some_and(
                        |(managed_source_path, source_head_digest, dependency_lock_digest, generation)| {
                            managed_source_path.is_none()
                                || source_head_digest.is_none()
                                || dependency_lock_digest.is_none()
                                || *generation <= 0
                        },
                    )
                {
                    return Err(conflict(
                        "Plugin Project build requires managed source, dependency lock, and positive generation",
                    ));
                }
                project.is_some()
            }
            "plugin_mount" => {
                sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM plugin_mounts WHERE mount_id = ?)",
                )
                .bind(&params.owner_id)
                .fetch_one(&mut *tx)
                .await?
            }
            "miniapp" => true,
            _ => false,
        };
        if !owner_exists {
            return Err(conflict(format!(
                "product operation owner {} does not exist",
                params.owner_id
            )));
        }
        sqlx::query(
            "INSERT INTO product_operations (
                operation_id, kind, owner_kind, owner_id, state, progress_percent,
                bounded_log_tail_json, started_at_ms
             ) VALUES (?, ?, ?, ?, 'running', ?, ?, ?)",
        )
        .bind(&params.operation_id)
        .bind(params.kind.as_str())
        .bind(&params.owner_kind)
        .bind(&params.owner_id)
        .bind(params.progress_percent.map(i64::from))
        .bind(bounded_log_tail_json)
        .bind(params.started_at_ms)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let operation =
            sqlx::query_as("SELECT * FROM product_operations WHERE operation_id = ?")
            .bind(&params.operation_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(DbError::Query)?;
        tx.commit().await?;
        Ok(operation)
    }

    async fn finish_operation(
        &self,
        params: &FinishProductOperationParams,
    ) -> Result<ProductOperationRow, DbError> {
        validate_uuid(&params.operation_id, "operation_id")?;
        if params.finished_at_ms <= 0 {
            return Err(conflict("product operation finished_at_ms must be positive"));
        }
        if params.state == ProductOperationState::Running {
            return Err(conflict("finish_operation requires a terminal status"));
        }
        validate_error_code(params.last_error_code.as_deref())?;
        let bounded_log_tail_json = validate_log_tail(&params.bounded_log_tail)?;
        if (params.state == ProductOperationState::Failed) != params.last_error_code.is_some() {
            return Err(conflict(
                "only failed product operations carry last_error_code",
            ));
        }
        let mut tx = self.pool.begin().await?;
        let current: ProductOperationRow =
            sqlx::query_as("SELECT * FROM product_operations WHERE operation_id = ?")
                .bind(&params.operation_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| DbError::NotFound(format!("operation {}", params.operation_id)))?;
        if current.state != ProductOperationState::Running.as_str() {
            return Err(conflict(format!(
                "operation {} is already terminal",
                params.operation_id
            )));
        }
        if params.finished_at_ms < current.started_at_ms {
            return Err(conflict(
                "operation terminal timestamp predates started_at_ms",
            ));
        }
        if current.kind == ProductOperationKind::MiniappPermanentDelete.as_str()
            && params.state == ProductOperationState::Canceled
        {
            return Err(conflict("miniapp permanent delete cannot be canceled"));
        }
        if current.kind == ProductOperationKind::MiniappPermanentDelete.as_str()
            && params.progress_percent.is_some()
        {
            return Err(conflict(
                "miniapp permanent delete does not persist progress_percent",
            ));
        }
        if current.kind != ProductOperationKind::MiniappPermanentDelete.as_str()
            && params.state == ProductOperationState::Succeeded
            && params.progress_percent != Some(100)
        {
            return Err(conflict(
                "successful progress-reporting operation must finish at 100",
            ));
        }
        sqlx::query(
            "UPDATE product_operations
             SET state = ?, progress_percent = ?, last_error_code = ?,
                 bounded_log_tail_json = ?, finished_at_ms = ?
             WHERE operation_id = ? AND state = 'running'",
        )
        .bind(params.state.as_str())
        .bind(params.progress_percent.map(i64::from))
        .bind(&params.last_error_code)
        .bind(bounded_log_tail_json)
        .bind(params.finished_at_ms)
        .bind(&params.operation_id)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let updated = sqlx::query_as("SELECT * FROM product_operations WHERE operation_id = ?")
            .bind(&params.operation_id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(updated)
    }

    async fn record_ready_candidate(
        &self,
        params: &RecordPluginReadyCandidateParams,
    ) -> Result<PluginReadyCandidateRow, DbError> {
        validate_uuid(&params.candidate_id, "candidate_id")?;
        validate_uuid(&params.project_id, "project_id")?;
        validate_uuid(&params.origin_operation_id, "origin_operation_id")?;
        validate_digest(&params.candidate_digest, "candidate_digest")?;
        validate_uuid(&params.artifact_id, "artifact_id")?;
        validate_digest(&params.artifact_digest, "artifact_digest")?;
        validate_optional_digest(
            params.base_target_digest.as_deref(),
            "base_target_digest",
        )?;
        validate_optional_digest(
            params.source_snapshot_digest.as_deref(),
            "source_snapshot_digest",
        )?;
        validate_optional_digest(
            params.dependency_lock_digest.as_deref(),
            "dependency_lock_digest",
        )?;
        validate_timestamp(params.created_at, "created_at")?;
        let contract_diff_json = json_object(&params.contract_diff, "contract_diff")?;
        let mut tx = self.pool.begin().await?;
        let project = lock_project(&mut tx, &params.project_id).await?;
        require_project_generation(&project, params.expected_generation)?;
        if project.updated_at > params.created_at {
            return Err(conflict("candidate predates the current project generation"));
        }
        let origin_operation: ProductOperationRow =
            sqlx::query_as("SELECT * FROM product_operations WHERE operation_id = ?")
                .bind(&params.origin_operation_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| {
                    conflict(format!(
                        "candidate origin operation {} does not exist",
                        params.origin_operation_id
                    ))
                })?;
        if origin_operation.owner_kind != "plugin_project"
            || origin_operation.owner_id != project.project_id
            || origin_operation.kind != params.origin.as_str()
            || origin_operation.state != ProductOperationState::Succeeded.as_str()
        {
            return Err(conflict(
                "candidate origin must be the exact successful Project build or import operation",
            ));
        }
        if project.managed_source_path.is_some() {
            if params.expected_generation <= 0
                || params.source_snapshot_digest.is_none()
                || params.dependency_lock_digest.is_none()
                || params.source_snapshot_digest != project.source_head_digest
                || params.dependency_lock_digest != project.dependency_lock_digest
            {
                return Err(conflict(
                    "managed candidate requires positive generation and exact source/dependency lineage",
                ));
            }
        } else {
            if params.origin != PluginCandidateOrigin::Import {
                return Err(conflict(
                    "source-less read-only Project can only produce an import candidate",
                ));
            }
            if params.source_snapshot_digest.is_some()
                || params.dependency_lock_digest.is_some()
            {
                return Err(conflict(
                    "runtime-only import cannot claim managed source or dependency lineage",
                ));
            }
        }
        if let Some(old_candidate_id) = project.ready_candidate_id.as_deref() {
            sqlx::query("DELETE FROM plugin_candidate_test_receipts WHERE candidate_id = ?")
                .bind(old_candidate_id)
                .execute(&mut *tx)
                .await
                .map_err(query_error)?;
            sqlx::query("DELETE FROM plugin_ready_candidates WHERE candidate_id = ?")
                .bind(old_candidate_id)
                .execute(&mut *tx)
                .await
                .map_err(query_error)?;
        }
        sqlx::query(
            "INSERT INTO plugin_ready_candidates (
                candidate_id, project_id, candidate_digest, origin_kind,
                artifact_id, artifact_digest,
                base_target_digest, source_snapshot_digest, dependency_lock_digest,
                contract_diff_json, origin_operation_id, build_generation, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&params.candidate_id)
        .bind(&params.project_id)
        .bind(&params.candidate_digest)
        .bind(params.origin.as_str())
        .bind(&params.artifact_id)
        .bind(&params.artifact_digest)
        .bind(&params.base_target_digest)
        .bind(&params.source_snapshot_digest)
        .bind(&params.dependency_lock_digest)
        .bind(contract_diff_json)
        .bind(&params.origin_operation_id)
        .bind(params.expected_generation)
        .bind(params.created_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let pointed = sqlx::query(
            "UPDATE plugin_projects
             SET ready_candidate_id = ?, updated_at = ?
             WHERE project_id = ? AND build_generation = ?",
        )
        .bind(&params.candidate_id)
        .bind(params.created_at)
        .bind(&params.project_id)
        .bind(params.expected_generation)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if pointed.rows_affected() != 1 {
            return Err(conflict("plugin candidate lost its project generation CAS"));
        }
        let candidate = fetch_candidate_by_id(&mut tx, &params.candidate_id).await?;
        tx.commit().await?;
        Ok(candidate)
    }

    async fn record_candidate_test_receipt(
        &self,
        params: &RecordPluginCandidateTestReceiptParams,
    ) -> Result<PluginCandidateTestReceiptRow, DbError> {
        validate_uuid(&params.receipt_id, "receipt_id")?;
        validate_uuid(&params.candidate_id, "candidate_id")?;
        validate_digest(&params.candidate_digest, "candidate_digest")?;
        validate_uuid(&params.artifact_id, "artifact_id")?;
        validate_digest(&params.artifact_digest, "artifact_digest")?;
        validate_digest(&params.receipt_digest, "receipt_digest")?;
        validate_digest(
            &params.runtime_fingerprint_digest,
            "runtime_fingerprint_digest",
        )?;
        validate_timestamp(params.tested_at, "tested_at")?;
        let receipt_json = json_object(&params.receipt, "candidate test receipt")?;
        sqlx::query(
            "INSERT INTO plugin_candidate_test_receipts (
                receipt_id, candidate_id, candidate_digest, artifact_id, artifact_digest,
                receipt_digest, runtime_fingerprint_digest, receipt_json, tested_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&params.receipt_id)
        .bind(&params.candidate_id)
        .bind(&params.candidate_digest)
        .bind(&params.artifact_id)
        .bind(&params.artifact_digest)
        .bind(&params.receipt_digest)
        .bind(&params.runtime_fingerprint_digest)
        .bind(receipt_json)
        .bind(params.tested_at)
        .execute(&self.pool)
        .await
        .map_err(query_error)?;
        sqlx::query_as("SELECT * FROM plugin_candidate_test_receipts WHERE receipt_id = ?")
            .bind(&params.receipt_id)
            .fetch_one(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn apply_candidate(
        &self,
        params: &ApplyPluginCandidateParams,
    ) -> Result<PluginMountRow, DbError> {
        validate_uuid(&params.project_id, "project_id")?;
        validate_uuid(&params.candidate_id, "candidate_id")?;
        validate_timestamp(params.applied_at, "applied_at")?;
        validate_optional_digest(
            params.expected_current_artifact_digest.as_deref(),
            "expected_current_artifact_digest",
        )?;
        validate_digest(&params.config_schema_digest, "config_schema_digest")?;
        let initial_config_json = json_object(&params.initial_config, "initial_config")?;
        let mut tx = self.pool.begin().await?;
        let project = lock_project(&mut tx, &params.project_id).await?;
        require_project_generation(&project, params.expected_project_generation)?;
        if project.ready_candidate_id.as_deref() != Some(&params.candidate_id) {
            return Err(conflict("candidate is not the project's exact ready candidate"));
        }
        let candidate = fetch_candidate_by_id(&mut tx, &params.candidate_id).await?;
        let artifact: PluginArtifactRow = sqlx::query_as(
            "SELECT * FROM plugin_artifacts
             WHERE artifact_id = ? AND artifact_digest = ?",
        )
                .bind(&candidate.artifact_id)
                .bind(&candidate.artifact_digest)
                .fetch_one(&mut *tx)
                .await
                .map_err(DbError::Query)?;
        if artifact.package_id != project.package_id {
            return Err(conflict("candidate artifact package differs from its project"));
        }

        let mut mount = if let Some(mount_id) = project.linked_mount_id.as_deref() {
            if params.new_mount_id.is_some() || params.new_data_dir_path.is_some() {
                return Err(conflict("linked project cannot provide a replacement mount identity"));
            }
            lock_mount(&mut tx, mount_id).await?
        } else {
            let mount_id = params.new_mount_id.as_deref().ok_or_else(|| {
                conflict("first candidate apply requires a stable new_mount_id")
            })?;
            let data_dir_path = params.new_data_dir_path.as_deref().ok_or_else(|| {
                conflict("first candidate apply requires a stable new_data_dir_path")
            })?;
            validate_uuid(mount_id, "new_mount_id")?;
            if params.expected_mount_revision != Some(0)
                || params.expected_current_artifact_digest.is_some()
                || candidate.base_target_digest.is_some()
            {
                return Err(conflict("first candidate apply must target an empty revision zero mount"));
            }
            let collision: Option<String> =
                sqlx::query_scalar("SELECT mount_id FROM plugin_mounts WHERE package_id = ?")
                    .bind(&project.package_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            if collision.is_some() {
                return Err(conflict(
                    "plugin package already has an active or retained stable mount",
                ));
            }
            sqlx::query(
                "INSERT INTO plugin_mounts (
                    mount_id, package_id, data_dir_path, created_at, updated_at
                 ) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(mount_id)
            .bind(&project.package_id)
            .bind(data_dir_path)
            .bind(params.applied_at)
            .bind(params.applied_at)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?;
            lock_mount(&mut tx, mount_id).await?
        };
        let expected_revision = params
            .expected_mount_revision
            .ok_or_else(|| conflict("candidate apply requires expected_mount_revision"))?;
        require_mount_cas(
            &mount,
            expected_revision,
            params.expected_current_artifact_digest.as_deref(),
        )?;
        if mount.package_id != project.package_id || mount.delete_pending {
            return Err(conflict("plugin mount is not eligible for candidate apply"));
        }
        if candidate.base_target_digest != mount.current_artifact_digest {
            return Err(conflict(
                "candidate base target is stale relative to the mount current artifact",
            ));
        }
        let config_changed = mount.config_json != initial_config_json
            || mount.config_schema_digest.as_deref()
                != Some(params.config_schema_digest.as_str());
        let next_config_revision = if config_changed {
            mount
                .config_revision
                .checked_add(1)
                .ok_or_else(|| conflict("plugin config revision overflow"))?
        } else {
            mount.config_revision
        };
        let mount_revision_id = nomifun_common::generate_id();
        let next_revision = mount.revision + 1;
        sqlx::query(
            "INSERT INTO plugin_mount_revisions (
                mount_revision_id, mount_id, revision, artifact_id, artifact_digest,
                candidate_key, candidate_digest, base_target_digest, applied_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&mount_revision_id)
        .bind(&mount.mount_id)
        .bind(next_revision)
        .bind(&candidate.artifact_id)
        .bind(&candidate.artifact_digest)
        .bind(&candidate.candidate_id)
        .bind(&candidate.candidate_digest)
        .bind(&candidate.base_target_digest)
        .bind(params.applied_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let transitioned = sqlx::query(
            "UPDATE plugin_mounts
             SET previous_artifact_digest = current_artifact_digest,
                 previous_revision_id = current_revision_id,
                 current_artifact_digest = ?,
                 current_revision_id = ?,
                 enabled = 1,
                 retained = 0,
                 delete_pending = 0,
                 revision = ?,
                 config_json = ?,
                 config_schema_digest = ?,
                 config_revision = ?,
                 last_error = NULL,
                 updated_at = ?
             WHERE mount_id = ? AND revision = ?
               AND current_artifact_digest IS ?",
        )
        .bind(&candidate.artifact_digest)
        .bind(&mount_revision_id)
        .bind(next_revision)
        .bind(&initial_config_json)
        .bind(&params.config_schema_digest)
        .bind(next_config_revision)
        .bind(params.applied_at)
        .bind(&mount.mount_id)
        .bind(expected_revision)
        .bind(params.expected_current_artifact_digest.as_deref())
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if transitioned.rows_affected() != 1 {
            return Err(conflict("plugin mount candidate apply lost its exact CAS"));
        }
        sqlx::query(
            "UPDATE plugin_projects
             SET linked_mount_id = ?, ready_candidate_id = NULL, updated_at = ?
             WHERE project_id = ? AND build_generation = ? AND ready_candidate_id = ?",
        )
        .bind(&mount.mount_id)
        .bind(params.applied_at)
        .bind(&project.project_id)
        .bind(params.expected_project_generation)
        .bind(&candidate.candidate_id)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        sqlx::query("DELETE FROM plugin_candidate_test_receipts WHERE candidate_id = ?")
            .bind(&candidate.candidate_id)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?;
        sqlx::query("DELETE FROM plugin_ready_candidates WHERE candidate_id = ?")
            .bind(&candidate.candidate_id)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?;
        mount = sqlx::query_as("SELECT * FROM plugin_mounts WHERE mount_id = ?")
            .bind(&mount.mount_id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(mount)
    }

    async fn restore_previous(
        &self,
        params: &RestorePluginMountParams,
    ) -> Result<PluginMountRow, DbError> {
        validate_uuid(&params.mount_id, "mount_id")?;
        validate_digest(
            &params.expected_current_artifact_digest,
            "expected_current_artifact_digest",
        )?;
        validate_digest(
            &params.expected_previous_artifact_digest,
            "expected_previous_artifact_digest",
        )?;
        validate_timestamp(params.restored_at, "restored_at")?;
        let mut tx = self.pool.begin().await?;
        let mount = lock_mount(&mut tx, &params.mount_id).await?;
        require_mount_cas(
            &mount,
            params.expected_revision,
            Some(&params.expected_current_artifact_digest),
        )?;
        if mount.previous_artifact_digest.as_deref()
            != Some(&params.expected_previous_artifact_digest)
            || mount.previous_revision_id.is_none()
            || mount.current_revision_id.is_none()
            || mount.retained
            || mount.delete_pending
        {
            return Err(conflict("plugin restore previous exact CAS failed"));
        }
        sqlx::query(
            "UPDATE plugin_mounts
             SET current_artifact_digest = previous_artifact_digest,
                 current_revision_id = previous_revision_id,
                 previous_artifact_digest = ?,
                 previous_revision_id = ?,
                 enabled = 1,
                 revision = revision + 1,
                 last_error = NULL,
                 updated_at = ?
             WHERE mount_id = ? AND revision = ?
               AND current_artifact_digest = ?
               AND previous_artifact_digest = ?",
        )
        .bind(&params.expected_current_artifact_digest)
        .bind(mount.current_revision_id.as_deref())
        .bind(params.restored_at)
        .bind(&params.mount_id)
        .bind(params.expected_revision)
        .bind(&params.expected_current_artifact_digest)
        .bind(&params.expected_previous_artifact_digest)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let updated = sqlx::query_as("SELECT * FROM plugin_mounts WHERE mount_id = ?")
            .bind(&params.mount_id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(updated)
    }

    async fn uninstall_retain_data(
        &self,
        params: &UninstallPluginMountParams,
    ) -> Result<PluginMountRow, DbError> {
        validate_uuid(&params.mount_id, "mount_id")?;
        validate_digest(
            &params.expected_current_artifact_digest,
            "expected_current_artifact_digest",
        )?;
        validate_timestamp(params.uninstalled_at, "uninstalled_at")?;
        let mut tx = self.pool.begin().await?;
        let mount = lock_mount(&mut tx, &params.mount_id).await?;
        require_mount_cas(
            &mount,
            params.expected_revision,
            Some(&params.expected_current_artifact_digest),
        )?;
        sqlx::query(
            "UPDATE plugin_mounts
             SET current_artifact_digest = NULL, previous_artifact_digest = NULL,
                 current_revision_id = NULL, previous_revision_id = NULL,
                 enabled = 0, retained = 1, revision = revision + 1,
                 last_error = NULL, updated_at = ?
             WHERE mount_id = ? AND revision = ? AND current_artifact_digest = ?",
        )
        .bind(params.uninstalled_at)
        .bind(&params.mount_id)
        .bind(params.expected_revision)
        .bind(&params.expected_current_artifact_digest)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let updated = sqlx::query_as("SELECT * FROM plugin_mounts WHERE mount_id = ?")
            .bind(&params.mount_id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(updated)
    }

    async fn mark_mount_delete_pending(
        &self,
        mount_id: &str,
        expected_revision: i64,
        updated_at: i64,
    ) -> Result<PluginMountRow, DbError> {
        validate_uuid(mount_id, "mount_id")?;
        validate_timestamp(updated_at, "updated_at")?;
        let mut tx = self.pool.begin().await?;
        let mount = lock_mount(&mut tx, mount_id).await?;
        if mount.delete_pending {
            tx.commit().await?;
            return Ok(mount);
        }
        if !mount.retained
            || mount.current_artifact_digest.is_some()
            || mount.previous_artifact_digest.is_some()
            || mount.revision != expected_revision
        {
            return Err(conflict(
                "plugin mount data deletion requires an exact retained, uninstalled mount",
            ));
        }
        sqlx::query(
            "UPDATE plugin_mounts
             SET delete_pending = 1, revision = revision + 1, last_error = NULL, updated_at = ?
             WHERE mount_id = ? AND revision = ? AND retained = 1 AND delete_pending = 0",
        )
        .bind(updated_at)
        .bind(mount_id)
        .bind(expected_revision)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let updated = sqlx::query_as("SELECT * FROM plugin_mounts WHERE mount_id = ?")
            .bind(mount_id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(updated)
    }

    async fn complete_mount_data_delete(&self, mount_id: &str) -> Result<bool, DbError> {
        validate_uuid(mount_id, "mount_id")?;
        let mut tx = self.pool.begin().await?;
        let mount = match lock_mount(&mut tx, mount_id).await {
            Ok(mount) => mount,
            Err(DbError::NotFound(_)) => {
                tx.commit().await?;
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        if !mount.delete_pending
            || !mount.retained
            || mount.current_artifact_digest.is_some()
            || mount.previous_artifact_digest.is_some()
        {
            return Err(conflict("plugin mount does not have a deletable pending-data intent"));
        }
        sqlx::query(
            "UPDATE plugin_projects
             SET linked_mount_id = NULL,
                 build_generation = build_generation + 1,
                 updated_at = MAX(updated_at, ?)
             WHERE linked_mount_id = ?",
        )
        .bind(mount.updated_at)
        .bind(mount_id)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let target_bindings_revision = mount
            .credential_bindings_revision
            .checked_add(1)
            .ok_or_else(|| conflict("plugin credential bindings revision overflow"))?;
        sqlx::query(
            "INSERT INTO plugin_credential_binding_mutations (
                mount_id, expected_mount_revision, expected_current_artifact_digest,
                expected_bindings_revision, target_bindings_revision,
                allow_delete_pending, updated_at
             ) VALUES (?, ?, NULL, ?, ?, 1, ?)",
        )
        .bind(mount_id)
        .bind(mount.revision)
        .bind(mount.credential_bindings_revision)
        .bind(target_bindings_revision)
        .bind(mount.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        sqlx::query("DELETE FROM plugin_credential_bindings WHERE mount_id = ?")
            .bind(mount_id)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?;
        sqlx::query("DELETE FROM plugin_credential_binding_mutations WHERE mount_id = ?")
            .bind(mount_id)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?;
        sqlx::query("DELETE FROM plugin_kv WHERE mount_id = ?")
            .bind(mount_id)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?;
        sqlx::query("DELETE FROM plugin_mount_revisions WHERE mount_id = ?")
            .bind(mount_id)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?;
        sqlx::query("DELETE FROM plugin_mounts WHERE mount_id = ?")
            .bind(mount_id)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?;
        tx.commit().await?;
        Ok(true)
    }

    async fn update_mount_config_cas(
        &self,
        params: &UpdatePluginMountConfigParams,
    ) -> Result<PluginMountRow, DbError> {
        validate_uuid(&params.mount_id, "mount_id")?;
        validate_optional_digest(
            params.expected_current_artifact_digest.as_deref(),
            "expected_current_artifact_digest",
        )?;
        validate_optional_digest(
            params.expected_config_schema_digest.as_deref(),
            "expected_config_schema_digest",
        )?;
        validate_digest(&params.config_schema_digest, "config_schema_digest")?;
        validate_timestamp(params.updated_at, "updated_at")?;
        let config_json = json_object(&params.config, "plugin config")?;
        let mut tx = self.pool.begin().await?;
        let mount = lock_mount(&mut tx, &params.mount_id).await?;
        require_mount_runtime_cas(
            &mount,
            params.expected_mount_revision,
            params.expected_current_artifact_digest.as_deref(),
        )?;
        if mount.config_revision != params.expected_config_revision
            || mount.config_schema_digest != params.expected_config_schema_digest
        {
            return Err(conflict("plugin config revision or schema digest changed"));
        }
        if params.updated_at < mount.updated_at {
            return Err(conflict("plugin config timestamp predates the Mount state"));
        }
        if mount.config_json == config_json
            && mount.config_schema_digest.as_deref()
                == Some(params.config_schema_digest.as_str())
        {
            tx.commit().await?;
            return Ok(mount);
        }
        let next_revision = mount
            .config_revision
            .checked_add(1)
            .ok_or_else(|| conflict("plugin config revision overflow"))?;
        let changed = sqlx::query(
            "UPDATE plugin_mounts
             SET config_json = ?, config_schema_digest = ?, config_revision = ?,
                 updated_at = ?
             WHERE mount_id = ? AND revision = ?
               AND current_artifact_digest IS ?
               AND config_revision = ?
               AND config_schema_digest IS ?",
        )
        .bind(config_json)
        .bind(&params.config_schema_digest)
        .bind(next_revision)
        .bind(params.updated_at)
        .bind(&params.mount_id)
        .bind(params.expected_mount_revision)
        .bind(params.expected_current_artifact_digest.as_deref())
        .bind(params.expected_config_revision)
        .bind(params.expected_config_schema_digest.as_deref())
        .execute(&mut *tx)
        .await
        .map_err(query_error)?
        .rows_affected();
        if changed != 1 {
            return Err(conflict("plugin config exact CAS failed"));
        }
        let updated = sqlx::query_as("SELECT * FROM plugin_mounts WHERE mount_id = ?")
            .bind(&params.mount_id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(updated)
    }

    async fn replace_credential_bindings(
        &self,
        params: &ReplacePluginCredentialBindingsParams,
    ) -> Result<PluginCredentialBindingSnapshot, DbError> {
        validate_uuid(&params.mount_id, "mount_id")?;
        validate_optional_digest(
            params.expected_current_artifact_digest.as_deref(),
            "expected_current_artifact_digest",
        )?;
        validate_timestamp(params.updated_at, "updated_at")?;
        let mut slots = std::collections::BTreeSet::new();
        for binding in &params.bindings {
            validate_binding_input(binding)?;
            if !slots.insert(binding.slot.as_str()) {
                return Err(conflict(format!(
                    "duplicate plugin credential slot {}",
                    binding.slot
                )));
            }
        }
        let mut tx = self.pool.begin().await?;
        let mount = lock_mount(&mut tx, &params.mount_id).await?;
        require_mount_runtime_cas(
            &mount,
            params.expected_mount_revision,
            params.expected_current_artifact_digest.as_deref(),
        )?;
        if mount.credential_bindings_revision != params.expected_bindings_revision {
            return Err(conflict("plugin credential bindings revision changed"));
        }
        if params.updated_at < mount.updated_at {
            return Err(conflict(
                "plugin credential binding timestamp predates the Mount state",
            ));
        }
        let next_revision = mount
            .credential_bindings_revision
            .checked_add(1)
            .ok_or_else(|| conflict("plugin credential bindings revision overflow"))?;
        sqlx::query(
            "INSERT INTO plugin_credential_binding_mutations (
                mount_id, expected_mount_revision, expected_current_artifact_digest,
                expected_bindings_revision, target_bindings_revision,
                allow_delete_pending, updated_at
             ) VALUES (?, ?, ?, ?, ?, 0, ?)",
        )
        .bind(&params.mount_id)
        .bind(params.expected_mount_revision)
        .bind(params.expected_current_artifact_digest.as_deref())
        .bind(params.expected_bindings_revision)
        .bind(next_revision)
        .bind(params.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        sqlx::query("DELETE FROM plugin_credential_bindings WHERE mount_id = ?")
            .bind(&params.mount_id)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?;
        for binding in &params.bindings {
            sqlx::query(
                "INSERT INTO plugin_credential_bindings (
                    mount_id, slot, credential_id, created_at, updated_at
                 ) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&params.mount_id)
            .bind(&binding.slot)
            .bind(&binding.credential_id)
            .bind(params.updated_at)
            .bind(params.updated_at)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?;
        }
        let changed = sqlx::query(
            "UPDATE plugin_mounts
             SET credential_bindings_revision = ?, updated_at = ?
             WHERE mount_id = ? AND revision = ?
               AND current_artifact_digest IS ?
               AND credential_bindings_revision = ?",
        )
        .bind(next_revision)
        .bind(params.updated_at)
        .bind(&params.mount_id)
        .bind(params.expected_mount_revision)
        .bind(params.expected_current_artifact_digest.as_deref())
        .bind(params.expected_bindings_revision)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?
        .rows_affected();
        if changed != 1 {
            return Err(conflict("plugin credential bindings exact CAS failed"));
        }
        let bindings = fetch_bindings(&mut tx, &params.mount_id).await?;
        tx.commit().await?;
        Ok(PluginCredentialBindingSnapshot {
            mount_id: mount.mount_id,
            mount_revision: mount.revision,
            current_artifact_digest: mount.current_artifact_digest,
            bindings_revision: next_revision,
            bindings,
        })
    }

    async fn list_credential_bindings(
        &self,
        params: &ListPluginCredentialBindingsParams,
    ) -> Result<PluginCredentialBindingSnapshot, DbError> {
        validate_uuid(&params.mount_id, "mount_id")?;
        validate_optional_digest(
            params.expected_current_artifact_digest.as_deref(),
            "expected_current_artifact_digest",
        )?;
        let mut tx = self.pool.begin().await?;
        let mount: PluginMountRow =
            sqlx::query_as("SELECT * FROM plugin_mounts WHERE mount_id = ?")
                .bind(&params.mount_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| DbError::NotFound(format!("plugin mount {}", params.mount_id)))?;
        require_mount_runtime_cas(
            &mount,
            params.expected_mount_revision,
            params.expected_current_artifact_digest.as_deref(),
        )?;
        let bindings = fetch_bindings(&mut tx, &params.mount_id).await?;
        tx.commit().await?;
        Ok(PluginCredentialBindingSnapshot {
            mount_id: mount.mount_id,
            mount_revision: mount.revision,
            current_artifact_digest: mount.current_artifact_digest,
            bindings_revision: mount.credential_bindings_revision,
            bindings,
        })
    }

    async fn put_kv_cas(&self, params: &PutPluginKvParams) -> Result<PluginKvRow, DbError> {
        validate_uuid(&params.mount_id, "mount_id")?;
        validate_timestamp(params.updated_at, "updated_at")?;
        let value_json = json_value(&params.value, "plugin KV value")?;
        let mut tx = self.pool.begin().await?;
        let mount = lock_mount(&mut tx, &params.mount_id).await?;
        require_mount_runtime_cas(
            &mount,
            params.expected_mount_revision,
            params.expected_current_artifact_digest.as_deref(),
        )?;
        if params.updated_at < mount.updated_at {
            return Err(conflict("plugin KV timestamp predates the Mount state"));
        }
        let changed = if let Some(expected_revision) = params.expected_revision {
            sqlx::query(
                "UPDATE plugin_kv
                 SET value_json = ?, revision = revision + 1, updated_at = ?
                 WHERE mount_id = ? AND namespace = ? AND key = ?
                   AND revision = ? AND updated_at <= ?",
            )
            .bind(&value_json)
            .bind(params.updated_at)
            .bind(&params.mount_id)
            .bind(&params.namespace)
            .bind(&params.key)
            .bind(expected_revision)
            .bind(params.updated_at)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?
            .rows_affected()
        } else {
            sqlx::query(
                "INSERT INTO plugin_kv (
                    mount_id, namespace, key, value_json, revision, created_at, updated_at
                 ) VALUES (?, ?, ?, ?, 1, ?, ?)
                 ON CONFLICT(mount_id, namespace, key) DO NOTHING",
            )
            .bind(&params.mount_id)
            .bind(&params.namespace)
            .bind(&params.key)
            .bind(&value_json)
            .bind(params.updated_at)
            .bind(params.updated_at)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?
            .rows_affected()
        };
        if changed != 1 {
            return Err(conflict("plugin KV revision CAS failed"));
        }
        let row = sqlx::query_as(
            "SELECT * FROM plugin_kv WHERE mount_id = ? AND namespace = ? AND key = ?",
        )
        .bind(&params.mount_id)
        .bind(&params.namespace)
        .bind(&params.key)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    async fn get_kv(&self, params: &GetPluginKvParams) -> Result<Option<PluginKvRow>, DbError> {
        validate_uuid(&params.mount_id, "mount_id")?;
        validate_optional_digest(
            params.expected_current_artifact_digest.as_deref(),
            "expected_current_artifact_digest",
        )?;
        let mut tx = self.pool.begin().await?;
        let mount: PluginMountRow =
            sqlx::query_as("SELECT * FROM plugin_mounts WHERE mount_id = ?")
                .bind(&params.mount_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| DbError::NotFound(format!("plugin mount {}", params.mount_id)))?;
        require_mount_runtime_cas(
            &mount,
            params.expected_mount_revision,
            params.expected_current_artifact_digest.as_deref(),
        )?;
        let value = sqlx::query_as(
            "SELECT * FROM plugin_kv WHERE mount_id = ? AND namespace = ? AND key = ?",
        )
        .bind(&params.mount_id)
        .bind(&params.namespace)
        .bind(&params.key)
        .fetch_optional(&mut *tx)
        .await
        .map_err(DbError::Query)?;
        tx.commit().await?;
        Ok(value)
    }

    async fn delete_kv_cas(&self, params: &DeletePluginKvParams) -> Result<bool, DbError> {
        validate_uuid(&params.mount_id, "mount_id")?;
        validate_optional_digest(
            params.expected_current_artifact_digest.as_deref(),
            "expected_current_artifact_digest",
        )?;
        validate_timestamp(params.updated_at, "updated_at")?;
        let mut tx = self.pool.begin().await?;
        let mount = lock_mount(&mut tx, &params.mount_id).await?;
        require_mount_runtime_cas(
            &mount,
            params.expected_mount_revision,
            params.expected_current_artifact_digest.as_deref(),
        )?;
        if params.updated_at < mount.updated_at {
            return Err(conflict("plugin KV timestamp predates the Mount state"));
        }
        let deleted = sqlx::query(
            "DELETE FROM plugin_kv
             WHERE mount_id = ? AND namespace = ? AND key = ?
               AND revision = ? AND updated_at <= ?",
        )
        .bind(&params.mount_id)
        .bind(&params.namespace)
        .bind(&params.key)
        .bind(params.expected_revision)
        .bind(params.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?
        .rows_affected();
        if deleted == 0 {
            let current: Option<i64> = sqlx::query_scalar(
                "SELECT revision FROM plugin_kv
                 WHERE mount_id = ? AND namespace = ? AND key = ?",
            )
            .bind(&params.mount_id)
            .bind(&params.namespace)
            .bind(&params.key)
            .fetch_optional(&mut *tx)
            .await?;
            if current.is_some() {
                return Err(conflict("plugin KV delete revision CAS failed"));
            }
        }
        tx.commit().await?;
        Ok(deleted == 1)
    }

    async fn get_project(&self, project_id: &str) -> Result<Option<PluginProjectRow>, DbError> {
        validate_uuid(project_id, "project_id")?;
        sqlx::query_as("SELECT * FROM plugin_projects WHERE project_id = ?")
            .bind(project_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn get_ready_candidate(
        &self,
        project_id: &str,
    ) -> Result<Option<PluginReadyCandidateRow>, DbError> {
        validate_uuid(project_id, "project_id")?;
        sqlx::query_as::<_, PluginReadyCandidateRow>(&format!(
            "{READY_CANDIDATE_SELECT}
             WHERE candidate.project_id = ?
               AND EXISTS (
                   SELECT 1
                   FROM plugin_projects project
                   WHERE project.project_id = candidate.project_id
                     AND project.ready_candidate_id = candidate.candidate_id
               )"
        ))
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn get_mount(&self, mount_id: &str) -> Result<Option<PluginMountRow>, DbError> {
        validate_uuid(mount_id, "mount_id")?;
        sqlx::query_as("SELECT * FROM plugin_mounts WHERE mount_id = ?")
            .bind(mount_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(DbError::Query)
    }

    async fn get_mount_runtime_state(
        &self,
        params: &ListPluginCredentialBindingsParams,
    ) -> Result<Option<PluginMountRuntimeState>, DbError> {
        validate_uuid(&params.mount_id, "mount_id")?;
        validate_optional_digest(
            params.expected_current_artifact_digest.as_deref(),
            "expected_current_artifact_digest",
        )?;
        let mut tx = self.pool.begin().await?;
        let Some(mount) =
            sqlx::query_as::<_, PluginMountRow>("SELECT * FROM plugin_mounts WHERE mount_id = ?")
                .bind(&params.mount_id)
                .fetch_optional(&mut *tx)
                .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        require_mount_runtime_cas(
            &mount,
            params.expected_mount_revision,
            params.expected_current_artifact_digest.as_deref(),
        )?;
        let credential_bindings = fetch_bindings(&mut tx, &params.mount_id).await?;
        tx.commit().await?;
        Ok(Some(PluginMountRuntimeState {
            mount,
            credential_bindings,
        }))
    }
}
