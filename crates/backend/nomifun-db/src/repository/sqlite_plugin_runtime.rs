use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::{
    ArtifactId, PluginReadyRelease, PluginServiceTestOutcome, PluginServiceTestReceipt,
    PLUGIN_RELEASE_PROFILE_VERSION, PLUGIN_SERVICE_HOST_PROTOCOL_VERSION,
    PLUGIN_SERVICE_SDK_CONTRACT_VERSION, PLUGIN_SERVICE_TEST_CONTRACT_VERSION,
    canonical_json_bytes, digest_payload,
};
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::DbError;
use crate::models::{
    PluginRuntimeBuildOperationLineageRow, PluginRuntimeCatalogPublicationRow,
    PluginRuntimeCredentialBindingRow, PluginRuntimeLibraryStateRow,
    PluginRuntimeLibrarySnapshot, PluginRuntimeProjectSourceState, PluginRuntimeSnapshot,
    PluginRuntimeKvRow, PluginRuntimeProductRow, PluginRuntimeProjectRow, PluginRuntimePublishAuthorizationRow,
    PluginRuntimeReleaseArtifactRow, PluginRuntimeReleaseRow, PluginRuntimeSourceMutationIntentRow,
    PluginRuntimeSurfaceSessionRow, ProductOperationRow, ProductOperationState,
};
use crate::repository::plugin_runtime::{
    AbortPluginSourceMutationParams, BeginPluginRuntimeDeleteParams,
    BeginPluginRuntimeImportAsNewParams, BeginPluginSourceMutationParams,
    BeginPluginRuntimeImportAsNewResult, CancelPluginRuntimeBuildOperationParams,
    CancelPluginRuntimeExportOperationParams, CancelPluginRuntimeImportParams,
    ClosePluginRuntimeSurfaceSessionParams, CommitPluginRuntimeLifecycleParams,
    CreatePluginRuntimeParams, CreatePluginRuntimeWithSourceParams,
    ExecutePluginRuntimeSurfaceKvParams, FailPluginRuntimeDeleteParams,
    FailPluginRuntimeExportOperationParams, FailPluginRuntimeImportParams,
    FinalizePluginRuntimeDeleteParams, FinalizePluginSourceMutationParams,
    FinishPluginRuntimeBuildAndRecordReadyParams,
    FinishPluginRuntimeBuildOperationParams, FinishPluginRuntimeExportOperationParams,
    FinishPluginRuntimeImportReadyParams, IPluginRuntimeRepository,
    FinishPluginRuntimeBackupImportParams, PluginRuntimeBackupExportSnapshot,
    PluginRuntimeBackupReleaseSlot,
    StartPluginRuntimeBackupExportParams,
    PluginRuntimeAutoPublishGuard, PluginRuntimeImportSource, PluginRuntimeManagedSourceLineage,
    PluginRuntimeSurfaceKvOperation, PluginRuntimeSurfaceKvResult,
    PluginRuntimeServiceTestReceiptRow, OpenPluginRuntimeSurfaceSessionParams,
    PublishPluginRuntimeReadyParams, RecordPluginRuntimeReadyReleaseParams,
    RecordPluginRuntimeServiceTestReceiptParams, ResolvePluginRuntimeSurfaceSessionParams,
    RestartPluginRuntimeDeleteParams, RestorePluginRuntimeParams,
    RollbackPluginRuntimePreviousParams, SetPluginRuntimeAutoPublishParams,
    StartPluginRuntimeBuildOperationParams, StartPluginRuntimeExportOperationParams,
    TrashPluginRuntimeParams, UpdatePluginRuntimeProjectSourceParams, conflict,
    normalize_incoming_artifact, query_error, serialize_product_operation_log_tail,
    validate_artifact, validate_digest, validate_json_object, validate_kv_row,
    validate_managed_source_lineage, validate_optional_digest,
    validate_product_artifact_contract, validate_product_operation_error_code,
    validate_project_source, validate_release, validate_uuid, validate_visible_ascii_key,
};

#[derive(Clone, Debug)]
pub struct SqlitePluginRuntimeRepository {
    pool: SqlitePool,
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct PluginDeletionIntentQueryRow {
    #[sqlx(rename = "id")]
    _id: i64,
    plugin_product_id: String,
    owner_user_id: String,
    operation_id: String,
    started_at_ms: i64,
    last_error_code: Option<String>,
}

impl SqlitePluginRuntimeRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn create_inner(
        &self,
        params: &CreatePluginRuntimeParams,
        source: Option<&PluginRuntimeManagedSourceLineage>,
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&params.project_id, "project_id")?;
        if params.expected_library_revision < 0 || params.created_at < 0 {
            return Err(conflict("Plugin create CAS/timestamp is invalid"));
        }
        if params.display_name.trim().is_empty()
            || params.display_name.chars().count() > 255
        {
            return Err(conflict(
                "Plugin display_name must contain 1 to 255 characters",
            ));
        }
        validate_json_object(&params.config_schema_json, "config_schema_json")?;
        validate_json_object(&params.config_json, "config_json")?;
        validate_digest(
            &params.materialized_catalog_digest,
            "materialized_catalog_digest",
        )?;
        if let Some(source) = source {
            validate_managed_source_lineage(source)?;
            if source.build_profile_version != PLUGIN_RELEASE_PROFILE_VERSION {
                return Err(conflict(
                    "Plugin managed source uses an unsupported release profile version",
                ));
            }
        }
        ensure_owner(&self.pool, &params.owner_user_id).await?;
        let mut tx = self.pool.begin().await?;
        let current = sqlx::query_as::<_, PluginRuntimeLibraryStateRow>(
            "SELECT * FROM plugin_library_state
             WHERE owner_user_id = ?",
        )
        .bind(&params.owner_user_id)
        .fetch_optional(&mut *tx)
        .await?;
        let current_revision = current.as_ref().map_or(0, |row| row.revision);
        if current_revision != params.expected_library_revision {
            return Err(conflict(format!(
                "Plugin library revision changed from expected {} to {}",
                params.expected_library_revision, current_revision
            )));
        }
        let next_library_revision = current_revision + 1;
        if current.is_none() {
            sqlx::query(
                "INSERT INTO plugin_library_state
                 (singleton_key, owner_user_id, revision, updated_at)
                 VALUES ('plugin_runtime', ?, ?, ?)",
            )
            .bind(&params.owner_user_id)
            .bind(next_library_revision)
            .bind(params.created_at)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?;
        } else {
            let changed = sqlx::query(
                "UPDATE plugin_library_state SET revision = ?, updated_at = ?
                 WHERE owner_user_id = ? AND revision = ? AND updated_at <= ?",
            )
            .bind(next_library_revision)
            .bind(params.created_at)
            .bind(&params.owner_user_id)
            .bind(current_revision)
            .bind(params.created_at)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?;
            if changed.rows_affected() != 1 {
                return Err(conflict(
                    "Plugin library create CAS or timestamp check failed",
                ));
            }
        }
        sqlx::query(
            "INSERT INTO plugin_products
             (plugin_product_id, owner_user_id, display_name, description,
              icon_asset_id, kind, materialized_catalog_digest,
              config_schema_json, config_json, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&params.plugin_product_id)
        .bind(&params.owner_user_id)
        .bind(&params.display_name)
        .bind(&params.description)
        .bind(&params.icon_asset_id)
        .bind(params.kind.as_str())
        .bind(&params.materialized_catalog_digest)
        .bind(&params.config_schema_json)
        .bind(&params.config_json)
        .bind(params.created_at)
        .bind(params.created_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let source_state = source.map_or(
            (
                "empty",
                None,
                None,
                None,
                None,
                0_i64,
            ),
            |source| {
                (
                    "editable",
                    Some(source.managed_source_path.as_str()),
                    Some(source.source_head_digest.as_str()),
                    Some(source.dependency_lock_digest.as_str()),
                    Some(source.build_profile_version.as_str()),
                    source.build_generation,
                )
            },
        );
        sqlx::query(
            "INSERT INTO plugin_projects
             (project_id, plugin_product_id, owner_user_id, source_state,
              managed_source_path, source_head_digest, dependency_lock_digest,
              build_profile_version, build_generation, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&params.project_id)
        .bind(&params.plugin_product_id)
        .bind(&params.owner_user_id)
        .bind(source_state.0)
        .bind(source_state.1)
        .bind(source_state.2)
        .bind(source_state.3)
        .bind(source_state.4)
        .bind(source_state.5)
        .bind(params.created_at)
        .bind(params.created_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let snapshot = fetch_snapshot_in_tx(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
        )
            .await?
            .ok_or_else(|| DbError::Init("Plugin create lost its Product".into()))?;
        tx.commit().await?;
        Ok(snapshot)
    }
}

async fn ensure_owner(pool: &SqlitePool, owner_user_id: &str) -> Result<(), DbError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users WHERE user_id = ?",
    )
    .bind(owner_user_id)
    .fetch_one(pool)
    .await?;
    if count != 1 {
        return Err(DbError::NotFound(format!(
            "Plugin owner {owner_user_id}"
        )));
    }
    Ok(())
}

async fn fetch_snapshot(
    pool: &SqlitePool,
    owner_user_id: &str,
    plugin_product_id: &str,
) -> Result<Option<PluginRuntimeSnapshot>, DbError> {
    let mut tx = pool.begin().await?;
    let snapshot = fetch_snapshot_in_tx(&mut tx, owner_user_id, plugin_product_id).await?;
    tx.commit().await?;
    Ok(snapshot)
}

async fn fetch_snapshot_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    owner_user_id: &str,
    plugin_product_id: &str,
) -> Result<Option<PluginRuntimeSnapshot>, DbError> {
    let Some(product) = sqlx::query_as::<_, PluginRuntimeProductRow>(
        "SELECT * FROM plugin_products
         WHERE owner_user_id = ? AND plugin_product_id = ?",
    )
    .bind(owner_user_id)
    .bind(plugin_product_id)
    .fetch_optional(&mut **tx)
    .await?
    else {
        return Ok(None);
    };
    validate_deletion_state_in_tx(tx, &product).await?;
    let project = sqlx::query_as::<_, PluginRuntimeProjectRow>(
        "SELECT * FROM plugin_projects
         WHERE owner_user_id = ? AND plugin_product_id = ?",
    )
    .bind(owner_user_id)
    .bind(plugin_product_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| {
        DbError::Init(format!(
            "Plugin {plugin_product_id} has no exact owner-scoped Project"
        ))
    })?;
    let mut releases = BTreeMap::new();
    for release_id in [
        product.ready_release_id.as_deref(),
        product.active_release_id.as_deref(),
        product.previous_release_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if releases.contains_key(release_id) {
            continue;
        }
        let release = sqlx::query_as::<_, PluginRuntimeReleaseRow>(
            "SELECT * FROM plugin_releases
             WHERE owner_user_id = ? AND release_id = ?",
        )
        .bind(owner_user_id)
        .bind(release_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| {
            DbError::Init(format!(
                "Plugin pointer references missing Release {release_id}"
            ))
        })?;
        let artifact = sqlx::query_as::<_, PluginRuntimeReleaseArtifactRow>(
            "SELECT * FROM plugin_release_artifacts
             WHERE owner_user_id = ? AND artifact_id = ?",
        )
        .bind(owner_user_id)
        .bind(&release.artifact_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| {
            DbError::Init(format!(
                "Plugin Release {} references missing Artifact {}",
                release.release_id, release.artifact_id
            ))
        })?;
        let artifact_payload = validate_artifact(&artifact).map_err(|error| {
            DbError::Init(format!(
                "Plugin Artifact {} failed persisted row validation: {error}",
                artifact.artifact_id
            ))
        })?;
        validate_product_artifact_contract(&product.kind, &artifact_payload).map_err(|error| {
            DbError::Init(format!(
                "Plugin {} Release {} failed product/artifact kind validation: {error}",
                product.kind, release_id
            ))
        })?;
        validate_release(&release, &artifact_payload).map_err(|error| {
            DbError::Init(format!(
                "Plugin Release {release_id} failed persisted row validation: {error}"
            ))
        })?;
        validate_persisted_release_lineage(tx, &release).await?;
        releases.insert(release_id.to_owned(), release);
    }
    for (release_id, release_digest, label) in [
        (
            product.ready_release_id.as_deref(),
            product.ready_release_digest.as_deref(),
            "Ready",
        ),
        (
            product.active_release_id.as_deref(),
            product.active_release_digest.as_deref(),
            "Active",
        ),
        (
            product.previous_release_id.as_deref(),
            product.previous_release_digest.as_deref(),
            "Previous",
        ),
    ] {
        if let (Some(release_id), Some(release_digest)) = (release_id, release_digest) {
            let release = releases.get(release_id).ok_or_else(|| {
                DbError::Init(format!("Plugin {label} pointer lost Release {release_id}"))
            })?;
            if release.plugin_product_id != plugin_product_id
                || release.owner_user_id != owner_user_id
                || release.release_digest != release_digest
            {
                return Err(DbError::Init(format!(
                    "Plugin {label} pointer does not bind its exact owner Release"
                )));
            }
        }
    }
    let auto_publish_authorization =
        sqlx::query_as::<_, PluginRuntimePublishAuthorizationRow>(
            "SELECT * FROM plugin_publish_authorizations
             WHERE owner_user_id = ? AND plugin_product_id = ?",
        )
        .bind(owner_user_id)
        .bind(plugin_product_id)
        .fetch_optional(&mut **tx)
        .await?;
    let catalog_publication =
        sqlx::query_as::<_, PluginRuntimeCatalogPublicationRow>(
            "SELECT * FROM plugin_catalog_publications
             WHERE owner_user_id = ? AND plugin_product_id = ?",
        )
        .bind(owner_user_id)
        .bind(plugin_product_id)
        .fetch_optional(&mut **tx)
        .await?;
    match (product.lifecycle.as_str(), catalog_publication.as_ref()) {
        ("enabled", Some(catalog))
            if product.active_release_id.as_deref() == Some(&catalog.active_release_id)
                && product.active_release_digest.as_deref()
                    == Some(&catalog.active_release_digest)
                && product.active_release_epoch == catalog.active_release_epoch
                && product.materialized_catalog_digest == catalog.catalog_digest => {}
        ("enabled", _) => {
            return Err(DbError::Init(
                "enabled Plugin has no exact Active Catalog publication".to_owned(),
            ));
        }
        (_, Some(_)) => {
            return Err(DbError::Init(
                "disabled Plugin retains a Catalog publication".to_owned(),
            ));
        }
        (_, None) => {}
    }
    let credentials = sqlx::query_as::<_, PluginRuntimeCredentialBindingRow>(
        "SELECT * FROM plugin_credential_bindings
         WHERE owner_user_id = ? AND plugin_product_id = ? ORDER BY slot_key",
    )
    .bind(owner_user_id)
    .bind(plugin_product_id)
    .fetch_all(&mut **tx)
    .await?;
    let library_revision = sqlx::query_scalar::<_, i64>(
        "SELECT revision FROM plugin_library_state
         WHERE owner_user_id = ?",
    )
    .bind(owner_user_id)
    .fetch_optional(&mut **tx)
    .await?
    .unwrap_or(0);
    Ok(Some(PluginRuntimeSnapshot {
        library_revision,
        ready_release: product
            .ready_release_id
            .as_deref()
            .and_then(|id| releases.get(id).cloned()),
        active_release: product
            .active_release_id
            .as_deref()
            .and_then(|id| releases.get(id).cloned()),
        previous_release: product
            .previous_release_id
            .as_deref()
            .and_then(|id| releases.get(id).cloned()),
        auto_publish_authorization,
        catalog_publication,
        product,
        project,
        credential_bindings: credentials,
    }))
}

async fn validate_deletion_state_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    product: &PluginRuntimeProductRow,
) -> Result<(), DbError> {
    let intent = sqlx::query_as::<_, PluginDeletionIntentQueryRow>(
        "SELECT * FROM plugin_deletion_intents
         WHERE owner_user_id = ? AND plugin_product_id = ?",
    )
    .bind(&product.owner_user_id)
    .bind(&product.plugin_product_id)
    .fetch_optional(&mut **tx)
    .await?;

    let Some(intent) = intent else {
        if product.lifecycle == "deleting" {
            return Err(DbError::Init(format!(
                "Plugin {} is deleting without an exact deletion intent",
                product.plugin_product_id
            )));
        }
        return Ok(());
    };
    if product.lifecycle != "deleting"
        || intent.plugin_product_id != product.plugin_product_id
        || intent.owner_user_id != product.owner_user_id
    {
        return Err(DbError::Init(format!(
            "Plugin deletion intent exists outside the deleting lifecycle for {}",
            product.plugin_product_id
        )));
    }
    validate_uuid(&intent.operation_id, "deletion.operation_id")?;
    validate_uuid(&intent.plugin_product_id, "deletion.plugin_product_id")?;
    validate_uuid(&intent.owner_user_id, "deletion.owner_user_id")?;
    validate_product_operation_error_code(intent.last_error_code.as_deref())?;

    let operation = sqlx::query_as::<_, ProductOperationRow>(
        "SELECT * FROM product_operations
         WHERE operation_id = ? AND owner_kind = 'plugin' AND owner_id = ?",
    )
    .bind(&intent.operation_id)
    .bind(&product.plugin_product_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| {
        DbError::Init(format!(
            "Plugin deletion intent references missing operation {}",
            intent.operation_id
        ))
    })?;
    if operation.owner_id != intent.plugin_product_id
        || operation.kind != "plugin_permanent_delete"
        || operation.started_at_ms != intent.started_at_ms
        || operation.progress_percent.is_some()
        || !matches!(operation.state.as_str(), "running" | "failed")
        || operation.last_error_code != intent.last_error_code
    {
        return Err(DbError::Init(format!(
            "Plugin deletion intent and operation are not an exact pair for {}",
            product.plugin_product_id
        )));
    }
    Ok(())
}

async fn lock_product(
    tx: &mut Transaction<'_, Sqlite>,
    owner_user_id: &str,
    plugin_product_id: &str,
) -> Result<PluginRuntimeProductRow, DbError> {
    sqlx::query_as::<_, PluginRuntimeProductRow>(
        "SELECT * FROM plugin_products
         WHERE owner_user_id = ? AND plugin_product_id = ?",
    )
    .bind(owner_user_id)
    .bind(plugin_product_id)
    .fetch_optional(&mut **tx)
    .await?
        .ok_or_else(|| DbError::NotFound(format!("Plugin {plugin_product_id}")))
}

async fn fetch_owned_product_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    owner_user_id: &str,
    plugin_product_id: &str,
) -> Result<Option<PluginRuntimeProductRow>, DbError> {
    sqlx::query_as::<_, PluginRuntimeProductRow>(
        "SELECT * FROM plugin_products
         WHERE owner_user_id = ? AND plugin_product_id = ?",
    )
    .bind(owner_user_id)
    .bind(plugin_product_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(DbError::Query)
}

async fn lock_product_for_update(
    tx: &mut Transaction<'_, Sqlite>,
    owner_user_id: &str,
    plugin_product_id: &str,
) -> Result<PluginRuntimeProductRow, DbError> {
    let locked = sqlx::query(
        "UPDATE plugin_products
         SET updated_at = updated_at
         WHERE owner_user_id = ? AND plugin_product_id = ?",
    )
    .bind(owner_user_id)
    .bind(plugin_product_id)
    .execute(&mut **tx)
    .await
    .map_err(query_error)?;
    if locked.rows_affected() != 1 {
        return Err(DbError::NotFound(format!("Plugin {plugin_product_id}")));
    }
    lock_product(tx, owner_user_id, plugin_product_id).await
}

async fn fetch_project(
    tx: &mut Transaction<'_, Sqlite>,
    owner_user_id: &str,
    plugin_product_id: &str,
    project_id: &str,
) -> Result<PluginRuntimeProjectRow, DbError> {
    sqlx::query_as::<_, PluginRuntimeProjectRow>(
        "SELECT * FROM plugin_projects
         WHERE owner_user_id = ? AND plugin_product_id = ? AND project_id = ?",
    )
    .bind(owner_user_id)
    .bind(plugin_product_id)
    .bind(project_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| DbError::NotFound(format!("Plugin Project {project_id}")))
}

async fn fetch_build_operation(
    tx: &mut Transaction<'_, Sqlite>,
    owner_user_id: &str,
    plugin_product_id: &str,
    operation_id: &str,
) -> Result<Option<ProductOperationRow>, DbError> {
    sqlx::query_as::<_, ProductOperationRow>(
        "SELECT operation.*
         FROM product_operations operation
         JOIN plugin_products product
           ON product.plugin_product_id = operation.owner_id
          AND product.owner_user_id = ?
         WHERE operation.operation_id = ?
           AND operation.kind = 'build'
           AND operation.owner_kind = 'plugin'
           AND operation.owner_id = ?
           AND EXISTS (
               SELECT 1
               FROM plugin_build_operation_lineage lineage
               WHERE lineage.operation_id = operation.operation_id
                 AND lineage.owner_user_id = ?
                 AND lineage.plugin_product_id = operation.owner_id
           )",
    )
    .bind(owner_user_id)
    .bind(operation_id)
    .bind(plugin_product_id)
    .bind(owner_user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(DbError::Query)
}

async fn fetch_build_lineage(
    tx: &mut Transaction<'_, Sqlite>,
    owner_user_id: &str,
    plugin_product_id: &str,
    operation_id: &str,
) -> Result<Option<PluginRuntimeBuildOperationLineageRow>, DbError> {
    sqlx::query_as::<_, PluginRuntimeBuildOperationLineageRow>(
        "SELECT lineage.*
         FROM plugin_build_operation_lineage lineage
         JOIN plugin_products product
           ON product.plugin_product_id = lineage.plugin_product_id
          AND product.owner_user_id = lineage.owner_user_id
         WHERE lineage.owner_user_id = ?
           AND lineage.plugin_product_id = ?
           AND lineage.operation_id = ?",
    )
    .bind(owner_user_id)
    .bind(plugin_product_id)
    .bind(operation_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(DbError::Query)
}

async fn validate_persisted_release_lineage(
    tx: &mut Transaction<'_, Sqlite>,
    release: &PluginRuntimeReleaseRow,
) -> Result<(), DbError> {
    if release.origin_kind == "import" {
        let operation =
            fetch_plugin_operation_in_tx(tx, &release.plugin_product_id, &release.origin_operation_id)
                .await?
                .ok_or_else(|| {
                    DbError::Init(format!(
                        "imported Plugin Release {} lost its Import operation",
                        release.release_id
                    ))
                })?;
        if operation.kind != "import"
            || operation.state != ProductOperationState::Succeeded.as_str()
            || release.created_at < operation.started_at_ms
            || operation
                .finished_at_ms
                .is_none_or(|finished_at_ms| release.created_at > finished_at_ms)
        {
            return Err(DbError::Init(format!(
                "imported Plugin Release {} does not match its successful Import operation",
                release.release_id
            )));
        }
        return Ok(());
    }
    if release.origin_kind != "build" {
        return Ok(());
    }
    let operation = fetch_build_operation(
        tx,
        &release.owner_user_id,
        &release.plugin_product_id,
        &release.origin_operation_id,
    )
    .await?
    .ok_or_else(|| {
        DbError::Init(format!(
            "built Plugin Release {} lost its Build operation",
            release.release_id
        ))
    })?;
    if operation.state != ProductOperationState::Succeeded.as_str() {
        return Err(DbError::Init(format!(
            "built Plugin Release {} references a non-successful Build",
            release.release_id
        )));
    }
    let lineage = fetch_build_lineage(
        tx,
        &release.owner_user_id,
        &release.plugin_product_id,
        &release.origin_operation_id,
    )
    .await?
    .ok_or_else(|| {
        DbError::Init(format!(
            "built Plugin Release {} lost its source lineage",
            release.release_id
        ))
    })?;
    if release.project_id.as_deref() != Some(lineage.project_id.as_str())
        || release.source_snapshot_digest.as_deref()
            != Some(lineage.source_snapshot_digest.as_str())
        || release.dependency_lock_digest.as_deref()
            != Some(lineage.dependency_lock_digest.as_str())
        || release.build_profile_version.as_deref()
            != Some(lineage.build_profile_version.as_str())
        || release.build_generation != Some(lineage.build_generation)
        || release.created_at < lineage.started_at_ms
    {
        return Err(DbError::Init(format!(
            "built Plugin Release {} does not match its immutable source lineage",
            release.release_id
        )));
    }
    Ok(())
}

fn validate_build_source(source: &PluginRuntimeManagedSourceLineage) -> Result<(), DbError> {
    validate_managed_source_lineage(source)?;
    if source.build_profile_version != PLUGIN_RELEASE_PROFILE_VERSION {
        return Err(conflict(
            "Plugin Build requires the canonical release profile version",
        ));
    }
    Ok(())
}

fn project_matches_source(
    project: &PluginRuntimeProjectRow,
    expected_project_revision: i64,
    source: &PluginRuntimeManagedSourceLineage,
) -> bool {
    project.project_revision == expected_project_revision
        && project.source_state == "editable"
        && project.managed_source_path.as_deref() == Some(source.managed_source_path.as_str())
        && project.source_head_digest.as_deref() == Some(source.source_head_digest.as_str())
        && project.dependency_lock_digest.as_deref()
            == Some(source.dependency_lock_digest.as_str())
        && project.build_profile_version.as_deref() == Some(source.build_profile_version.as_str())
        && project.build_generation == source.build_generation
}

fn validate_auto_publish_guard(guard: &PluginRuntimeAutoPublishGuard) -> Result<(), DbError> {
    validate_uuid(&guard.authorization_id, "auto_publish.authorization_id")?;
    validate_uuid(&guard.project_id, "auto_publish.project_id")?;
    validate_digest(
        &guard.source_head_digest,
        "auto_publish.source_head_digest",
    )?;
    validate_digest(
        &guard.dependency_lock_digest,
        "auto_publish.dependency_lock_digest",
    )?;
    if guard.authorization_revision < 1
        || guard.project_revision < 1
        || guard.build_generation < 1
        || guard.build_profile_version != PLUGIN_RELEASE_PROFILE_VERSION
    {
        return Err(conflict("Plugin auto Publish guard is invalid"));
    }
    Ok(())
}

fn validate_build_finish_shape(
    state: ProductOperationState,
    progress_percent: u8,
    last_error_code: Option<&str>,
) -> Result<(), DbError> {
    if state != ProductOperationState::Failed {
        return Err(conflict(
            "finish_build_operation only records failed Builds; successful Builds must commit Ready",
        ));
    }
    validate_product_operation_error_code(last_error_code)?;
    if last_error_code.is_none() {
        return Err(conflict(
            "failed Plugin Build operations require last_error_code",
        ));
    }
    if progress_percent > 100 {
        return Err(conflict("Plugin Build progress must be between 0 and 100"));
    }
    Ok(())
}

fn validate_ready_binding(
    params: &FinishPluginRuntimeBuildAndRecordReadyParams,
) -> Result<(), DbError> {
    validate_uuid(&params.owner_user_id, "owner_user_id")?;
    validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
    validate_uuid(&params.project_id, "project_id")?;
    validate_uuid(&params.operation_id, "operation_id")?;
    if params.expected_product_revision < 1
        || params.expected_pointer_revision < 1
        || params.expected_project_revision < 1
        || params.expected_build_generation < 1
        || params.finished_at_ms <= 0
    {
        return Err(conflict(
            "Plugin Build Ready CAS expectations are invalid",
        ));
    }
    let artifact_payload = validate_artifact(&params.artifact)?;
    validate_release(&params.release, &artifact_payload)?;
    if params.artifact.owner_user_id != params.owner_user_id
        || params.release.owner_user_id != params.owner_user_id
        || params.release.plugin_product_id != params.plugin_product_id
        || params.release.project_id.as_deref() != Some(params.project_id.as_str())
        || params.release.origin_kind != "build"
        || params.release.origin_operation_id != params.operation_id
        || params.release.artifact_id != params.artifact.artifact_id
        || params.release.artifact_digest != params.artifact.artifact_digest
        || params.release.manifest_digest != params.artifact.manifest_digest
        || params.release.release_digest != params.artifact.artifact_digest
        || params.release.build_generation != Some(params.expected_build_generation)
    {
        return Err(conflict(
            "Plugin Ready Release does not bind its exact Build operation and artifact",
        ));
    }
    if params.artifact.created_at > params.finished_at_ms
        || params.release.created_at > params.finished_at_ms
    {
        return Err(conflict(
            "Plugin Ready artifact/release timestamps exceed operation completion",
        ));
    }
    serialize_product_operation_log_tail(&params.bounded_log_tail)?;
    Ok(())
}

async fn bump_library_revision(
    tx: &mut Transaction<'_, Sqlite>,
    owner_user_id: &str,
    updated_at: i64,
) -> Result<(), DbError> {
    let changed = sqlx::query(
        "UPDATE plugin_library_state
         SET revision = revision + 1, updated_at = ?
         WHERE owner_user_id = ? AND updated_at <= ?",
    )
    .bind(updated_at)
    .bind(owner_user_id)
    .bind(updated_at)
    .execute(&mut **tx)
    .await
    .map_err(query_error)?;
    if changed.rows_affected() != 1 {
        return Err(DbError::Init(format!(
            "Plugin owner {owner_user_id} has no library state"
        )));
    }
    Ok(())
}

async fn require_pointer_release(
    tx: &mut Transaction<'_, Sqlite>,
    owner_user_id: &str,
    plugin_product_id: &str,
    release_id: Option<&str>,
    release_digest: Option<&str>,
    label: &str,
) -> Result<(), DbError> {
    let (Some(release_id), Some(release_digest)) =
        (release_id, release_digest)
    else {
        return Ok(());
    };
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM plugin_releases
         WHERE owner_user_id = ? AND plugin_product_id = ?
           AND release_id = ? AND release_digest = ?",
    )
    .bind(owner_user_id)
    .bind(plugin_product_id)
    .bind(release_id)
    .bind(release_digest)
    .fetch_one(&mut **tx)
    .await?;
    if count != 1 {
        return Err(DbError::Conflict(format!(
            "{label} does not bind an exact Release owned by this Plugin"
        )));
    }
    Ok(())
}

async fn ensure_no_running_build(
    tx: &mut Transaction<'_, Sqlite>,
    plugin_product_id: &str,
) -> Result<(), DbError> {
    let running: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM product_operations
         WHERE owner_kind = 'plugin' AND owner_id = ?
           AND kind = 'build' AND state = 'running'",
    )
    .bind(plugin_product_id)
    .fetch_one(&mut **tx)
    .await?;
    if running != 0 {
        return Err(conflict(
            "Plugin Release/lifecycle mutation is blocked by a running Build",
        ));
    }
    Ok(())
}

fn validate_lifecycle_identity(
    owner_user_id: &str,
    plugin_product_id: &str,
    expected_product_revision: i64,
    expected_pointer_revision: i64,
    updated_at: i64,
) -> Result<(), DbError> {
    validate_uuid(owner_user_id, "owner_user_id")?;
    validate_uuid(plugin_product_id, "plugin_product_id")?;
    if expected_product_revision < 1 || expected_pointer_revision < 1 || updated_at <= 0 {
        return Err(conflict(
            "Plugin lifecycle CAS expectations are invalid",
        ));
    }
    Ok(())
}

async fn ensure_no_running_plugin_operation(
    tx: &mut Transaction<'_, Sqlite>,
    plugin_product_id: &str,
) -> Result<(), DbError> {
    let running: Option<String> = sqlx::query_scalar(
        "SELECT operation_id FROM product_operations
         WHERE owner_kind = 'plugin' AND owner_id = ? AND state = 'running'
         ORDER BY started_at_ms ASC, operation_id ASC LIMIT 1",
    )
    .bind(plugin_product_id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(operation_id) = running {
        return Err(conflict(format!(
            "Plugin lifecycle mutation is blocked by running operation {operation_id}"
        )));
    }
    Ok(())
}

async fn fetch_deletion_intent_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    owner_user_id: &str,
    plugin_product_id: &str,
    operation_id: &str,
) -> Result<PluginDeletionIntentQueryRow, DbError> {
    sqlx::query_as::<_, PluginDeletionIntentQueryRow>(
        "SELECT * FROM plugin_deletion_intents
         WHERE owner_user_id = ? AND plugin_product_id = ? AND operation_id = ?",
    )
    .bind(owner_user_id)
    .bind(plugin_product_id)
    .bind(operation_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| DbError::NotFound(format!("Plugin deletion intent {operation_id}")))
}

async fn fetch_plugin_operation_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    plugin_product_id: &str,
    operation_id: &str,
) -> Result<Option<ProductOperationRow>, DbError> {
    sqlx::query_as::<_, ProductOperationRow>(
        "SELECT * FROM product_operations
         WHERE operation_id = ? AND owner_kind = 'plugin' AND owner_id = ?",
    )
    .bind(operation_id)
    .bind(plugin_product_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(DbError::Query)
}

async fn revoke_catalog_publication(
    tx: &mut Transaction<'_, Sqlite>,
    owner_user_id: &str,
    plugin_product_id: &str,
) -> Result<(), DbError> {
    sqlx::query(
        "DELETE FROM plugin_catalog_publications
         WHERE owner_user_id = ? AND plugin_product_id = ?",
    )
    .bind(owner_user_id)
    .bind(plugin_product_id)
    .execute(&mut **tx)
    .await
    .map_err(query_error)?;
    Ok(())
}

fn rewrite_release_record_artifact_id(
    value: &str,
    artifact_id: &str,
) -> Result<String, DbError> {
    let mut record: PluginReadyRelease = serde_json::from_str(value)
        .map_err(|error| conflict(format!("release_record_json is invalid: {error}")))?;
    record.release.artifact_id = ArtifactId::from(artifact_id);
    let bytes = canonical_json_bytes(&record)
        .map_err(|error| conflict(format!("release_record_json cannot be serialized: {error}")))?;
    String::from_utf8(bytes)
        .map_err(|error| conflict(format!("release_record_json is not UTF-8: {error}")))
}

fn service_test_outcome(value: PluginServiceTestOutcome) -> &'static str {
    match value {
        PluginServiceTestOutcome::Passed => "passed",
        PluginServiceTestOutcome::Failed => "failed",
        PluginServiceTestOutcome::NeedsTestInput => "needs_test_input",
    }
}

fn canonical_service_test_receipt(
    params: &RecordPluginRuntimeServiceTestReceiptParams,
) -> Result<(String, PluginServiceTestReceipt), DbError> {
    let receipt: PluginServiceTestReceipt = serde_json::from_value(params.receipt.clone())
        .map_err(|error| conflict(format!("Service Test receipt JSON is invalid: {error}")))?;
    let receipt_error_code = receipt.error_code.as_ref().map(AsRef::as_ref);
    if receipt.receipt_id.as_ref() != params.receipt_id
        || receipt.plugin_product_id.as_ref() != params.plugin_product_id
        || receipt.release.release_id.as_ref() != params.expected_ready_release_id
        || receipt.release.release_digest.as_ref() != params.expected_ready_release_digest
        || receipt.service_run_key.as_ref() != params.service_run_key
        || receipt.outcome != params.outcome
        || receipt_error_code != params.error_code.as_deref()
        || receipt.resolved_test_input_digest.as_ref() != params.resolved_test_input_digest
        || receipt.issued_at_ms != params.issued_at_ms
    {
        return Err(conflict(
            "Service Test receipt JSON does not match its exact record fields",
        ));
    }
    match (receipt.outcome, receipt_error_code) {
        (PluginServiceTestOutcome::Failed, Some(_))
        | (PluginServiceTestOutcome::Passed, None)
        | (PluginServiceTestOutcome::NeedsTestInput, None) => {}
        (PluginServiceTestOutcome::Failed, None) => {
            return Err(conflict(
                "failed Service Test receipt requires an error_code",
            ));
        }
        (_, Some(_)) => {
            return Err(conflict(
                "non-failed Service Test receipt cannot carry an error_code",
            ));
        }
    }
    if receipt.host_target != receipt.runtime.runtime_target
        || receipt.host_protocol_version.as_ref() != PLUGIN_SERVICE_HOST_PROTOCOL_VERSION
        || receipt.sdk_contract_version.as_ref() != PLUGIN_SERVICE_SDK_CONTRACT_VERSION
        || receipt.test_contract_version.as_ref() != PLUGIN_SERVICE_TEST_CONTRACT_VERSION
        || receipt.host_generation == 0
        || receipt.issued_at_ms <= 0
        || receipt.empty_files_dir == Some(false)
    {
        return Err(conflict(
            "Service Test receipt Host identity or contract version is invalid",
        ));
    }
    for (value, label) in [
        (receipt.release.release_digest.as_ref(), "receipt.release_digest"),
        (receipt.service_run_key.as_ref(), "receipt.service_run_key"),
        (
            receipt.runtime.runtime_executable_digest.as_ref(),
            "receipt.runtime.runtime_executable_digest",
        ),
        (
            receipt.resolved_test_input_digest.as_ref(),
            "receipt.resolved_test_input_digest",
        ),
        (receipt.copied_kv_digest.as_ref(), "receipt.copied_kv_digest"),
    ] {
        validate_digest(value, label)?;
    }
    validate_optional_digest(
        receipt
            .copied_private_database_digest
            .as_ref()
            .map(AsRef::as_ref),
        "receipt.copied_private_database_digest",
    )?;
    validate_optional_digest(
        receipt
            .migration_ledger_digest
            .as_ref()
            .map(AsRef::as_ref),
        "receipt.migration_ledger_digest",
    )?;
    let computed_receipt_digest = digest_payload(&receipt)
        .map_err(|error| conflict(format!("Service Test receipt cannot be digested: {error}")))?;
    if computed_receipt_digest.as_ref() != params.receipt_digest {
        return Err(conflict("Service Test receipt_digest does not match receipt JSON"));
    }
    let computed_runtime_digest = digest_payload(&receipt.runtime)
        .map_err(|error| conflict(format!("Service Test runtime cannot be digested: {error}")))?;
    if computed_runtime_digest.as_ref() != params.runtime_fingerprint_digest {
        return Err(conflict(
            "Service Test runtime_fingerprint_digest does not match receipt JSON",
        ));
    }
    let canonical = String::from_utf8(
        canonical_json_bytes(&receipt)
            .map_err(|error| conflict(format!("Service Test receipt cannot be canonicalized: {error}")))?,
    )
    .map_err(|error| conflict(format!("Service Test receipt JSON is not UTF-8: {error}")))?;
    Ok((canonical, receipt))
}

fn validate_persisted_service_test_receipt(
    row: &PluginRuntimeServiceTestReceiptRow,
) -> Result<PluginServiceTestReceipt, DbError> {
    let receipt: PluginServiceTestReceipt = serde_json::from_str(&row.receipt_json)
        .map_err(|error| DbError::Init(format!("persisted Service Test receipt is invalid: {error}")))?;
    let canonical = String::from_utf8(canonical_json_bytes(&receipt).map_err(|error| {
        DbError::Init(format!(
            "persisted Service Test receipt cannot be canonicalized: {error}"
        ))
    })?)
    .map_err(|error| {
        DbError::Init(format!(
            "persisted Service Test receipt canonical JSON is not UTF-8: {error}"
        ))
    })?;
    let receipt_digest = digest_payload(&receipt).map_err(|error| {
        DbError::Init(format!(
            "persisted Service Test receipt cannot be digested: {error}"
        ))
    })?;
    let runtime_digest = digest_payload(&receipt.runtime).map_err(|error| {
        DbError::Init(format!(
            "persisted Service Test runtime cannot be digested: {error}"
        ))
    })?;
    if canonical != row.receipt_json
        || receipt.receipt_id.as_ref() != row.receipt_id
        || receipt.plugin_product_id.as_ref() != row.plugin_product_id
        || receipt.release.release_id.as_ref() != row.release_id
        || receipt.release.release_digest.as_ref() != row.release_digest
        || receipt.service_run_key.as_ref() != row.service_run_key
        || service_test_outcome(receipt.outcome) != row.outcome
        || receipt.error_code.as_ref().map(AsRef::as_ref) != row.error_code.as_deref()
        || receipt.resolved_test_input_digest.as_ref() != row.resolved_test_input_digest
        || receipt.issued_at_ms != row.issued_at_ms
        || receipt_digest.as_ref() != row.receipt_digest
        || runtime_digest.as_ref() != row.runtime_fingerprint_digest
    {
        return Err(DbError::Init(
            "persisted Service Test receipt row does not match its canonical JSON".to_owned(),
        ));
    }
    Ok(receipt)
}

async fn persist_or_reuse_artifact(
    tx: &mut Transaction<'_, Sqlite>,
    incoming: &PluginRuntimeReleaseArtifactRow,
) -> Result<PluginRuntimeReleaseArtifactRow, DbError> {
    let (incoming, incoming_payload) = normalize_incoming_artifact(incoming)?;
    sqlx::query(
        "INSERT INTO plugin_release_artifacts
         (artifact_id, owner_user_id, artifact_digest, manifest_digest,
          artifact_record_json, managed_path, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (artifact_digest) DO NOTHING",
    )
    .bind(&incoming.artifact_id)
    .bind(&incoming.owner_user_id)
    .bind(&incoming.artifact_digest)
    .bind(&incoming.manifest_digest)
    .bind(&incoming.artifact_record_json)
    .bind(&incoming.managed_path)
    .bind(incoming.created_at)
    .execute(&mut **tx)
    .await
    .map_err(query_error)?;

    let stored = sqlx::query_as::<_, PluginRuntimeReleaseArtifactRow>(
        "SELECT * FROM plugin_release_artifacts
         WHERE owner_user_id = ? AND artifact_digest = ?",
    )
    .bind(&incoming.owner_user_id)
    .bind(&incoming.artifact_digest)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| {
        conflict("Plugin Artifact digest is owned by another owner")
    })?;
    let stored_payload = validate_artifact(&stored)?;
    let mut comparable_incoming = incoming_payload;
    comparable_incoming.artifact_id = stored_payload.artifact_id.clone();
    if stored.manifest_digest != incoming.manifest_digest
        || comparable_incoming != stored_payload
    {
        return Err(conflict(
            "Plugin Artifact digest is already bound to different immutable metadata",
        ));
    }
    Ok(stored)
}

#[allow(clippy::too_many_arguments)]
async fn synchronize_catalog_publication(
    tx: &mut Transaction<'_, Sqlite>,
    owner_user_id: &str,
    plugin_product_id: &str,
    lifecycle: &str,
    active_release_id: Option<&str>,
    active_release_digest: Option<&str>,
    active_release_epoch: i64,
    catalog_digest: &str,
) -> Result<(), DbError> {
    if lifecycle == "enabled" {
        let (Some(active_release_id), Some(active_release_digest)) =
            (active_release_id, active_release_digest)
        else {
            return Err(conflict(
                "enabled Plugin Catalog publication requires an Active Release",
            ));
        };
        sqlx::query(
            "INSERT INTO plugin_catalog_publications (
                plugin_product_id, owner_user_id, active_release_id,
                active_release_digest, active_release_epoch, catalog_digest
             ) VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(plugin_product_id) DO UPDATE SET
                owner_user_id = excluded.owner_user_id,
                active_release_id = excluded.active_release_id,
                active_release_digest = excluded.active_release_digest,
                active_release_epoch = excluded.active_release_epoch,
                catalog_digest = excluded.catalog_digest",
        )
        .bind(plugin_product_id)
        .bind(owner_user_id)
        .bind(active_release_id)
        .bind(active_release_digest)
        .bind(active_release_epoch)
        .bind(catalog_digest)
        .execute(&mut **tx)
        .await
        .map_err(query_error)?;
    } else {
        sqlx::query(
            "DELETE FROM plugin_catalog_publications
             WHERE owner_user_id = ? AND plugin_product_id = ?",
        )
        .bind(owner_user_id)
        .bind(plugin_product_id)
        .execute(&mut **tx)
        .await
        .map_err(query_error)?;
    }
    Ok(())
}

async fn revoke_surface_session(
    tx: &mut Transaction<'_, Sqlite>,
    owner_user_id: &str,
    plugin_product_id: &str,
) -> Result<(), DbError> {
    sqlx::query(
        "DELETE FROM plugin_surface_sessions
         WHERE owner_user_id = ? AND plugin_product_id = ?",
    )
    .bind(owner_user_id)
    .bind(plugin_product_id)
    .execute(&mut **tx)
    .await
    .map_err(query_error)?;
    Ok(())
}

async fn fetch_kv_row(
    tx: &mut Transaction<'_, Sqlite>,
    owner_user_id: &str,
    plugin_product_id: &str,
    namespace: &str,
    key: &str,
) -> Result<Option<PluginRuntimeKvRow>, DbError> {
    let row = sqlx::query_as::<_, PluginRuntimeKvRow>(
        "SELECT * FROM plugin_kv
         WHERE owner_user_id = ? AND plugin_product_id = ?
           AND namespace = ? AND key = ?",
    )
    .bind(owner_user_id)
    .bind(plugin_product_id)
    .bind(namespace)
    .bind(key)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(row) = &row {
        validate_kv_row(row)?;
    }
    Ok(row)
}

fn next_kv_revision(revision: i64) -> Result<i64, DbError> {
    revision
        .checked_add(1)
        .ok_or_else(|| conflict("Plugin KV revision overflow"))
}

fn next_kv_generation(generation: i64) -> Result<i64, DbError> {
    generation
        .checked_add(1)
        .ok_or_else(|| conflict("Plugin KV key generation overflow"))
}

async fn insert_live_kv(
    tx: &mut Transaction<'_, Sqlite>,
    owner_user_id: &str,
    plugin_product_id: &str,
    namespace: &str,
    key: &str,
    value_json: &str,
    updated_at: i64,
) -> Result<PluginRuntimeKvRow, DbError> {
    let inserted = sqlx::query(
        "INSERT INTO plugin_kv (
            plugin_product_id, owner_user_id, namespace, key, value_json,
            revision, key_generation, is_tombstone, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, 1, 1, 0, ?, ?)",
    )
    .bind(plugin_product_id)
    .bind(owner_user_id)
    .bind(namespace)
    .bind(key)
    .bind(value_json)
    .bind(updated_at)
    .bind(updated_at)
    .execute(&mut **tx)
    .await
    .map_err(query_error)?;
    if inserted.rows_affected() != 1 {
        return Err(conflict(
            "Plugin KV key creation CAS found an existing logical key",
        ));
    }
    fetch_kv_row(tx, owner_user_id, plugin_product_id, namespace, key)
        .await?
        .ok_or_else(|| DbError::Init("Plugin KV insert lost its key".into()))
}

async fn write_live_kv(
    tx: &mut Transaction<'_, Sqlite>,
    row: &PluginRuntimeKvRow,
    value_json: &str,
    updated_at: i64,
) -> Result<PluginRuntimeKvRow, DbError> {
    if updated_at < row.updated_at {
        return Err(conflict(
            "Plugin KV write timestamp predates the existing key",
        ));
    }
    let next_revision = next_kv_revision(row.revision)?;
    let changed = sqlx::query(
        "UPDATE plugin_kv
         SET value_json = ?, revision = ?, is_tombstone = 0, updated_at = ?
         WHERE owner_user_id = ? AND plugin_product_id = ?
           AND namespace = ? AND key = ? AND revision = ?
           AND key_generation = ? AND updated_at <= ?",
    )
    .bind(value_json)
    .bind(next_revision)
    .bind(updated_at)
    .bind(&row.owner_user_id)
    .bind(&row.plugin_product_id)
    .bind(&row.namespace)
    .bind(&row.key)
    .bind(row.revision)
    .bind(row.key_generation)
    .bind(updated_at)
    .execute(&mut **tx)
    .await
    .map_err(query_error)?;
    if changed.rows_affected() != 1 {
        return Err(conflict("Plugin KV live write lost its revision CAS"));
    }
    fetch_kv_row(
        tx,
        &row.owner_user_id,
        &row.plugin_product_id,
        &row.namespace,
        &row.key,
    )
    .await?
    .ok_or_else(|| DbError::Init("Plugin KV live write lost its key".into()))
}

async fn tombstone_kv(
    tx: &mut Transaction<'_, Sqlite>,
    row: &PluginRuntimeKvRow,
    updated_at: i64,
) -> Result<Option<PluginRuntimeKvRow>, DbError> {
    if row.is_tombstone {
        return Ok(Some(row.clone()));
    }
    if updated_at < row.updated_at {
        return Err(conflict(
            "Plugin KV delete timestamp predates the existing key",
        ));
    }
    let next_revision = next_kv_revision(row.revision)?;
    let next_generation = next_kv_generation(row.key_generation)?;
    let changed = sqlx::query(
        "UPDATE plugin_kv
         SET value_json = 'null', revision = ?, key_generation = ?,
             is_tombstone = 1, updated_at = ?
         WHERE owner_user_id = ? AND plugin_product_id = ?
           AND namespace = ? AND key = ? AND revision = ?
           AND key_generation = ? AND is_tombstone = 0
           AND updated_at <= ?",
    )
    .bind(next_revision)
    .bind(next_generation)
    .bind(updated_at)
    .bind(&row.owner_user_id)
    .bind(&row.plugin_product_id)
    .bind(&row.namespace)
    .bind(&row.key)
    .bind(row.revision)
    .bind(row.key_generation)
    .bind(updated_at)
    .execute(&mut **tx)
    .await
    .map_err(query_error)?;
    if changed.rows_affected() != 1 {
        return Err(conflict("Plugin KV delete lost its revision CAS"));
    }
    fetch_kv_row(
        tx,
        &row.owner_user_id,
        &row.plugin_product_id,
        &row.namespace,
        &row.key,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn terminalize_import_operation(
    pool: &SqlitePool,
    owner_user_id: &str,
    plugin_product_id: &str,
    project_id: &str,
    operation_id: &str,
    expected_product_revision: i64,
    expected_pointer_revision: i64,
    expected_project_revision: i64,
    state: ProductOperationState,
    progress_percent: Option<u8>,
    error_code: Option<&str>,
    bounded_log_tail: &[String],
    finished_at_ms: i64,
) -> Result<ProductOperationRow, DbError> {
    validate_uuid(owner_user_id, "owner_user_id")?;
    validate_uuid(plugin_product_id, "plugin_product_id")?;
    validate_uuid(project_id, "project_id")?;
    validate_uuid(operation_id, "operation_id")?;
    validate_product_operation_error_code(error_code)?;
    if expected_product_revision < 1
        || expected_pointer_revision < 1
        || expected_project_revision < 1
        || finished_at_ms <= 0
        || progress_percent.is_some_and(|progress| progress > 100)
        || !matches!(
            (state, error_code),
            (ProductOperationState::Failed, Some(_))
                | (ProductOperationState::Canceled, None)
        )
    {
        return Err(conflict(
            "Plugin Import terminal operation expectations are invalid",
        ));
    }
    let bounded_log_tail_json = serialize_product_operation_log_tail(bounded_log_tail)?;
    let mut tx = pool.begin().await?;
    let product = lock_product_for_update(&mut tx, owner_user_id, plugin_product_id).await?;
    if product.lifecycle != "disabled"
        || product.product_revision != expected_product_revision
        || product.pointer_revision != expected_pointer_revision
        || product.ready_release_id.is_some()
        || product.active_release_id.is_some()
        || product.previous_release_id.is_some()
    {
        return Err(conflict(
            "Plugin Import terminal operation lost its exact disabled Product CAS",
        ));
    }
    let project = fetch_project(&mut tx, owner_user_id, plugin_product_id, project_id).await?;
    if project.project_revision != expected_project_revision {
        return Err(conflict(
            "Plugin Import terminal operation lost its exact Project CAS",
        ));
    }
    let operation = fetch_plugin_operation_in_tx(&mut tx, plugin_product_id, operation_id)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("Plugin Import operation {operation_id}")))?;
    if operation.kind != "import"
        || operation.state != ProductOperationState::Running.as_str()
        || operation.progress_percent.is_none()
        || operation.last_error_code.is_some()
        || operation.finished_at_ms.is_some()
        || finished_at_ms < operation.started_at_ms
        || finished_at_ms < product.updated_at
        || finished_at_ms < project.updated_at
    {
        return Err(conflict(
            "Plugin Import terminal operation lost its running-operation CAS",
        ));
    }
    let release_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM plugin_releases
         WHERE owner_user_id = ? AND plugin_product_id = ?
           AND origin_kind = 'import' AND origin_operation_id = ?",
    )
    .bind(owner_user_id)
    .bind(plugin_product_id)
    .bind(operation_id)
    .fetch_one(&mut *tx)
    .await?;
    let catalog_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM plugin_catalog_publications
         WHERE owner_user_id = ? AND plugin_product_id = ?",
    )
    .bind(owner_user_id)
    .bind(plugin_product_id)
    .fetch_one(&mut *tx)
    .await?;
    if release_count != 0 || catalog_count != 0 {
        return Err(DbError::Init(
            "unfinished Plugin Import leaked Release or Catalog state".to_owned(),
        ));
    }
    let changed = sqlx::query(
        "UPDATE product_operations
         SET state = ?, progress_percent = ?, last_error_code = ?,
             bounded_log_tail_json = ?, finished_at_ms = ?
         WHERE operation_id = ? AND kind = 'import'
           AND owner_kind = 'plugin' AND owner_id = ? AND state = 'running'",
    )
    .bind(state.as_str())
    .bind(progress_percent.map(i64::from).or(operation.progress_percent))
    .bind(error_code)
    .bind(bounded_log_tail_json)
    .bind(finished_at_ms)
    .bind(operation_id)
    .bind(plugin_product_id)
    .execute(&mut *tx)
    .await
    .map_err(query_error)?;
    if changed.rows_affected() != 1 {
        return Err(conflict(
            "Plugin Import terminal update lost its running-operation CAS",
        ));
    }
    let operation = sqlx::query_as::<_, ProductOperationRow>(
        "SELECT * FROM product_operations WHERE operation_id = ?",
    )
    .bind(operation_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(DbError::Query)?;
    tx.commit().await?;
    Ok(operation)
}

#[allow(clippy::too_many_arguments)]
async fn terminalize_export_operation(
    pool: &SqlitePool,
    owner_user_id: &str,
    plugin_product_id: &str,
    operation_id: &str,
    state: ProductOperationState,
    progress_percent: Option<u8>,
    error_code: Option<&str>,
    bounded_log_tail: &[String],
    finished_at_ms: i64,
) -> Result<ProductOperationRow, DbError> {
    validate_uuid(owner_user_id, "owner_user_id")?;
    validate_uuid(plugin_product_id, "plugin_product_id")?;
    validate_uuid(operation_id, "operation_id")?;
    validate_product_operation_error_code(error_code)?;
    if finished_at_ms <= 0
        || progress_percent.is_some_and(|progress| progress > 100)
        || !matches!(
            (state, error_code),
            (ProductOperationState::Succeeded, None)
                | (ProductOperationState::Failed, Some(_))
                | (ProductOperationState::Canceled, None)
        )
    {
        return Err(conflict(
            "Plugin Export terminal operation expectations are invalid",
        ));
    }
    let bounded_log_tail_json = serialize_product_operation_log_tail(bounded_log_tail)?;
    let mut tx = pool.begin().await?;
    lock_product_for_update(&mut tx, owner_user_id, plugin_product_id).await?;
    let operation = fetch_plugin_operation_in_tx(&mut tx, plugin_product_id, operation_id)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("Plugin Export operation {operation_id}")))?;
    if operation.kind != "export"
        || operation.state != ProductOperationState::Running.as_str()
        || operation.progress_percent.is_none()
        || operation.last_error_code.is_some()
        || operation.finished_at_ms.is_some()
        || finished_at_ms < operation.started_at_ms
    {
        return Err(conflict(
            "Plugin Export terminal operation lost its running-operation CAS",
        ));
    }
    let terminal_progress = if state == ProductOperationState::Succeeded {
        Some(100_i64)
    } else {
        progress_percent.map(i64::from).or(operation.progress_percent)
    };
    let changed = sqlx::query(
        "UPDATE product_operations
         SET state = ?, progress_percent = ?, last_error_code = ?,
             bounded_log_tail_json = ?, finished_at_ms = ?
         WHERE operation_id = ? AND kind = 'export'
           AND owner_kind = 'plugin' AND owner_id = ? AND state = 'running'",
    )
    .bind(state.as_str())
    .bind(terminal_progress)
    .bind(error_code)
    .bind(bounded_log_tail_json)
    .bind(finished_at_ms)
    .bind(operation_id)
    .bind(plugin_product_id)
    .execute(&mut *tx)
    .await
    .map_err(query_error)?;
    if changed.rows_affected() != 1 {
        return Err(conflict(
            "Plugin Export terminal update lost its running-operation CAS",
        ));
    }
    let operation = sqlx::query_as::<_, ProductOperationRow>(
        "SELECT * FROM product_operations WHERE operation_id = ?",
    )
    .bind(operation_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(DbError::Query)?;
    tx.commit().await?;
    Ok(operation)
}

#[async_trait::async_trait]
impl IPluginRuntimeRepository for SqlitePluginRuntimeRepository {
    async fn library(
        &self,
        owner_user_id: &str,
    ) -> Result<PluginRuntimeLibrarySnapshot, DbError> {
        nomifun_common::validate_uuidv7(owner_user_id)
            .map_err(|error| DbError::Conflict(error.to_string()))?;
        ensure_owner(&self.pool, owner_user_id).await?;
        let mut tx = self.pool.begin().await?;
        let library = sqlx::query_as::<_, PluginRuntimeLibraryStateRow>(
            "SELECT * FROM plugin_library_state WHERE owner_user_id = ?",
        )
        .bind(owner_user_id)
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or_else(|| PluginRuntimeLibraryStateRow {
            id: 0,
            singleton_key: "plugin_runtime".to_owned(),
            owner_user_id: owner_user_id.to_owned(),
            revision: 0,
            updated_at: 0,
        });
        let products = sqlx::query_as::<_, PluginRuntimeProductRow>(
            "SELECT * FROM plugin_products
             WHERE owner_user_id = ? ORDER BY updated_at DESC, id DESC",
        )
        .bind(owner_user_id)
        .fetch_all(&mut *tx)
        .await?;
        for product in &products {
            validate_deletion_state_in_tx(&mut tx, product).await?;
        }
        tx.commit().await?;
        Ok(PluginRuntimeLibrarySnapshot { library, products })
    }

    async fn get(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
    ) -> Result<Option<PluginRuntimeSnapshot>, DbError> {
        nomifun_common::validate_uuidv7(owner_user_id)
            .map_err(|error| DbError::Conflict(error.to_string()))?;
        nomifun_common::validate_uuidv7(plugin_product_id)
            .map_err(|error| DbError::Conflict(error.to_string()))?;
        ensure_owner(&self.pool, owner_user_id).await?;
        fetch_snapshot(&self.pool, owner_user_id, plugin_product_id).await
    }

    async fn create(
        &self,
        params: &CreatePluginRuntimeParams,
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        self.create_inner(params, None).await
    }

    async fn create_with_source(
        &self,
        params: &CreatePluginRuntimeWithSourceParams,
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        self.create_inner(&params.create, Some(&params.source)).await
    }

    async fn begin_import_as_new(
        &self,
        params: &BeginPluginRuntimeImportAsNewParams,
    ) -> Result<BeginPluginRuntimeImportAsNewResult, DbError> {
        let create = &params.create;
        validate_uuid(&create.owner_user_id, "owner_user_id")?;
        validate_uuid(&create.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&create.project_id, "project_id")?;
        validate_uuid(&params.operation_id, "operation_id")?;
        if create.expected_library_revision < 0
            || create.created_at < 0
            || params.started_at_ms <= 0
            || params.started_at_ms < create.created_at
        {
            return Err(conflict(
                "Plugin Import create CAS/timestamps are invalid",
            ));
        }
        if create.display_name.trim().is_empty()
            || create.display_name.chars().count() > 255
        {
            return Err(conflict(
                "Plugin display_name must contain 1 to 255 characters",
            ));
        }
        validate_json_object(&create.config_schema_json, "config_schema_json")?;
        validate_json_object(&create.config_json, "config_json")?;
        validate_digest(
            &create.materialized_catalog_digest,
            "materialized_catalog_digest",
        )?;
        let source_fields = match &params.source {
            PluginRuntimeImportSource::Managed(source) => {
                validate_managed_source_lineage(source)?;
                if source.build_profile_version != PLUGIN_RELEASE_PROFILE_VERSION {
                    return Err(conflict(
                        "Plugin managed Import uses an unsupported release profile version",
                    ));
                }
                (
                    PluginRuntimeProjectSourceState::Editable,
                    Some(source.managed_source_path.as_str()),
                    Some(source.source_head_digest.as_str()),
                    Some(source.dependency_lock_digest.as_str()),
                    Some(source.build_profile_version.as_str()),
                    source.build_generation,
                )
            }
            PluginRuntimeImportSource::RuntimeOnly => (
                PluginRuntimeProjectSourceState::RuntimeOnly,
                None,
                None,
                None,
                None,
                0,
            ),
        };
        validate_project_source(
            source_fields.0,
            source_fields.1,
            source_fields.2,
            source_fields.3,
            source_fields.4,
            source_fields.5,
        )?;
        let bounded_log_tail_json =
            serialize_product_operation_log_tail(&params.bounded_log_tail)?;
        ensure_owner(&self.pool, &create.owner_user_id).await?;

        let mut tx = self.pool.begin().await?;
        let current = sqlx::query_as::<_, PluginRuntimeLibraryStateRow>(
            "SELECT * FROM plugin_library_state WHERE owner_user_id = ?",
        )
        .bind(&create.owner_user_id)
        .fetch_optional(&mut *tx)
        .await?;
        let current_revision = current.as_ref().map_or(0, |row| row.revision);
        if current_revision != create.expected_library_revision {
            return Err(conflict(format!(
                "Plugin Import library revision changed from expected {} to {}",
                create.expected_library_revision, current_revision
            )));
        }
        let next_library_revision = current_revision
            .checked_add(1)
            .ok_or_else(|| conflict("Plugin library revision overflow"))?;
        if current.is_none() {
            sqlx::query(
                "INSERT INTO plugin_library_state
                 (singleton_key, owner_user_id, revision, updated_at)
                 VALUES ('plugin_runtime', ?, ?, ?)",
            )
            .bind(&create.owner_user_id)
            .bind(next_library_revision)
            .bind(create.created_at)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?;
        } else {
            let changed = sqlx::query(
                "UPDATE plugin_library_state
                 SET revision = ?, updated_at = ?
                 WHERE owner_user_id = ? AND revision = ? AND updated_at <= ?",
            )
            .bind(next_library_revision)
            .bind(create.created_at)
            .bind(&create.owner_user_id)
            .bind(current_revision)
            .bind(create.created_at)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?;
            if changed.rows_affected() != 1 {
                return Err(conflict(
                    "Plugin Import library create CAS or timestamp check failed",
                ));
            }
        }
        sqlx::query(
            "INSERT INTO plugin_products
             (plugin_product_id, owner_user_id, display_name, description,
              icon_asset_id, kind, materialized_catalog_digest,
              config_schema_json, config_json, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&create.plugin_product_id)
        .bind(&create.owner_user_id)
        .bind(&create.display_name)
        .bind(&create.description)
        .bind(&create.icon_asset_id)
        .bind(create.kind.as_str())
        .bind(&create.materialized_catalog_digest)
        .bind(&create.config_schema_json)
        .bind(&create.config_json)
        .bind(create.created_at)
        .bind(create.created_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        sqlx::query(
            "INSERT INTO plugin_projects
             (project_id, plugin_product_id, owner_user_id, source_state,
              managed_source_path, source_head_digest, dependency_lock_digest,
              build_profile_version, build_generation, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&create.project_id)
        .bind(&create.plugin_product_id)
        .bind(&create.owner_user_id)
        .bind(source_fields.0.as_str())
        .bind(source_fields.1)
        .bind(source_fields.2)
        .bind(source_fields.3)
        .bind(source_fields.4)
        .bind(source_fields.5)
        .bind(create.created_at)
        .bind(create.created_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        sqlx::query(
            "INSERT INTO product_operations (
                operation_id, kind, owner_kind, owner_id, state,
                progress_percent, bounded_log_tail_json, started_at_ms
             ) VALUES (?, 'import', 'plugin', ?, 'running', 0, ?, ?)",
        )
        .bind(&params.operation_id)
        .bind(&create.plugin_product_id)
        .bind(bounded_log_tail_json)
        .bind(params.started_at_ms)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let snapshot =
            fetch_snapshot_in_tx(&mut tx, &create.owner_user_id, &create.plugin_product_id)
                .await?
                .ok_or_else(|| DbError::Init("Plugin Import create lost Product".into()))?;
        let operation = sqlx::query_as::<_, ProductOperationRow>(
            "SELECT * FROM product_operations
             WHERE operation_id = ? AND kind = 'import'
               AND owner_kind = 'plugin' AND owner_id = ?",
        )
        .bind(&params.operation_id)
        .bind(&create.plugin_product_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(DbError::Query)?;
        tx.commit().await?;
        Ok(BeginPluginRuntimeImportAsNewResult {
            snapshot,
            operation,
        })
    }

    async fn finish_import_ready(
        &self,
        params: &FinishPluginRuntimeImportReadyParams,
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&params.project_id, "project_id")?;
        validate_uuid(&params.operation_id, "operation_id")?;
        if params.expected_library_revision < 1
            || params.expected_product_revision < 1
            || params.expected_pointer_revision < 1
            || params.expected_project_revision < 1
            || params.finished_at_ms <= 0
        {
            return Err(conflict(
                "Plugin Import Ready CAS expectations are invalid",
            ));
        }
        let bounded_log_tail_json =
            serialize_product_operation_log_tail(&params.bounded_log_tail)?;
        let artifact_payload = validate_artifact(&params.artifact)?;
        validate_release(&params.release, &artifact_payload)?;
        if params.artifact.owner_user_id != params.owner_user_id
            || params.release.owner_user_id != params.owner_user_id
            || params.release.plugin_product_id != params.plugin_product_id
            || params.release.origin_kind != "import"
            || params.release.origin_operation_id != params.operation_id
            || params.release.artifact_id != params.artifact.artifact_id
            || params.release.artifact_digest != params.artifact.artifact_digest
            || params.release.manifest_digest != params.artifact.manifest_digest
            || params.release.release_digest != params.artifact.artifact_digest
            || params.artifact.created_at > params.finished_at_ms
            || params.release.created_at > params.finished_at_ms
        {
            return Err(conflict(
                "Plugin Import Ready does not bind its exact operation and Artifact",
            ));
        }

        let mut tx = self.pool.begin().await?;
        let product =
            lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        validate_product_artifact_contract(&product.kind, &artifact_payload)?;
        if product.lifecycle != "disabled"
            || product.product_revision != params.expected_product_revision
            || product.pointer_revision != params.expected_pointer_revision
            || product.ready_release_id.is_some()
            || product.active_release_id.is_some()
            || product.previous_release_id.is_some()
        {
            return Err(conflict(
                "Plugin Import Ready lost its exact disabled Product CAS",
            ));
        }
        let library_revision: i64 = sqlx::query_scalar(
            "SELECT revision FROM plugin_library_state WHERE owner_user_id = ?",
        )
        .bind(&params.owner_user_id)
        .fetch_one(&mut *tx)
        .await?;
        if library_revision != params.expected_library_revision {
            return Err(conflict(
                "Plugin Import Ready lost its exact Library revision CAS",
            ));
        }
        let project = fetch_project(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
            &params.project_id,
        )
        .await?;
        if project.project_revision != params.expected_project_revision {
            return Err(conflict(
                "Plugin Import Ready lost its exact Project revision CAS",
            ));
        }
        match params.release.source_kind.as_str() {
            "managed"
                if project.source_state == "editable"
                    && params.release.project_id.as_deref()
                        == Some(params.project_id.as_str())
                    && params.release.source_snapshot_digest == project.source_head_digest
                    && params.release.dependency_lock_digest == project.dependency_lock_digest
                    && params.release.build_profile_version == project.build_profile_version
                    && params.release.build_generation == Some(project.build_generation) => {}
            "runtime_only"
                if project.source_state == "runtime_only"
                    && params.release.project_id.is_none()
                    && params.release.source_snapshot_digest.is_none()
                    && params.release.dependency_lock_digest.is_none()
                    && params.release.build_profile_version.is_none()
                    && params.release.build_generation.is_none() => {}
            _ => {
                return Err(conflict(
                    "Plugin Import Ready lineage differs from its exact Project source state",
                ));
            }
        }
        let operation = fetch_plugin_operation_in_tx(
            &mut tx,
            &params.plugin_product_id,
            &params.operation_id,
        )
        .await?
        .ok_or_else(|| {
            DbError::NotFound(format!(
                "Plugin Import operation {}",
                params.operation_id
            ))
        })?;
        if operation.kind != "import"
            || operation.state != ProductOperationState::Running.as_str()
            || operation.progress_percent.is_none()
            || operation.last_error_code.is_some()
            || operation.finished_at_ms.is_some()
            || params.finished_at_ms < operation.started_at_ms
            || params.artifact.created_at < operation.started_at_ms
            || params.release.created_at < operation.started_at_ms
            || params.finished_at_ms < product.updated_at
            || params.finished_at_ms < project.updated_at
        {
            return Err(conflict(
                "Plugin Import Ready lost its exact running-operation CAS",
            ));
        }
        let catalog_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM plugin_catalog_publications
             WHERE owner_user_id = ? AND plugin_product_id = ?",
        )
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .fetch_one(&mut *tx)
        .await?;
        if catalog_count != 0 {
            return Err(DbError::Init(
                "unfinished Plugin Import unexpectedly published Catalog state".to_owned(),
            ));
        }

        let persisted_artifact =
            persist_or_reuse_artifact(&mut tx, &params.artifact).await?;
        let mut release = params.release.clone();
        release.artifact_id = persisted_artifact.artifact_id.clone();
        release.manifest_digest = persisted_artifact.manifest_digest.clone();
        if params.release.artifact_id != persisted_artifact.artifact_id {
            release.release_record_json = rewrite_release_record_artifact_id(
                &release.release_record_json,
                &persisted_artifact.artifact_id,
            )?;
        }
        let persisted_artifact_payload = validate_artifact(&persisted_artifact)?;
        validate_release(&release, &persisted_artifact_payload)?;
        sqlx::query(
            "INSERT INTO plugin_releases
             (release_id, plugin_product_id, owner_user_id, artifact_id,
              artifact_digest, manifest_digest, release_digest, origin_kind,
              origin_operation_id, source_kind, project_id, source_snapshot_digest,
              dependency_lock_digest, build_profile_version, build_generation,
              release_record_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 'import', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&release.release_id)
        .bind(&release.plugin_product_id)
        .bind(&release.owner_user_id)
        .bind(&release.artifact_id)
        .bind(&release.artifact_digest)
        .bind(&release.manifest_digest)
        .bind(&release.release_digest)
        .bind(&release.origin_operation_id)
        .bind(&release.source_kind)
        .bind(&release.project_id)
        .bind(&release.source_snapshot_digest)
        .bind(&release.dependency_lock_digest)
        .bind(&release.build_profile_version)
        .bind(release.build_generation)
        .bind(&release.release_record_json)
        .bind(release.created_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let product_updated = sqlx::query(
            "UPDATE plugin_products
             SET product_revision = product_revision + 1,
                 pointer_revision = pointer_revision + 1,
                 ready_release_id = ?, ready_release_digest = ?,
                 updated_at = ?
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND lifecycle = 'disabled'
               AND product_revision = ? AND pointer_revision = ?
               AND ready_release_id IS NULL AND active_release_id IS NULL
               AND previous_release_id IS NULL AND updated_at <= ?",
        )
        .bind(&release.release_id)
        .bind(&release.release_digest)
        .bind(params.finished_at_ms)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(params.expected_product_revision)
        .bind(params.expected_pointer_revision)
        .bind(params.finished_at_ms)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if product_updated.rows_affected() != 1 {
            return Err(conflict("Plugin Import Ready Product/pointer CAS failed"));
        }
        let library_updated = sqlx::query(
            "UPDATE plugin_library_state
             SET revision = revision + 1, updated_at = ?
             WHERE owner_user_id = ? AND revision = ? AND updated_at <= ?",
        )
        .bind(params.finished_at_ms)
        .bind(&params.owner_user_id)
        .bind(params.expected_library_revision)
        .bind(params.finished_at_ms)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if library_updated.rows_affected() != 1 {
            return Err(conflict("Plugin Import Ready Library revision CAS failed"));
        }
        let operation_updated = sqlx::query(
            "UPDATE product_operations
             SET state = 'succeeded', progress_percent = 100,
                 last_error_code = NULL, bounded_log_tail_json = ?,
                 finished_at_ms = ?
             WHERE operation_id = ? AND kind = 'import'
               AND owner_kind = 'plugin' AND owner_id = ? AND state = 'running'",
        )
        .bind(bounded_log_tail_json)
        .bind(params.finished_at_ms)
        .bind(&params.operation_id)
        .bind(&params.plugin_product_id)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if operation_updated.rows_affected() != 1 {
            return Err(conflict(
                "Plugin Import success lost its running-operation CAS",
            ));
        }
        let snapshot =
            fetch_snapshot_in_tx(&mut tx, &params.owner_user_id, &params.plugin_product_id)
                .await?
                .ok_or_else(|| DbError::Init("Plugin Import Ready lost Product".into()))?;
        tx.commit().await?;
        Ok(snapshot)
    }

    async fn fail_import(
        &self,
        params: &FailPluginRuntimeImportParams,
    ) -> Result<ProductOperationRow, DbError> {
        terminalize_import_operation(
            &self.pool,
            &params.owner_user_id,
            &params.plugin_product_id,
            &params.project_id,
            &params.operation_id,
            params.expected_product_revision,
            params.expected_pointer_revision,
            params.expected_project_revision,
            ProductOperationState::Failed,
            Some(params.progress_percent),
            Some(&params.error_code),
            &params.bounded_log_tail,
            params.finished_at_ms,
        )
        .await
    }

    async fn cancel_import(
        &self,
        params: &CancelPluginRuntimeImportParams,
    ) -> Result<ProductOperationRow, DbError> {
        terminalize_import_operation(
            &self.pool,
            &params.owner_user_id,
            &params.plugin_product_id,
            &params.project_id,
            &params.operation_id,
            params.expected_product_revision,
            params.expected_pointer_revision,
            params.expected_project_revision,
            ProductOperationState::Canceled,
            None,
            None,
            &params.bounded_log_tail,
            params.finished_at_ms,
        )
        .await
    }

    async fn start_export_operation(
        &self,
        params: &StartPluginRuntimeExportOperationParams,
    ) -> Result<ProductOperationRow, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&params.operation_id, "operation_id")?;
        if params.expected_product_revision < 1
            || params.expected_pointer_revision < 1
            || params.started_at_ms <= 0
        {
            return Err(conflict(
                "Plugin Export start CAS/timestamp is invalid",
            ));
        }
        let bounded_log_tail_json =
            serialize_product_operation_log_tail(&params.bounded_log_tail)?;
        let mut tx = self.pool.begin().await?;
        let product =
            lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        if matches!(product.lifecycle.as_str(), "trashed" | "deleting")
            || product.product_revision != params.expected_product_revision
            || product.pointer_revision != params.expected_pointer_revision
            || params.started_at_ms < product.updated_at
        {
            return Err(conflict(
                "Plugin Export start lost its exact non-trashed Product CAS",
            ));
        }
        ensure_no_running_plugin_operation(&mut tx, &params.plugin_product_id).await?;
        sqlx::query(
            "INSERT INTO product_operations (
                operation_id, kind, owner_kind, owner_id, state,
                progress_percent, bounded_log_tail_json, started_at_ms
             ) VALUES (?, 'export', 'plugin', ?, 'running', 0, ?, ?)",
        )
        .bind(&params.operation_id)
        .bind(&params.plugin_product_id)
        .bind(bounded_log_tail_json)
        .bind(params.started_at_ms)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let operation = sqlx::query_as::<_, ProductOperationRow>(
            "SELECT * FROM product_operations
             WHERE operation_id = ? AND kind = 'export'
               AND owner_kind = 'plugin' AND owner_id = ?",
        )
        .bind(&params.operation_id)
        .bind(&params.plugin_product_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(DbError::Query)?;
        tx.commit().await?;
        Ok(operation)
    }

    async fn start_backup_export(
        &self,
        params: &StartPluginRuntimeBackupExportParams,
    ) -> Result<PluginRuntimeBackupExportSnapshot, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&params.operation_id, "operation_id")?;
        if params.expected_product_revision < 1
            || params.expected_pointer_revision < 1
            || params.expected_config_revision < 1
            || params.expected_credential_bindings_revision < 1
            || params.started_at_ms <= 0
        {
            return Err(conflict(
                "Plugin Whole-App Backup export CAS/timestamp is invalid",
            ));
        }
        let bounded_log_tail_json = serialize_product_operation_log_tail(&[
            "Plugin Whole-App Backup export started".to_owned(),
        ])?;
        let mut tx = self.pool.begin().await?;
        let product =
            lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        if product.lifecycle != "disabled"
            || product.product_revision != params.expected_product_revision
            || product.pointer_revision != params.expected_pointer_revision
            || product.config_revision != params.expected_config_revision
            || product.credential_bindings_revision
                != params.expected_credential_bindings_revision
            || params.started_at_ms < product.updated_at
        {
            return Err(conflict(
                "Whole-App Backup requires the exact disabled Product and configuration revisions",
            ));
        }
        ensure_no_running_plugin_operation(&mut tx, &params.plugin_product_id).await?;
        let catalog_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM plugin_catalog_publications
             WHERE owner_user_id = ? AND plugin_product_id = ?",
        )
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .fetch_one(&mut *tx)
        .await?;
        let surface_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM plugin_surface_sessions
             WHERE owner_user_id = ? AND plugin_product_id = ?",
        )
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .fetch_one(&mut *tx)
        .await?;
        if catalog_count != 0 || surface_count != 0 {
            return Err(conflict(
                "Whole-App Backup requires no Catalog publication or Surface session",
            ));
        }
        sqlx::query(
            "INSERT INTO product_operations (
                operation_id, kind, owner_kind, owner_id, state,
                progress_percent, bounded_log_tail_json, started_at_ms
             ) VALUES (?, 'export', 'plugin', ?, 'running', 0, ?, ?)",
        )
        .bind(&params.operation_id)
        .bind(&params.plugin_product_id)
        .bind(bounded_log_tail_json)
        .bind(params.started_at_ms)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;

        let snapshot = fetch_snapshot_in_tx(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
        )
        .await?
        .ok_or_else(|| DbError::Init("Whole-App Backup export lost Product".into()))?;
        let mut release_ids = BTreeSet::new();
        for release_id in [
            snapshot.product.ready_release_id.as_deref(),
            snapshot.product.active_release_id.as_deref(),
            snapshot.product.previous_release_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            release_ids.insert(release_id.to_owned());
        }
        let mut releases = Vec::with_capacity(release_ids.len());
        let mut artifact_ids = BTreeSet::new();
        for release_id in release_ids {
            let release = sqlx::query_as::<_, PluginRuntimeReleaseRow>(
                "SELECT * FROM plugin_releases
                 WHERE owner_user_id = ? AND plugin_product_id = ? AND release_id = ?",
            )
            .bind(&params.owner_user_id)
            .bind(&params.plugin_product_id)
            .bind(&release_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| {
                DbError::Init(format!(
                    "Whole-App Backup pointer references missing Release {release_id}"
                ))
            })?;
            let artifact = sqlx::query_as::<_, PluginRuntimeReleaseArtifactRow>(
                "SELECT * FROM plugin_release_artifacts
                 WHERE owner_user_id = ? AND artifact_id = ?",
            )
            .bind(&params.owner_user_id)
            .bind(&release.artifact_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| {
                DbError::Init(format!(
                    "Whole-App Backup Release {} references missing Artifact {}",
                    release.release_id, release.artifact_id
                ))
            })?;
            let artifact_payload = validate_artifact(&artifact)?;
            validate_product_artifact_contract(&product.kind, &artifact_payload)?;
            validate_release(&release, &artifact_payload)?;
            artifact_ids.insert(artifact.artifact_id.clone());
            releases.push(release);
        }
        let mut kv = sqlx::query_as::<_, PluginRuntimeKvRow>(
            "SELECT * FROM plugin_kv
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND namespace NOT LIKE 'service-test:%'
             ORDER BY namespace, key",
        )
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .fetch_all(&mut *tx)
        .await?;
        for row in &kv {
            if row.owner_user_id != params.owner_user_id
                || row.plugin_product_id != params.plugin_product_id
                || row.namespace.starts_with("service-test:")
            {
                return Err(DbError::Init(
                    "Whole-App Backup captured a foreign or transient KV row".into(),
                ));
            }
            validate_visible_ascii_key(&row.namespace, "Plugin KV namespace", 128)?;
            validate_visible_ascii_key(&row.key, "Plugin KV key", 256)?;
            validate_kv_row(row)?;
        }
        releases.sort_by(|left, right| left.release_id.cmp(&right.release_id));
        let mut artifacts = Vec::with_capacity(artifact_ids.len());
        for artifact_id in artifact_ids {
            let artifact = sqlx::query_as::<_, PluginRuntimeReleaseArtifactRow>(
                "SELECT * FROM plugin_release_artifacts
                 WHERE owner_user_id = ? AND artifact_id = ?",
            )
            .bind(&params.owner_user_id)
            .bind(&artifact_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(DbError::Query)?;
            artifacts.push(artifact);
        }
        let operation = sqlx::query_as::<_, ProductOperationRow>(
            "SELECT * FROM product_operations
             WHERE operation_id = ? AND kind = 'export'
               AND owner_kind = 'plugin' AND owner_id = ?",
        )
        .bind(&params.operation_id)
        .bind(&params.plugin_product_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(DbError::Query)?;
        tx.commit().await?;
        Ok(PluginRuntimeBackupExportSnapshot {
            snapshot,
            releases,
            artifacts,
            kv: std::mem::take(&mut kv),
            operation,
        })
    }

    async fn finish_backup_import(
        &self,
        params: &FinishPluginRuntimeBackupImportParams,
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&params.project_id, "project_id")?;
        validate_uuid(&params.operation_id, "operation_id")?;
        validate_digest(&params.target_catalog_digest, "target_catalog_digest")?;
        if params.expected_library_revision < 1
            || params.expected_product_revision < 1
            || params.expected_pointer_revision < 1
            || params.expected_project_revision < 1
            || params.finished_at_ms <= 0
            || params.releases.len() > 3
        {
            return Err(conflict(
                "Whole-App Backup import CAS or Release inventory is invalid",
            ));
        }
        let mut slots = BTreeSet::new();
        let mut release_ids = BTreeSet::new();
        for item in &params.releases {
            if !slots.insert(item.slot)
                || !release_ids.insert(item.release.release_id.clone())
            {
                return Err(conflict(
                    "Whole-App Backup import Release slots and identities must be unique",
                ));
            }
            validate_uuid(&item.artifact.artifact_id, "backup artifact_id")?;
            validate_uuid(&item.artifact.owner_user_id, "backup artifact owner")?;
            validate_uuid(&item.release.release_id, "backup release_id")?;
            validate_uuid(&item.release.plugin_product_id, "backup release plugin_product_id")?;
            validate_uuid(&item.release.owner_user_id, "backup release owner")?;
            validate_uuid(&item.release.origin_operation_id, "backup release operation")?;
            validate_digest(&item.release.release_digest, "backup release digest")?;
            if item.release.origin_kind != "import"
                || item.release.plugin_product_id != params.plugin_product_id
                || item.release.owner_user_id != params.owner_user_id
                || item.release.origin_operation_id != params.operation_id
                || item.artifact.owner_user_id != params.owner_user_id
            {
                return Err(conflict(
                    "Whole-App Backup Release must bind the target owner and Import operation",
                ));
            }
        }

        let mut kv_keys = BTreeSet::new();
        for row in &params.kv {
            if row.owner_user_id != params.owner_user_id
                || row.plugin_product_id != params.plugin_product_id
                || row.namespace.starts_with("service-test:")
                || !kv_keys.insert((row.namespace.clone(), row.key.clone()))
            {
                return Err(conflict(
                    "Whole-App Backup KV rows must be target-scoped, non-transient, and unique",
                ));
            }
            validate_visible_ascii_key(&row.namespace, "Plugin KV namespace", 128)?;
            validate_visible_ascii_key(&row.key, "Plugin KV key", 256)?;
            validate_kv_row(row)?;
            let value: serde_json::Value = serde_json::from_str(&row.value_json)
                .map_err(|error| {
                    conflict(format!("Whole-App Backup KV value is invalid: {error}"))
                })?;
            let canonical = String::from_utf8(
                canonical_json_bytes(&value).map_err(|error| {
                    conflict(format!("Whole-App Backup KV cannot be canonicalized: {error}"))
                })?,
            )
            .map_err(|error| {
                conflict(format!("Whole-App Backup KV canonical JSON is not UTF-8: {error}"))
            })?;
            if canonical != row.value_json {
                return Err(conflict(
                    "Whole-App Backup KV values must use canonical JSON",
                ));
            }
            if row.created_at < 0
                || row.updated_at < row.created_at
                || row.updated_at > params.finished_at_ms
            {
                return Err(conflict(
                    "Whole-App Backup KV timestamps are invalid",
                ));
            }
        }

        let mut tx = self.pool.begin().await?;
        let product =
            lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        if product.lifecycle != "disabled"
            || product.product_revision != params.expected_product_revision
            || product.pointer_revision != params.expected_pointer_revision
            || product.ready_release_id.is_some()
            || product.active_release_id.is_some()
            || product.previous_release_id.is_some()
        {
            return Err(conflict(
                "Whole-App Backup import requires an exact empty disabled Product",
            ));
        }
        let library_revision: i64 = sqlx::query_scalar(
            "SELECT revision FROM plugin_library_state WHERE owner_user_id = ?",
        )
        .bind(&params.owner_user_id)
        .fetch_one(&mut *tx)
        .await?;
        if library_revision != params.expected_library_revision {
            return Err(conflict(
                "Whole-App Backup import lost its exact Library revision",
            ));
        }
        let project = fetch_project(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
            &params.project_id,
        )
        .await?;
        if project.project_revision != params.expected_project_revision {
            return Err(conflict(
                "Whole-App Backup import lost its exact Project revision",
            ));
        }
        let operation =
            fetch_plugin_operation_in_tx(&mut tx, &params.plugin_product_id, &params.operation_id)
                .await?
                .ok_or_else(|| {
                    DbError::NotFound(format!(
                        "Plugin Backup import operation {}",
                        params.operation_id
                    ))
                })?;
        if operation.kind != "import"
            || operation.state != ProductOperationState::Running.as_str()
            || operation.progress_percent.is_none()
            || operation.last_error_code.is_some()
            || operation.finished_at_ms.is_some()
            || params.finished_at_ms < operation.started_at_ms
        {
            return Err(conflict(
                "Whole-App Backup import lost its exact running Operation",
            ));
        }

        let mut persisted = Vec::with_capacity(params.releases.len());
        for item in &params.releases {
            let artifact_payload = validate_artifact(&item.artifact)?;
            validate_product_artifact_contract(&product.kind, &artifact_payload)?;
            let artifact = persist_or_reuse_artifact(&mut tx, &item.artifact).await?;
            let mut release = item.release.clone();
            release.artifact_id = artifact.artifact_id.clone();
            release.manifest_digest = artifact.manifest_digest.clone();
            if item.release.artifact_id != artifact.artifact_id {
                release.release_record_json = rewrite_release_record_artifact_id(
                    &release.release_record_json,
                    &artifact.artifact_id,
                )?;
            }
            let persisted_payload = validate_artifact(&artifact)?;
            validate_release(&release, &persisted_payload)?;
            match release.source_kind.as_str() {
                "managed"
                    if release.project_id.as_deref() == Some(params.project_id.as_str())
                        && release.source_snapshot_digest.is_some()
                        && release.dependency_lock_digest.is_some()
                        && release.build_profile_version.as_deref()
                            == Some(PLUGIN_RELEASE_PROFILE_VERSION)
                        && release.build_generation.is_some_and(|value| value > 0) => {}
                "runtime_only"
                    if release.project_id.is_none()
                        && release.source_snapshot_digest.is_none()
                        && release.dependency_lock_digest.is_none()
                        && release.build_profile_version.is_none()
                        && release.build_generation.is_none() => {}
                _ => {
                    return Err(conflict(
                        "Whole-App Backup Release lineage does not bind the target Project",
                    ));
                }
            }
            if release.created_at < operation.started_at_ms
                || release.created_at > params.finished_at_ms
            {
                return Err(conflict(
                    "Whole-App Backup Release timestamp is outside Import operation",
                ));
            }
            sqlx::query(
                "INSERT INTO plugin_releases
                 (release_id, plugin_product_id, owner_user_id, artifact_id,
                  artifact_digest, manifest_digest, release_digest, origin_kind,
                  origin_operation_id, source_kind, project_id, source_snapshot_digest,
                  dependency_lock_digest, build_profile_version, build_generation,
                  release_record_json, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, 'import', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&release.release_id)
            .bind(&release.plugin_product_id)
            .bind(&release.owner_user_id)
            .bind(&release.artifact_id)
            .bind(&release.artifact_digest)
            .bind(&release.manifest_digest)
            .bind(&release.release_digest)
            .bind(&release.origin_operation_id)
            .bind(&release.source_kind)
            .bind(&release.project_id)
            .bind(&release.source_snapshot_digest)
            .bind(&release.dependency_lock_digest)
            .bind(&release.build_profile_version)
            .bind(release.build_generation)
            .bind(&release.release_record_json)
            .bind(release.created_at)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?;
            persisted.push((item.slot, release));
        }
        for row in &params.kv {
            sqlx::query(
                "INSERT INTO plugin_kv (
                    plugin_product_id, owner_user_id, namespace, key, value_json,
                    revision, key_generation, is_tombstone, created_at, updated_at
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&row.plugin_product_id)
            .bind(&row.owner_user_id)
            .bind(&row.namespace)
            .bind(&row.key)
            .bind(&row.value_json)
            .bind(row.revision)
            .bind(row.key_generation)
            .bind(row.is_tombstone)
            .bind(row.created_at)
            .bind(row.updated_at)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?;
        }
        let pointer = |slot: PluginRuntimeBackupReleaseSlot| {
            persisted
                .iter()
                .find(|(candidate, _)| *candidate == slot)
                .map(|(_, release)| (release.release_id.clone(), release.release_digest.clone()))
        };
        let ready = pointer(PluginRuntimeBackupReleaseSlot::Ready);
        let active = pointer(PluginRuntimeBackupReleaseSlot::Active);
        let previous = pointer(PluginRuntimeBackupReleaseSlot::Previous);
        let changed = sqlx::query(
            "UPDATE plugin_products
             SET product_revision = product_revision + 1,
                 pointer_revision = pointer_revision + 1,
                 active_release_epoch = ?,
                 ready_release_id = ?, ready_release_digest = ?,
                 active_release_id = ?, active_release_digest = ?,
                 previous_release_id = ?, previous_release_digest = ?,
                 materialized_catalog_digest = ?, updated_at = ?
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND lifecycle = 'disabled'
               AND product_revision = ? AND pointer_revision = ?
               AND ready_release_id IS NULL AND active_release_id IS NULL
               AND previous_release_id IS NULL AND updated_at <= ?",
        )
        .bind(if active.is_some() { 1_i64 } else { 0_i64 })
        .bind(ready.as_ref().map(|value| &value.0))
        .bind(ready.as_ref().map(|value| &value.1))
        .bind(active.as_ref().map(|value| &value.0))
        .bind(active.as_ref().map(|value| &value.1))
        .bind(previous.as_ref().map(|value| &value.0))
        .bind(previous.as_ref().map(|value| &value.1))
        .bind(&params.target_catalog_digest)
        .bind(params.finished_at_ms)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(params.expected_product_revision)
        .bind(params.expected_pointer_revision)
        .bind(params.finished_at_ms)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if changed.rows_affected() != 1 {
            return Err(conflict(
                "Whole-App Backup import Product/pointer CAS failed",
            ));
        }
        let library_updated = sqlx::query(
            "UPDATE plugin_library_state
             SET revision = revision + 1, updated_at = ?
             WHERE owner_user_id = ? AND revision = ? AND updated_at <= ?",
        )
        .bind(params.finished_at_ms)
        .bind(&params.owner_user_id)
        .bind(params.expected_library_revision)
        .bind(params.finished_at_ms)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if library_updated.rows_affected() != 1 {
            return Err(conflict(
                "Whole-App Backup import Library CAS failed",
            ));
        }
        let operation_updated = sqlx::query(
            "UPDATE product_operations
             SET state = 'succeeded', progress_percent = 100,
                 last_error_code = NULL, bounded_log_tail_json = ?,
                 finished_at_ms = ?
             WHERE operation_id = ? AND kind = 'import'
               AND owner_kind = 'plugin' AND owner_id = ? AND state = 'running'",
        )
        .bind(serialize_product_operation_log_tail(&[
            "Plugin Whole-App Backup import committed".to_owned(),
        ])?)
        .bind(params.finished_at_ms)
        .bind(&params.operation_id)
        .bind(&params.plugin_product_id)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if operation_updated.rows_affected() != 1 {
            return Err(conflict(
                "Whole-App Backup import Operation completion CAS failed",
            ));
        }
        let snapshot =
            fetch_snapshot_in_tx(&mut tx, &params.owner_user_id, &params.plugin_product_id)
                .await?
                .ok_or_else(|| DbError::Init("Whole-App Backup import lost Product".into()))?;
        tx.commit().await?;
        Ok(snapshot)
    }

    async fn finish_export_operation(
        &self,
        params: &FinishPluginRuntimeExportOperationParams,
    ) -> Result<ProductOperationRow, DbError> {
        terminalize_export_operation(
            &self.pool,
            &params.owner_user_id,
            &params.plugin_product_id,
            &params.operation_id,
            ProductOperationState::Succeeded,
            Some(100),
            None,
            &params.bounded_log_tail,
            params.finished_at_ms,
        )
        .await
    }

    async fn fail_export_operation(
        &self,
        params: &FailPluginRuntimeExportOperationParams,
    ) -> Result<ProductOperationRow, DbError> {
        terminalize_export_operation(
            &self.pool,
            &params.owner_user_id,
            &params.plugin_product_id,
            &params.operation_id,
            ProductOperationState::Failed,
            Some(params.progress_percent),
            Some(&params.error_code),
            &params.bounded_log_tail,
            params.finished_at_ms,
        )
        .await
    }

    async fn cancel_export_operation(
        &self,
        params: &CancelPluginRuntimeExportOperationParams,
    ) -> Result<ProductOperationRow, DbError> {
        terminalize_export_operation(
            &self.pool,
            &params.owner_user_id,
            &params.plugin_product_id,
            &params.operation_id,
            ProductOperationState::Canceled,
            None,
            None,
            &params.bounded_log_tail,
            params.finished_at_ms,
        )
        .await
    }

    async fn start_build_operation(
        &self,
        params: &StartPluginRuntimeBuildOperationParams,
    ) -> Result<ProductOperationRow, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&params.project_id, "project_id")?;
        validate_uuid(&params.operation_id, "operation_id")?;
        if params.expected_project_revision < 1 || params.started_at_ms <= 0 {
            return Err(conflict(
                "Plugin Build start revision/timestamp is invalid",
            ));
        }
        validate_build_source(&params.expected_source)?;
        let bounded_log_tail_json =
            serialize_product_operation_log_tail(&params.bounded_log_tail)?;

        let mut tx = self.pool.begin().await?;
        let product =
            lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        if matches!(product.lifecycle.as_str(), "trashed" | "deleting") {
            return Err(conflict(
                "Plugin Build requires a non-trashed product",
            ));
        }
        let project = fetch_project(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
            &params.project_id,
        )
        .await?;
        if !project_matches_source(
            &project,
            params.expected_project_revision,
            &params.expected_source,
        ) {
            return Err(conflict(
                "Plugin Build source/project CAS does not match the exact Project head",
            ));
        }
        if params.started_at_ms < project.updated_at || params.started_at_ms < product.created_at {
            return Err(conflict(
                "Plugin Build start timestamp predates the captured Project/Product state",
            ));
        }
        let running_operation: Option<String> = sqlx::query_scalar(
            "SELECT operation_id
             FROM product_operations
             WHERE owner_kind = 'plugin' AND owner_id = ? AND kind = 'build'
               AND state = 'running'
             ORDER BY started_at_ms ASC, operation_id ASC
             LIMIT 1",
        )
        .bind(&params.plugin_product_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(running_operation) = running_operation {
            return Err(conflict(format!(
                "Plugin already has a running Build operation {running_operation}"
            )));
        }
        sqlx::query(
            "INSERT INTO product_operations (
                operation_id, kind, owner_kind, owner_id, state,
                progress_percent, bounded_log_tail_json, started_at_ms
             ) VALUES (?, 'build', 'plugin', ?, 'running', 0, ?, ?)",
        )
        .bind(&params.operation_id)
        .bind(&params.plugin_product_id)
        .bind(bounded_log_tail_json)
        .bind(params.started_at_ms)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        sqlx::query(
            "INSERT INTO plugin_build_operation_lineage (
                operation_id, owner_user_id, plugin_product_id, project_id,
                project_revision, source_snapshot_digest,
                dependency_lock_digest, build_profile_version,
                build_generation, started_at_ms
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&params.operation_id)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(&params.project_id)
        .bind(params.expected_project_revision)
        .bind(&params.expected_source.source_head_digest)
        .bind(&params.expected_source.dependency_lock_digest)
        .bind(&params.expected_source.build_profile_version)
        .bind(params.expected_source.build_generation)
        .bind(params.started_at_ms)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let operation = sqlx::query_as::<_, ProductOperationRow>(
            "SELECT * FROM product_operations
             WHERE operation_id = ? AND kind = 'build'
               AND owner_kind = 'plugin' AND owner_id = ?",
        )
        .bind(&params.operation_id)
        .bind(&params.plugin_product_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(DbError::Query)?;
        tx.commit().await?;
        Ok(operation)
    }

    async fn finish_build_operation(
        &self,
        params: &FinishPluginRuntimeBuildOperationParams,
    ) -> Result<ProductOperationRow, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&params.operation_id, "operation_id")?;
        if params.finished_at_ms <= 0 {
            return Err(conflict(
                "Plugin Build finish timestamp must be positive",
            ));
        }
        validate_build_finish_shape(
            params.state,
            params.progress_percent,
            params.last_error_code.as_deref(),
        )?;
        let bounded_log_tail_json =
            serialize_product_operation_log_tail(&params.bounded_log_tail)?;

        let mut tx = self.pool.begin().await?;
        lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        let current = fetch_build_operation(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
            &params.operation_id,
        )
        .await?
        .ok_or_else(|| {
            DbError::NotFound(format!(
                "Plugin Build operation {}",
                params.operation_id
            ))
        })?;
        if current.state != ProductOperationState::Running.as_str() {
            return Err(conflict(format!(
                "Plugin Build operation {} is already terminal",
                params.operation_id
            )));
        }
        if params.finished_at_ms < current.started_at_ms {
            return Err(conflict(
                "Plugin Build terminal timestamp predates started_at_ms",
            ));
        }
        let updated = sqlx::query(
            "UPDATE product_operations
             SET state = ?, progress_percent = ?, last_error_code = ?,
                 bounded_log_tail_json = ?, finished_at_ms = ?
             WHERE operation_id = ? AND kind = 'build'
               AND owner_kind = 'plugin' AND owner_id = ? AND state = 'running'",
        )
        .bind(params.state.as_str())
        .bind(i64::from(params.progress_percent))
        .bind(&params.last_error_code)
        .bind(bounded_log_tail_json)
        .bind(params.finished_at_ms)
        .bind(&params.operation_id)
        .bind(&params.plugin_product_id)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if updated.rows_affected() != 1 {
            return Err(conflict(
                "Plugin Build finish lost its running-operation CAS",
            ));
        }
        let operation = sqlx::query_as::<_, ProductOperationRow>(
            "SELECT * FROM product_operations WHERE operation_id = ?",
        )
        .bind(&params.operation_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(DbError::Query)?;
        tx.commit().await?;
        Ok(operation)
    }

    async fn get_build_operation(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
        operation_id: &str,
    ) -> Result<Option<ProductOperationRow>, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        validate_uuid(plugin_product_id, "plugin_product_id")?;
        validate_uuid(operation_id, "operation_id")?;
        ensure_owner(&self.pool, owner_user_id).await?;
        let mut tx = self.pool.begin().await?;
        let operation =
            fetch_build_operation(&mut tx, owner_user_id, plugin_product_id, operation_id).await?;
        tx.commit().await?;
        Ok(operation)
    }

    async fn list_build_operations(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
    ) -> Result<Vec<ProductOperationRow>, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        validate_uuid(plugin_product_id, "plugin_product_id")?;
        ensure_owner(&self.pool, owner_user_id).await?;
        sqlx::query_as::<_, ProductOperationRow>(
            "SELECT operation.*
             FROM product_operations operation
             WHERE operation.owner_kind = 'plugin'
               AND operation.kind = 'build'
               AND operation.owner_id = ?
               AND EXISTS (
                   SELECT 1 FROM plugin_products product
                   WHERE product.plugin_product_id = operation.owner_id
                     AND product.owner_user_id = ?
               )
             ORDER BY operation.started_at_ms DESC, operation.operation_id DESC",
        )
        .bind(plugin_product_id)
        .bind(owner_user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(DbError::Query)
    }

    async fn cancel_build_operation(
        &self,
        params: &CancelPluginRuntimeBuildOperationParams,
    ) -> Result<ProductOperationRow, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&params.operation_id, "operation_id")?;
        if params.finished_at_ms <= 0 {
            return Err(conflict(
                "Plugin Build cancellation timestamp must be positive",
            ));
        }
        let bounded_log_tail_json =
            serialize_product_operation_log_tail(&params.bounded_log_tail)?;

        let mut tx = self.pool.begin().await?;
        lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        let current = fetch_build_operation(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
            &params.operation_id,
        )
        .await?
        .ok_or_else(|| {
            DbError::NotFound(format!(
                "Plugin Build operation {}",
                params.operation_id
            ))
        })?;
        if current.state != ProductOperationState::Running.as_str() {
            return Err(conflict(format!(
                "Plugin Build operation {} is already terminal",
                params.operation_id
            )));
        }
        if params.finished_at_ms < current.started_at_ms {
            return Err(conflict(
                "Plugin Build cancellation timestamp predates started_at_ms",
            ));
        }
        let updated = sqlx::query(
            "UPDATE product_operations
             SET state = 'canceled', progress_percent = ?,
                 last_error_code = NULL, bounded_log_tail_json = ?,
                 finished_at_ms = ?
             WHERE operation_id = ? AND kind = 'build'
               AND owner_kind = 'plugin' AND owner_id = ? AND state = 'running'",
        )
        .bind(current.progress_percent)
        .bind(bounded_log_tail_json)
        .bind(params.finished_at_ms)
        .bind(&params.operation_id)
        .bind(&params.plugin_product_id)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if updated.rows_affected() != 1 {
            return Err(conflict(
                "Plugin Build cancellation lost its running-operation CAS",
            ));
        }
        let operation = sqlx::query_as::<_, ProductOperationRow>(
            "SELECT * FROM product_operations WHERE operation_id = ?",
        )
        .bind(&params.operation_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(DbError::Query)?;
        tx.commit().await?;
        Ok(operation)
    }

    async fn finish_build_and_record_ready(
        &self,
        params: &FinishPluginRuntimeBuildAndRecordReadyParams,
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        validate_ready_binding(params)?;
        let bounded_log_tail_json =
            serialize_product_operation_log_tail(&params.bounded_log_tail)?;

        let mut tx = self.pool.begin().await?;
        let product =
            lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        if matches!(product.lifecycle.as_str(), "trashed" | "deleting") {
            return Err(conflict(
                "Plugin Build Ready requires a non-trashed product",
            ));
        }
        let artifact_payload = validate_artifact(&params.artifact)?;
        validate_product_artifact_contract(&product.kind, &artifact_payload)?;
        if product.product_revision < params.expected_product_revision
            || product.pointer_revision != params.expected_pointer_revision
        {
            return Err(conflict(
                "Plugin Build Ready product/pointer CAS failed",
            ));
        }
        let project = fetch_project(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
            &params.project_id,
        )
        .await?;
        if project.source_state != "editable"
            || project.project_revision != params.expected_project_revision
            || project.build_generation != params.expected_build_generation
            || project.build_profile_version.as_deref()
                != Some(PLUGIN_RELEASE_PROFILE_VERSION)
        {
            return Err(conflict(
                "Plugin Build Ready project/source CAS does not match the exact Project head",
            ));
        }
        if params.release.source_snapshot_digest != project.source_head_digest
            || params.release.dependency_lock_digest != project.dependency_lock_digest
            || params.release.build_profile_version != project.build_profile_version
        {
            return Err(conflict(
                "Plugin Build Ready release lineage differs from the exact Project head",
            ));
        }
        if [
            product.ready_release_id.as_deref(),
            product.active_release_id.as_deref(),
            product.previous_release_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|release_id| release_id == params.release.release_id)
        {
            return Err(conflict(
                "Plugin Build Ready must use a distinct Release identity",
            ));
        }
        if params.finished_at_ms < project.updated_at {
            return Err(conflict(
                "Plugin Build Ready timestamp predates the Project state",
            ));
        }
        let committed_at_ms = params.finished_at_ms.max(product.updated_at);
        let operation = fetch_build_operation(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
            &params.operation_id,
        )
        .await?
        .ok_or_else(|| {
            DbError::NotFound(format!(
                "Plugin Build operation {}",
                params.operation_id
            ))
        })?;
        if operation.state != ProductOperationState::Running.as_str() {
            return Err(conflict(format!(
                "Plugin Build operation {} is already terminal",
                params.operation_id
            )));
        }
        let lineage = fetch_build_lineage(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
            &params.operation_id,
        )
        .await?
        .ok_or_else(|| {
            DbError::Init(format!(
                "Plugin Build operation {} has no persisted source lineage",
                params.operation_id
            ))
        })?;
        if lineage.project_id != params.project_id
            || lineage.project_revision != params.expected_project_revision
            || params.release.source_snapshot_digest.as_deref()
                != Some(lineage.source_snapshot_digest.as_str())
            || params.release.dependency_lock_digest.as_deref()
                != Some(lineage.dependency_lock_digest.as_str())
            || params.release.build_profile_version.as_deref()
                != Some(lineage.build_profile_version.as_str())
            || lineage.build_generation != params.expected_build_generation
        {
            return Err(conflict(
                "Plugin Build Ready does not match the immutable start-time source lineage",
            ));
        }
        if params.finished_at_ms < operation.started_at_ms
            || params.artifact.created_at < operation.started_at_ms
            || params.release.created_at < operation.started_at_ms
        {
            return Err(conflict(
                "Plugin Build Ready artifact/release timestamps do not cover the operation",
            ));
        }
        let other_running: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM product_operations
             WHERE owner_kind = 'plugin' AND owner_id = ? AND kind = 'build'
               AND state = 'running' AND operation_id <> ?",
        )
        .bind(&params.plugin_product_id)
        .bind(&params.operation_id)
        .fetch_one(&mut *tx)
        .await?;
        if other_running != 0 {
            return Err(conflict(
                "Plugin Build Ready cannot commit while another Build is running",
            ));
        }

        let persisted_artifact =
            persist_or_reuse_artifact(&mut tx, &params.artifact).await?;
        let mut release = params.release.clone();
        release.artifact_id = persisted_artifact.artifact_id.clone();
        release.manifest_digest = persisted_artifact.manifest_digest.clone();
        if params.release.artifact_id != persisted_artifact.artifact_id {
            release.release_record_json = rewrite_release_record_artifact_id(
                &release.release_record_json,
                &persisted_artifact.artifact_id,
            )?;
        }
        let persisted_artifact_payload = validate_artifact(&persisted_artifact)?;
        validate_release(&release, &persisted_artifact_payload)?;
        sqlx::query(
            "INSERT INTO plugin_releases
             (release_id, plugin_product_id, owner_user_id, artifact_id,
              artifact_digest, manifest_digest, release_digest, origin_kind,
              origin_operation_id, source_kind, project_id, source_snapshot_digest,
              dependency_lock_digest, build_profile_version, build_generation,
              release_record_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&release.release_id)
        .bind(&release.plugin_product_id)
        .bind(&release.owner_user_id)
        .bind(&release.artifact_id)
        .bind(&release.artifact_digest)
        .bind(&release.manifest_digest)
        .bind(&release.release_digest)
        .bind(&release.origin_kind)
        .bind(&release.origin_operation_id)
        .bind(&release.source_kind)
        .bind(&release.project_id)
        .bind(&release.source_snapshot_digest)
        .bind(&release.dependency_lock_digest)
        .bind(&release.build_profile_version)
        .bind(release.build_generation)
        .bind(&release.release_record_json)
        .bind(release.created_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let pointer_updated = sqlx::query(
            "UPDATE plugin_products
             SET product_revision = product_revision + 1,
                 pointer_revision = pointer_revision + 1,
                 ready_release_id = ?, ready_release_digest = ?,
                 updated_at = ?
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND product_revision = ? AND pointer_revision = ?
               AND updated_at <= ?",
        )
        .bind(&release.release_id)
        .bind(&release.release_digest)
        .bind(committed_at_ms)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(product.product_revision)
        .bind(params.expected_pointer_revision)
        .bind(committed_at_ms)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if pointer_updated.rows_affected() != 1 {
            return Err(conflict(
                "Plugin Build Ready pointer CAS failed",
            ));
        }
        bump_library_revision(
            &mut tx,
            &params.owner_user_id,
            committed_at_ms,
        )
        .await?;
        let operation_updated = sqlx::query(
            "UPDATE product_operations
             SET state = 'succeeded', progress_percent = 100,
                 last_error_code = NULL, bounded_log_tail_json = ?,
                 finished_at_ms = ?
             WHERE operation_id = ? AND kind = 'build'
               AND owner_kind = 'plugin' AND owner_id = ? AND state = 'running'",
        )
        .bind(bounded_log_tail_json)
        .bind(committed_at_ms)
        .bind(&params.operation_id)
        .bind(&params.plugin_product_id)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if operation_updated.rows_affected() != 1 {
            return Err(conflict(
                "Plugin Build success lost its running-operation CAS",
            ));
        }
        let snapshot = fetch_snapshot_in_tx(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
        )
            .await?
            .ok_or_else(|| DbError::Init("Plugin Build Ready commit lost Product".into()))?;
        tx.commit().await?;
        Ok(snapshot)
    }

    async fn begin_source_mutation(
        &self,
        params: &BeginPluginSourceMutationParams,
    ) -> Result<PluginRuntimeSourceMutationIntentRow, DbError> {
        validate_uuid(&params.intent_id, "intent_id")?;
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&params.project_id, "project_id")?;
        validate_digest(&params.expected_source_digest, "expected_source_digest")?;
        validate_digest(&params.next_source_digest, "next_source_digest")?;
        if params.expected_product_revision < 1
            || params.expected_project_revision < 1
            || params.expected_build_generation < 1
            || params.next_build_generation != params.expected_build_generation + 1
            || params.expected_source_digest == params.next_source_digest
            || params.created_at < 1
        {
            return Err(conflict("Plugin Source mutation intent is invalid"));
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO plugin_source_mutation_intents (
                intent_id, owner_user_id, plugin_product_id, project_id,
                expected_product_revision, expected_project_revision,
                expected_build_generation, expected_source_digest,
                next_source_digest, next_build_generation, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&params.intent_id)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(&params.project_id)
        .bind(params.expected_product_revision)
        .bind(params.expected_project_revision)
        .bind(params.expected_build_generation)
        .bind(&params.expected_source_digest)
        .bind(&params.next_source_digest)
        .bind(params.next_build_generation)
        .bind(params.created_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let intent = sqlx::query_as::<_, PluginRuntimeSourceMutationIntentRow>(
            "SELECT * FROM plugin_source_mutation_intents WHERE intent_id = ?",
        )
        .bind(&params.intent_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(query_error)?;
        tx.commit().await?;
        Ok(intent)
    }

    async fn finalize_source_mutation(
        &self,
        params: &FinalizePluginSourceMutationParams,
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        validate_uuid(&params.intent_id, "intent_id")?;
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&params.project_id, "project_id")?;
        if params.updated_at < 1 {
            return Err(conflict("Plugin Source mutation timestamp is invalid"));
        }
        let mut tx = self.pool.begin().await?;
        let intent = sqlx::query_as::<_, PluginRuntimeSourceMutationIntentRow>(
            "SELECT * FROM plugin_source_mutation_intents
             WHERE intent_id = ? AND owner_user_id = ?
               AND plugin_product_id = ? AND project_id = ?",
        )
        .bind(&params.intent_id)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(&params.project_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(query_error)?
        .ok_or_else(|| conflict("Plugin Source mutation intent is unavailable"))?;
        sqlx::query(
            "INSERT INTO plugin_source_mutation_commits
                (project_id, intent_id, created_at)
             VALUES (?, ?, ?)",
        )
        .bind(&params.project_id)
        .bind(&params.intent_id)
        .bind(params.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let changed = sqlx::query(
            "UPDATE plugin_projects
             SET project_revision = project_revision + 1,
                 source_head_digest = ?, build_generation = ?, updated_at = ?
             WHERE owner_user_id = ? AND plugin_product_id = ? AND project_id = ?
               AND project_revision = ? AND build_generation = ?
               AND source_head_digest = ? AND updated_at < ?",
        )
        .bind(&intent.next_source_digest)
        .bind(intent.next_build_generation)
        .bind(params.updated_at)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(&params.project_id)
        .bind(intent.expected_project_revision)
        .bind(intent.expected_build_generation)
        .bind(&intent.expected_source_digest)
        .bind(params.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if changed.rows_affected() != 1 {
            return Err(conflict("Plugin Source mutation lost its Project CAS"));
        }
        let deleted = sqlx::query(
            "DELETE FROM plugin_source_mutation_intents
             WHERE intent_id = ? AND owner_user_id = ?
               AND plugin_product_id = ? AND project_id = ?",
        )
        .bind(&params.intent_id)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(&params.project_id)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if deleted.rows_affected() != 1 {
            return Err(conflict("Plugin Source mutation intent cleanup lost its CAS"));
        }
        let marker_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM plugin_source_mutation_commits
             WHERE project_id = ? OR intent_id = ?",
        )
        .bind(&params.project_id)
        .bind(&params.intent_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(query_error)?;
        if marker_count != 0 {
            return Err(conflict("Plugin Source commit marker was not consumed"));
        }
        bump_library_revision(&mut tx, &params.owner_user_id, params.updated_at).await?;
        let snapshot = fetch_snapshot_in_tx(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
        )
        .await?
        .ok_or_else(|| conflict("Plugin Source mutation lost its Product"))?;
        tx.commit().await?;
        Ok(snapshot)
    }

    async fn abort_source_mutation(
        &self,
        params: &AbortPluginSourceMutationParams,
    ) -> Result<(), DbError> {
        validate_uuid(&params.intent_id, "intent_id")?;
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&params.project_id, "project_id")?;
        let deleted = sqlx::query(
            "DELETE FROM plugin_source_mutation_intents
             WHERE intent_id = ? AND owner_user_id = ?
               AND plugin_product_id = ? AND project_id = ?
               AND EXISTS (
                   SELECT 1 FROM plugin_projects project
                    WHERE project.owner_user_id = plugin_source_mutation_intents.owner_user_id
                      AND project.plugin_product_id = plugin_source_mutation_intents.plugin_product_id
                      AND project.project_id = plugin_source_mutation_intents.project_id
                      AND project.project_revision = plugin_source_mutation_intents.expected_project_revision
                      AND project.build_generation = plugin_source_mutation_intents.expected_build_generation
                      AND project.source_head_digest = plugin_source_mutation_intents.expected_source_digest
               )",
        )
        .bind(&params.intent_id)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(&params.project_id)
        .execute(&self.pool)
        .await
        .map_err(query_error)?;
        if deleted.rows_affected() != 1 {
            return Err(conflict("Plugin Source mutation abort lost its CAS"));
        }
        Ok(())
    }

    async fn list_source_mutation_intents(
        &self,
    ) -> Result<Vec<PluginRuntimeSourceMutationIntentRow>, DbError> {
        sqlx::query_as(
            "SELECT * FROM plugin_source_mutation_intents
             ORDER BY created_at ASC, intent_id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(query_error)
    }

    async fn get_source_mutation_intent(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
        project_id: &str,
    ) -> Result<Option<PluginRuntimeSourceMutationIntentRow>, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        validate_uuid(plugin_product_id, "plugin_product_id")?;
        validate_uuid(project_id, "project_id")?;
        sqlx::query_as(
            "SELECT * FROM plugin_source_mutation_intents
             WHERE owner_user_id = ? AND plugin_product_id = ? AND project_id = ?",
        )
        .bind(owner_user_id)
        .bind(plugin_product_id)
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(query_error)
    }

    async fn update_project_source_cas(
        &self,
        params: &UpdatePluginRuntimeProjectSourceParams,
    ) -> Result<PluginRuntimeProjectRow, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&params.project_id, "project_id")?;
        validate_project_source(
            params.source_state,
            params.managed_source_path.as_deref(),
            params.source_head_digest.as_deref(),
            params.dependency_lock_digest.as_deref(),
            params.build_profile_version.as_deref(),
            params.build_generation,
        )?;
        let mut tx = self.pool.begin().await?;
        let product =
            lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        if matches!(product.lifecycle.as_str(), "trashed" | "deleting") {
            return Err(conflict(
                "Plugin Project source requires a non-trashed product",
            ));
        }
        let current: PluginRuntimeProjectRow = sqlx::query_as(
            "SELECT * FROM plugin_projects
             WHERE owner_user_id = ? AND plugin_product_id = ? AND project_id = ?",
        )
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(&params.project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("Plugin Project {}", params.project_id)))?;
        if params.updated_at < current.updated_at {
            return Err(conflict(
                "Plugin Project source timestamp predates the current Project",
            ));
        }
        let running_operation: Option<String> = sqlx::query_scalar(
            "SELECT operation_id
             FROM product_operations
             WHERE owner_kind = 'plugin' AND owner_id = ? AND kind = 'build'
               AND state = 'running'
             LIMIT 1",
        )
        .bind(&params.plugin_product_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(operation_id) = running_operation {
            return Err(conflict(format!(
                "Plugin Project source cannot change while Build {operation_id} is running"
            )));
        }
        if params.source_state == PluginRuntimeProjectSourceState::Editable
            && params.build_generation <= current.build_generation
        {
            return Err(conflict(
                "Plugin Project build_generation must increase monotonically",
            ));
        }
        let changed = sqlx::query(
            "UPDATE plugin_projects
             SET project_revision = project_revision + 1,
                 source_state = ?, managed_source_path = ?,
                 source_head_digest = ?, dependency_lock_digest = ?,
                 build_profile_version = ?, build_generation = ?,
                 updated_at = ?
             WHERE owner_user_id = ? AND plugin_product_id = ? AND project_id = ?
               AND project_revision = ? AND updated_at <= ?",
        )
        .bind(params.source_state.as_str())
        .bind(&params.managed_source_path)
        .bind(&params.source_head_digest)
        .bind(&params.dependency_lock_digest)
        .bind(&params.build_profile_version)
        .bind(params.build_generation)
        .bind(params.updated_at)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(&params.project_id)
        .bind(params.expected_project_revision)
        .bind(params.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if changed.rows_affected() != 1 {
            return Err(DbError::Conflict(
                "Plugin Project source CAS failed".to_owned(),
            ));
        }
        bump_library_revision(
            &mut tx,
            &params.owner_user_id,
            params.updated_at,
        )
        .await?;
        let project = sqlx::query_as(
            "SELECT * FROM plugin_projects
             WHERE owner_user_id = ? AND project_id = ?",
        )
        .bind(&params.owner_user_id)
        .bind(&params.project_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(DbError::Query)?;
        tx.commit().await?;
        Ok(project)
    }

    async fn record_ready_release(
        &self,
        params: &RecordPluginRuntimeReadyReleaseParams,
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        let artifact_payload = validate_artifact(&params.artifact)?;
        validate_release(&params.release, &artifact_payload)?;
        if params.release.origin_kind == "build" {
            return Err(conflict(
                "built Plugin Ready Releases must use finish_build_and_record_ready",
            ));
        }
        if params.release.owner_user_id != params.owner_user_id
            || params.release.plugin_product_id != params.plugin_product_id
            || params.release.project_id.as_deref() != Some(params.project_id.as_str())
            || params.release.artifact_id != params.artifact.artifact_id
            || params.release.artifact_digest != params.artifact.artifact_digest
            || params.release.manifest_digest != params.artifact.manifest_digest
            || params.release.release_digest != params.artifact.artifact_digest
        {
            return Err(DbError::Conflict(
                "Plugin Ready Release does not bind its exact owner/Product/Project/Artifact"
                    .to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        let product = lock_product(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        validate_product_artifact_contract(&product.kind, &artifact_payload)?;
        if product.product_revision != params.expected_product_revision
            || product.pointer_revision != params.expected_pointer_revision
        {
            return Err(DbError::Conflict(
                "Plugin Ready Release product CAS failed".to_owned(),
            ));
        }
        let project: PluginRuntimeProjectRow = sqlx::query_as(
            "SELECT * FROM plugin_projects
             WHERE owner_user_id = ? AND plugin_product_id = ? AND project_id = ?",
        )
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(&params.project_id)
        .fetch_one(&mut *tx)
        .await?;
        if project.project_revision != params.expected_project_revision
            || project.build_generation != params.expected_build_generation
        {
            return Err(DbError::Conflict(
                "Plugin Ready Release project CAS failed".to_owned(),
            ));
        }
        if params.release.build_generation
            != Some(project.build_generation)
            || params.release.source_snapshot_digest
                != project.source_head_digest
            || params.release.dependency_lock_digest
                != project.dependency_lock_digest
            || params.release.build_profile_version
                != project.build_profile_version
        {
            return Err(DbError::Conflict(
                "Plugin Ready Release source lineage differs from the exact Project head"
                    .to_owned(),
            ));
        }
        let operation: Option<(String, String, String, String)> =
            sqlx::query_as(
                "SELECT kind, owner_kind, owner_id, state
                 FROM product_operations WHERE operation_id = ?",
            )
            .bind(&params.release.origin_operation_id)
            .fetch_optional(&mut *tx)
            .await?;
        let expected_kind = params.release.origin_kind.as_str();
        if operation.as_ref().is_none_or(
            |(kind, owner_kind, owner_id, state)| {
                kind != expected_kind
                    || owner_kind != "plugin"
                    || owner_id != &params.plugin_product_id
                    || state != "succeeded"
            },
        ) {
            return Err(DbError::Conflict(
                "Plugin Ready Release requires an exact successful owner Operation"
                    .to_owned(),
            ));
        }
        let persisted_artifact =
            persist_or_reuse_artifact(&mut tx, &params.artifact).await?;
        let mut release = params.release.clone();
        release.artifact_id = persisted_artifact.artifact_id.clone();
        release.manifest_digest = persisted_artifact.manifest_digest.clone();
        if params.release.artifact_id != persisted_artifact.artifact_id {
            release.release_record_json = rewrite_release_record_artifact_id(
                &release.release_record_json,
                &persisted_artifact.artifact_id,
            )?;
        }
        let persisted_artifact_payload = validate_artifact(&persisted_artifact)?;
        validate_release(&release, &persisted_artifact_payload)?;
        sqlx::query(
            "INSERT INTO plugin_releases
             (release_id, plugin_product_id, owner_user_id, artifact_id,
              artifact_digest, manifest_digest, release_digest, origin_kind,
              origin_operation_id, source_kind, project_id, source_snapshot_digest,
              dependency_lock_digest, build_profile_version, build_generation,
              release_record_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&release.release_id)
        .bind(&release.plugin_product_id)
        .bind(&release.owner_user_id)
        .bind(&release.artifact_id)
        .bind(&release.artifact_digest)
        .bind(&release.manifest_digest)
        .bind(&release.release_digest)
        .bind(&release.origin_kind)
        .bind(&release.origin_operation_id)
        .bind(&release.source_kind)
        .bind(&release.project_id)
        .bind(&release.source_snapshot_digest)
        .bind(&release.dependency_lock_digest)
        .bind(&release.build_profile_version)
        .bind(release.build_generation)
        .bind(&release.release_record_json)
        .bind(release.created_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let updated = sqlx::query(
            "UPDATE plugin_products
             SET product_revision = product_revision + 1,
                 pointer_revision = pointer_revision + 1,
                 ready_release_id = ?, ready_release_digest = ?,
                 updated_at = ?
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND product_revision = ? AND pointer_revision = ?
               AND updated_at <= ?",
        )
        .bind(&params.release.release_id)
        .bind(&params.release.release_digest)
        .bind(params.updated_at)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(params.expected_product_revision)
        .bind(params.expected_pointer_revision)
        .bind(params.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if updated.rows_affected() != 1 {
            return Err(DbError::Conflict(
                "Plugin Ready pointer CAS failed".to_owned(),
            ));
        }
        bump_library_revision(
            &mut tx,
            &params.owner_user_id,
            params.updated_at,
        )
        .await?;
        let snapshot = fetch_snapshot_in_tx(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
        )
            .await?
            .ok_or_else(|| DbError::Init("Plugin Ready commit lost Product".into()))?;
        tx.commit().await?;
        Ok(snapshot)
    }

    async fn record_service_test_receipt_cas(
        &self,
        params: &RecordPluginRuntimeServiceTestReceiptParams,
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&params.expected_ready_release_id, "expected_ready_release_id")?;
        validate_uuid(&params.receipt_id, "receipt_id")?;
        validate_digest(
            &params.expected_ready_release_digest,
            "expected_ready_release_digest",
        )?;
        validate_digest(&params.service_run_key, "service_run_key")?;
        validate_digest(&params.receipt_digest, "receipt_digest")?;
        validate_digest(
            &params.runtime_fingerprint_digest,
            "runtime_fingerprint_digest",
        )?;
        validate_digest(
            &params.resolved_test_input_digest,
            "resolved_test_input_digest",
        )?;
        validate_product_operation_error_code(params.error_code.as_deref())?;
        if params.expected_product_revision < 1
            || params.expected_pointer_revision < 1
            || params.expected_config_revision < 1
            || params.expected_credential_bindings_revision < 1
            || params.issued_at_ms <= 0
        {
            return Err(conflict(
                "Service Test receipt CAS expectations are invalid",
            ));
        }
        let (receipt_json, receipt) = canonical_service_test_receipt(params)?;
        let mut tx = self.pool.begin().await?;
        let product =
            lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        if matches!(product.lifecycle.as_str(), "trashed" | "deleting")
        {
            return Err(conflict(
                "Service Test receipt requires a non-trashed Service Plugin",
            ));
        }
        if product.product_revision != params.expected_product_revision
            || product.pointer_revision != params.expected_pointer_revision
            || product.config_revision != params.expected_config_revision
            || product.credential_bindings_revision
                != params.expected_credential_bindings_revision
            || product.ready_release_id.as_deref()
                != Some(params.expected_ready_release_id.as_str())
            || product.ready_release_digest.as_deref()
                != Some(params.expected_ready_release_digest.as_str())
        {
            return Err(conflict(
                "Service Test receipt lost its exact Product/Ready/config/credential CAS",
            ));
        }
        if params.issued_at_ms < product.updated_at {
            return Err(conflict(
                "Service Test receipt timestamp predates Product state",
            ));
        }
        let release = sqlx::query_as::<_, PluginRuntimeReleaseRow>(
            "SELECT * FROM plugin_releases
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND release_id = ? AND release_digest = ?",
        )
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(&params.expected_ready_release_id)
        .bind(&params.expected_ready_release_digest)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| conflict("Service Test receipt Ready Release is missing"))?;
        if params.issued_at_ms < release.created_at {
            return Err(conflict(
                "Service Test receipt timestamp predates Ready Release",
            ));
        }
        let artifact = sqlx::query_as::<_, PluginRuntimeReleaseArtifactRow>(
            "SELECT * FROM plugin_release_artifacts
             WHERE owner_user_id = ? AND artifact_id = ?",
        )
        .bind(&params.owner_user_id)
        .bind(&release.artifact_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| conflict("Service Test receipt Release Artifact is missing"))?;
        let artifact_payload = validate_artifact(&artifact)?;
        if artifact_payload.manifest.payload.service.is_none() {
            return Err(conflict(
                "Service Test receipt cannot bind a UI-only Ready Release",
            ));
        }
        let mut ready = validate_release(&release, &artifact_payload)?;
        if ready.plugin_product_id != receipt.plugin_product_id || ready.release != receipt.release {
            return Err(conflict(
                "Service Test receipt does not bind the exact typed Ready Release",
            ));
        }
        ready.matching_service_test_receipt = Some(receipt.reference());
        ready
            .validate_for_artifact(&artifact_payload)
            .map_err(|error| conflict(format!("Service Test Ready reference is invalid: {error}")))?;
        let ready_json = String::from_utf8(canonical_json_bytes(&ready).map_err(|error| {
            conflict(format!(
                "Service Test Ready Release cannot be canonicalized: {error}"
            ))
        })?)
        .map_err(|error| {
            conflict(format!(
                "Service Test Ready Release canonical JSON is not UTF-8: {error}"
            ))
        })?;

        sqlx::query(
            "INSERT INTO plugin_service_test_receipts (
                receipt_id, owner_user_id, plugin_product_id, release_id, release_digest,
                service_run_key, outcome, error_code, receipt_digest,
                runtime_fingerprint_digest, resolved_test_input_digest,
                tested_product_revision, tested_pointer_revision,
                tested_config_revision, tested_credential_bindings_revision,
                receipt_json, issued_at_ms
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&params.receipt_id)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(&params.expected_ready_release_id)
        .bind(&params.expected_ready_release_digest)
        .bind(&params.service_run_key)
        .bind(service_test_outcome(params.outcome))
        .bind(&params.error_code)
        .bind(&params.receipt_digest)
        .bind(&params.runtime_fingerprint_digest)
        .bind(&params.resolved_test_input_digest)
        .bind(params.expected_product_revision)
        .bind(params.expected_pointer_revision)
        .bind(params.expected_config_revision)
        .bind(params.expected_credential_bindings_revision)
        .bind(&receipt_json)
        .bind(params.issued_at_ms)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let release_updated = sqlx::query(
            "UPDATE plugin_releases
             SET release_record_json = ?
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND release_id = ? AND release_digest = ?
               AND release_record_json = ?",
        )
        .bind(&ready_json)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(&params.expected_ready_release_id)
        .bind(&params.expected_ready_release_digest)
        .bind(&release.release_record_json)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if release_updated.rows_affected() != 1 {
            return Err(conflict(
                "Service Test receipt lost the Ready Release record CAS",
            ));
        }
        let product_updated = sqlx::query(
            "UPDATE plugin_products
             SET product_revision = product_revision + 1, updated_at = ?
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND product_revision = ? AND pointer_revision = ?
               AND config_revision = ? AND credential_bindings_revision = ?
               AND ready_release_id = ? AND ready_release_digest = ?
               AND updated_at <= ?",
        )
        .bind(params.issued_at_ms)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(params.expected_product_revision)
        .bind(params.expected_pointer_revision)
        .bind(params.expected_config_revision)
        .bind(params.expected_credential_bindings_revision)
        .bind(&params.expected_ready_release_id)
        .bind(&params.expected_ready_release_digest)
        .bind(params.issued_at_ms)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if product_updated.rows_affected() != 1 {
            return Err(conflict(
                "Service Test receipt lost the Product revision CAS",
            ));
        }
        bump_library_revision(&mut tx, &params.owner_user_id, params.issued_at_ms).await?;
        let snapshot = fetch_snapshot_in_tx(&mut tx, &params.owner_user_id, &params.plugin_product_id)
            .await?
            .ok_or_else(|| DbError::Init("Service Test receipt commit lost Product".into()))?;
        tx.commit().await?;
        Ok(snapshot)
    }

    async fn get_ready_service_test_receipt(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
    ) -> Result<Option<PluginRuntimeServiceTestReceiptRow>, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        validate_uuid(plugin_product_id, "plugin_product_id")?;
        ensure_owner(&self.pool, owner_user_id).await?;
        let mut tx = self.pool.begin().await?;
        let Some(product) = fetch_owned_product_in_tx(&mut tx, owner_user_id, plugin_product_id).await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        let (Some(ready_release_id), Some(ready_release_digest)) = (
            product.ready_release_id.as_deref(),
            product.ready_release_digest.as_deref(),
        ) else {
            tx.commit().await?;
            return Ok(None);
        };
        let release: Option<PluginRuntimeReleaseRow> = sqlx::query_as(
            "SELECT * FROM plugin_releases
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND release_id = ? AND release_digest = ?",
        )
        .bind(owner_user_id)
        .bind(plugin_product_id)
        .bind(ready_release_id)
        .bind(ready_release_digest)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(release) = release else {
            return Err(DbError::Init(
                "Ready Service Test receipt lookup lost its Ready Release".to_owned(),
            ));
        };
        let ready: PluginReadyRelease = serde_json::from_str(&release.release_record_json)
            .map_err(|error| {
                DbError::Init(format!(
                    "Ready Service Test receipt lookup found invalid Release JSON: {error}"
                ))
            })?;
        let Some(reference) = ready.matching_service_test_receipt else {
            tx.commit().await?;
            return Ok(None);
        };
        let row = sqlx::query_as::<_, PluginRuntimeServiceTestReceiptRow>(
            "SELECT * FROM plugin_service_test_receipts
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND receipt_id = ? AND release_id = ? AND release_digest = ?
               AND service_run_key = ?",
        )
        .bind(owner_user_id)
        .bind(plugin_product_id)
        .bind(reference.receipt_id.as_ref())
        .bind(reference.release_id.as_ref())
        .bind(reference.release_digest.as_ref())
        .bind(reference.service_run_key.as_ref())
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            return Err(DbError::Init(
                "Ready Release references a missing Service Test receipt".to_owned(),
            ));
        };
        validate_persisted_service_test_receipt(&row)?;
        let matching = row
            .tested_product_revision
            .checked_add(1)
            .is_some_and(|revision| revision == product.product_revision)
            && row.tested_pointer_revision == product.pointer_revision
            && row.tested_config_revision == product.config_revision
            && row.tested_credential_bindings_revision
                == product.credential_bindings_revision
            && row.release_id == ready_release_id
            && row.release_digest == ready_release_digest;
        tx.commit().await?;
        Ok(matching.then_some(row))
    }

    async fn publish_ready_cas(
        &self,
        params: &PublishPluginRuntimeReadyParams,
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(
            &params.expected_ready_release_id,
            "expected_ready_release_id",
        )?;
        validate_digest(
            &params.expected_ready_release_digest,
            "expected_ready_release_digest",
        )?;
        validate_optional_digest(
            params.expected_active_release_digest.as_deref(),
            "expected_active_release_digest",
        )?;
        validate_digest(&params.target_catalog_digest, "target_catalog_digest")?;
        if let Some(guard) = &params.auto_publish_guard {
            validate_auto_publish_guard(guard)?;
        }
        if params.expected_product_revision < 1
            || params.expected_pointer_revision < 1
            || params.expected_active_release_epoch < 0
            || params.updated_at <= 0
        {
            return Err(conflict("Plugin Publish CAS expectations are invalid"));
        }

        let mut tx = self.pool.begin().await?;
        let current =
            lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        if matches!(current.lifecycle.as_str(), "trashed" | "deleting") {
            return Err(conflict(
                "Plugin Publish requires a non-trashed product",
            ));
        }
        if current.product_revision != params.expected_product_revision
            || current.pointer_revision != params.expected_pointer_revision
            || current.active_release_epoch != params.expected_active_release_epoch
            || current.ready_release_id.as_deref()
                != Some(params.expected_ready_release_id.as_str())
            || current.ready_release_digest.as_deref()
                != Some(params.expected_ready_release_digest.as_str())
            || current.active_release_digest.as_deref()
                != params.expected_active_release_digest.as_deref()
        {
            return Err(conflict(
                "Plugin Publish lost its exact Ready/Active pointer CAS",
            ));
        }
        if current.active_release_id.as_deref()
            == Some(params.expected_ready_release_id.as_str())
            || current.previous_release_id.as_deref()
                == Some(params.expected_ready_release_id.as_str())
        {
            return Err(conflict(
                "Plugin Publish target must use a distinct Release identity",
            ));
        }
        if params.updated_at < current.updated_at {
            return Err(conflict("Plugin Publish timestamp predates Product state"));
        }
        ensure_no_running_build(&mut tx, &params.plugin_product_id).await?;
        require_pointer_release(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
            Some(&params.expected_ready_release_id),
            Some(&params.expected_ready_release_digest),
            "Ready Release",
        )
        .await?;
        if let Some(guard) = &params.auto_publish_guard {
            let authorization =
                sqlx::query_as::<_, PluginRuntimePublishAuthorizationRow>(
                    "SELECT * FROM plugin_publish_authorizations
                     WHERE owner_user_id = ? AND plugin_product_id = ?",
                )
                .bind(&params.owner_user_id)
                .bind(&params.plugin_product_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| {
                    conflict("Plugin auto Publish authorization was revoked")
                })?;
            if !authorization.enabled
                || authorization.authorization_id != guard.authorization_id
                || authorization.revision != guard.authorization_revision
            {
                return Err(conflict(
                    "Plugin auto Publish authorization revision changed",
                ));
            }
            let project = fetch_project(
                &mut tx,
                &params.owner_user_id,
                &params.plugin_product_id,
                &guard.project_id,
            )
            .await?;
            if project.project_revision != guard.project_revision
                || project.source_state != "editable"
                || project.source_head_digest.as_deref()
                    != Some(guard.source_head_digest.as_str())
                || project.dependency_lock_digest.as_deref()
                    != Some(guard.dependency_lock_digest.as_str())
                || project.build_profile_version.as_deref()
                    != Some(guard.build_profile_version.as_str())
                || project.build_generation != guard.build_generation
            {
                return Err(conflict(
                    "Plugin auto Publish Project head advanced after proof",
                ));
            }
            let ready = sqlx::query_as::<_, PluginRuntimeReleaseRow>(
                "SELECT * FROM plugin_releases
                 WHERE owner_user_id = ? AND plugin_product_id = ? AND release_id = ?",
            )
            .bind(&params.owner_user_id)
            .bind(&params.plugin_product_id)
            .bind(&params.expected_ready_release_id)
            .fetch_one(&mut *tx)
            .await?;
            if ready.project_id.as_deref() != Some(guard.project_id.as_str())
                || ready.source_snapshot_digest.as_deref()
                    != Some(guard.source_head_digest.as_str())
                || ready.dependency_lock_digest.as_deref()
                    != Some(guard.dependency_lock_digest.as_str())
                || ready.build_profile_version.as_deref()
                    != Some(guard.build_profile_version.as_str())
                || ready.build_generation != Some(guard.build_generation)
            {
                return Err(conflict(
                    "Plugin auto Publish Ready lineage differs from its proof",
                ));
            }
        }
        let next_epoch = current
            .active_release_epoch
            .checked_add(1)
            .ok_or_else(|| conflict("Plugin Active Release epoch overflow"))?;
        let updated = sqlx::query(
            "UPDATE plugin_products
             SET product_revision = product_revision + 1,
                 pointer_revision = pointer_revision + 1,
                 active_release_epoch = ?,
                 ready_release_id = NULL, ready_release_digest = NULL,
                 active_release_id = ?, active_release_digest = ?,
                 previous_release_id = ?, previous_release_digest = ?,
                 materialized_catalog_digest = ?, updated_at = ?
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND product_revision = ? AND pointer_revision = ?
               AND active_release_epoch = ? AND ready_release_id = ?
               AND ready_release_digest = ? AND updated_at <= ?",
        )
        .bind(next_epoch)
        .bind(&params.expected_ready_release_id)
        .bind(&params.expected_ready_release_digest)
        .bind(&current.active_release_id)
        .bind(&current.active_release_digest)
        .bind(&params.target_catalog_digest)
        .bind(params.updated_at)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(params.expected_product_revision)
        .bind(params.expected_pointer_revision)
        .bind(params.expected_active_release_epoch)
        .bind(&params.expected_ready_release_id)
        .bind(&params.expected_ready_release_digest)
        .bind(params.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if updated.rows_affected() != 1 {
            return Err(conflict("Plugin Publish pointer CAS failed"));
        }
        synchronize_catalog_publication(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
            &current.lifecycle,
            Some(&params.expected_ready_release_id),
            Some(&params.expected_ready_release_digest),
            next_epoch,
            &params.target_catalog_digest,
        )
        .await?;
        revoke_surface_session(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
        )
        .await?;
        bump_library_revision(&mut tx, &params.owner_user_id, params.updated_at).await?;
        let snapshot = fetch_snapshot_in_tx(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
        )
            .await?
            .ok_or_else(|| DbError::Init("Plugin Publish lost Product".into()))?;
        tx.commit().await?;
        Ok(snapshot)
    }

    async fn rollback_previous_cas(
        &self,
        params: &RollbackPluginRuntimePreviousParams,
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(
            &params.expected_current_release_id,
            "expected_current_release_id",
        )?;
        validate_uuid(
            &params.expected_previous_release_id,
            "expected_previous_release_id",
        )?;
        validate_digest(
            &params.expected_current_release_digest,
            "expected_current_release_digest",
        )?;
        validate_digest(
            &params.expected_previous_release_digest,
            "expected_previous_release_digest",
        )?;
        validate_digest(&params.target_catalog_digest, "target_catalog_digest")?;
        if params.expected_product_revision < 1
            || params.expected_pointer_revision < 1
            || params.expected_active_release_epoch < 1
            || params.updated_at <= 0
        {
            return Err(conflict("Plugin Rollback CAS expectations are invalid"));
        }

        let mut tx = self.pool.begin().await?;
        let current =
            lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        if matches!(current.lifecycle.as_str(), "trashed" | "deleting") {
            return Err(conflict(
                "Plugin Rollback requires a non-trashed product",
            ));
        }
        if current.product_revision != params.expected_product_revision
            || current.pointer_revision != params.expected_pointer_revision
            || current.active_release_epoch != params.expected_active_release_epoch
            || current.active_release_id.as_deref()
                != Some(params.expected_current_release_id.as_str())
            || current.active_release_digest.as_deref()
                != Some(params.expected_current_release_digest.as_str())
            || current.previous_release_id.as_deref()
                != Some(params.expected_previous_release_id.as_str())
            || current.previous_release_digest.as_deref()
                != Some(params.expected_previous_release_digest.as_str())
        {
            return Err(conflict(
                "Plugin Rollback lost its exact Active/Previous pointer CAS",
            ));
        }
        if params.updated_at < current.updated_at {
            return Err(conflict("Plugin Rollback timestamp predates Product state"));
        }
        ensure_no_running_build(&mut tx, &params.plugin_product_id).await?;
        require_pointer_release(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
            Some(&params.expected_previous_release_id),
            Some(&params.expected_previous_release_digest),
            "Previous Release",
        )
        .await?;
        let next_epoch = current
            .active_release_epoch
            .checked_add(1)
            .ok_or_else(|| conflict("Plugin Active Release epoch overflow"))?;
        let updated = sqlx::query(
            "UPDATE plugin_products
             SET product_revision = product_revision + 1,
                 pointer_revision = pointer_revision + 1,
                 active_release_epoch = ?,
                 active_release_id = ?, active_release_digest = ?,
                 previous_release_id = ?, previous_release_digest = ?,
                 materialized_catalog_digest = ?, updated_at = ?
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND product_revision = ? AND pointer_revision = ?
               AND active_release_epoch = ? AND active_release_id = ?
               AND active_release_digest = ? AND previous_release_id = ?
               AND previous_release_digest = ? AND updated_at <= ?",
        )
        .bind(next_epoch)
        .bind(&params.expected_previous_release_id)
        .bind(&params.expected_previous_release_digest)
        .bind(&params.expected_current_release_id)
        .bind(&params.expected_current_release_digest)
        .bind(&params.target_catalog_digest)
        .bind(params.updated_at)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(params.expected_product_revision)
        .bind(params.expected_pointer_revision)
        .bind(params.expected_active_release_epoch)
        .bind(&params.expected_current_release_id)
        .bind(&params.expected_current_release_digest)
        .bind(&params.expected_previous_release_id)
        .bind(&params.expected_previous_release_digest)
        .bind(params.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if updated.rows_affected() != 1 {
            return Err(conflict("Plugin Rollback pointer CAS failed"));
        }
        synchronize_catalog_publication(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
            &current.lifecycle,
            Some(&params.expected_previous_release_id),
            Some(&params.expected_previous_release_digest),
            next_epoch,
            &params.target_catalog_digest,
        )
        .await?;
        revoke_surface_session(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
        )
        .await?;
        bump_library_revision(&mut tx, &params.owner_user_id, params.updated_at).await?;
        let snapshot = fetch_snapshot_in_tx(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
        )
            .await?
            .ok_or_else(|| DbError::Init("Plugin Rollback lost Product".into()))?;
        tx.commit().await?;
        Ok(snapshot)
    }

    async fn commit_lifecycle_cas(
        &self,
        params: &CommitPluginRuntimeLifecycleParams,
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_optional_digest(
            params.expected_active_release_digest.as_deref(),
            "expected_active_release_digest",
        )?;
        if params.expected_product_revision < 1
            || params.expected_pointer_revision < 1
            || params.updated_at <= 0
        {
            return Err(conflict("Plugin lifecycle CAS expectations are invalid"));
        }
        let mut tx = self.pool.begin().await?;
        let current =
            lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        if matches!(current.lifecycle.as_str(), "trashed" | "deleting")
            || current.product_revision != params.expected_product_revision
            || current.pointer_revision != params.expected_pointer_revision
            || current.active_release_digest.as_deref()
                != params.expected_active_release_digest.as_deref()
        {
            return Err(conflict("Plugin lifecycle exact CAS failed"));
        }
        let (expected_lifecycle, target_lifecycle) = if params.enabled {
            ("disabled", "enabled")
        } else {
            ("enabled", "disabled")
        };
        if current.lifecycle != expected_lifecycle {
            return Err(conflict("Plugin lifecycle transition is not applicable"));
        }
        if params.enabled
            && (current.active_release_id.is_none()
                || current.active_release_digest.is_none()
                || current.active_release_epoch < 1)
        {
            return Err(conflict("Plugin Enable requires an Active Release"));
        }
        if params.updated_at < current.updated_at {
            return Err(conflict("Plugin lifecycle timestamp predates Product state"));
        }
        ensure_no_running_build(&mut tx, &params.plugin_product_id).await?;
        let updated = sqlx::query(
            "UPDATE plugin_products
             SET product_revision = product_revision + 1,
                 lifecycle = ?, updated_at = ?
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND product_revision = ? AND pointer_revision = ?
               AND lifecycle = ? AND updated_at <= ?",
        )
        .bind(target_lifecycle)
        .bind(params.updated_at)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(params.expected_product_revision)
        .bind(params.expected_pointer_revision)
        .bind(expected_lifecycle)
        .bind(params.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if updated.rows_affected() != 1 {
            return Err(conflict("Plugin lifecycle CAS failed"));
        }
        synchronize_catalog_publication(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
            target_lifecycle,
            current.active_release_id.as_deref(),
            current.active_release_digest.as_deref(),
            current.active_release_epoch,
            &current.materialized_catalog_digest,
        )
        .await?;
        revoke_surface_session(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
        )
        .await?;
        bump_library_revision(&mut tx, &params.owner_user_id, params.updated_at).await?;
        let snapshot = fetch_snapshot_in_tx(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
        )
            .await?
            .ok_or_else(|| DbError::Init("Plugin lifecycle commit lost Product".into()))?;
        tx.commit().await?;
        Ok(snapshot)
    }

    async fn trash_cas(
        &self,
        params: &TrashPluginRuntimeParams,
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        validate_lifecycle_identity(
            &params.owner_user_id,
            &params.plugin_product_id,
            params.expected_product_revision,
            params.expected_pointer_revision,
            params.updated_at,
        )?;
        validate_optional_digest(
            params.expected_active_release_digest.as_deref(),
            "expected_active_release_digest",
        )?;

        let mut tx = self.pool.begin().await?;
        let current =
            lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        if current.product_revision != params.expected_product_revision
            || current.pointer_revision != params.expected_pointer_revision
            || current.active_release_digest.as_deref()
                != params.expected_active_release_digest.as_deref()
            || !matches!(current.lifecycle.as_str(), "enabled" | "disabled")
        {
            return Err(conflict("Plugin Trash exact lifecycle CAS failed"));
        }
        validate_deletion_state_in_tx(&mut tx, &current).await?;
        ensure_no_running_plugin_operation(&mut tx, &params.plugin_product_id).await?;
        let changed = sqlx::query(
            "UPDATE plugin_products
             SET product_revision = product_revision + 1,
                 lifecycle = 'trashed', updated_at = ?
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND product_revision = ? AND pointer_revision = ?
               AND lifecycle = ? AND updated_at <= ?",
        )
        .bind(params.updated_at)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(params.expected_product_revision)
        .bind(params.expected_pointer_revision)
        .bind(&current.lifecycle)
        .bind(params.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if changed.rows_affected() != 1 {
            return Err(conflict("Plugin Trash product CAS failed"));
        }
        revoke_catalog_publication(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        revoke_surface_session(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        bump_library_revision(&mut tx, &params.owner_user_id, params.updated_at).await?;
        let snapshot = fetch_snapshot_in_tx(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
        )
        .await?
        .ok_or_else(|| DbError::Init("Plugin Trash lost Product".into()))?;
        tx.commit().await?;
        Ok(snapshot)
    }

    async fn restore_cas(
        &self,
        params: &RestorePluginRuntimeParams,
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        validate_lifecycle_identity(
            &params.owner_user_id,
            &params.plugin_product_id,
            params.expected_product_revision,
            params.expected_pointer_revision,
            params.updated_at,
        )?;
        if params.expected_lifecycle != "trashed" {
            return Err(conflict(
                "Plugin Restore expected_lifecycle must be trashed",
            ));
        }

        let mut tx = self.pool.begin().await?;
        let current =
            lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        if current.product_revision != params.expected_product_revision
            || current.pointer_revision != params.expected_pointer_revision
            || current.lifecycle != params.expected_lifecycle
        {
            return Err(conflict("Plugin Restore exact lifecycle CAS failed"));
        }
        validate_deletion_state_in_tx(&mut tx, &current).await?;
        ensure_no_running_plugin_operation(&mut tx, &params.plugin_product_id).await?;
        let changed = sqlx::query(
            "UPDATE plugin_products
             SET product_revision = product_revision + 1,
                 lifecycle = 'disabled', updated_at = ?
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND product_revision = ? AND pointer_revision = ?
               AND lifecycle = 'trashed' AND updated_at <= ?",
        )
        .bind(params.updated_at)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(params.expected_product_revision)
        .bind(params.expected_pointer_revision)
        .bind(params.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if changed.rows_affected() != 1 {
            return Err(conflict("Plugin Restore product CAS failed"));
        }
        revoke_catalog_publication(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        revoke_surface_session(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        bump_library_revision(&mut tx, &params.owner_user_id, params.updated_at).await?;
        let snapshot = fetch_snapshot_in_tx(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
        )
        .await?
        .ok_or_else(|| DbError::Init("Plugin Restore lost Product".into()))?;
        tx.commit().await?;
        Ok(snapshot)
    }

    async fn begin_delete(
        &self,
        params: &BeginPluginRuntimeDeleteParams,
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        validate_lifecycle_identity(
            &params.owner_user_id,
            &params.plugin_product_id,
            params.expected_product_revision,
            params.expected_pointer_revision,
            params.started_at_ms,
        )?;
        validate_uuid(&params.operation_id, "operation_id")?;
        validate_optional_digest(
            params.expected_active_release_digest.as_deref(),
            "expected_active_release_digest",
        )?;

        let mut tx = self.pool.begin().await?;
        let current =
            lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        if current.product_revision != params.expected_product_revision
            || current.pointer_revision != params.expected_pointer_revision
            || current.active_release_digest.as_deref()
                != params.expected_active_release_digest.as_deref()
            || current.lifecycle != "trashed"
            || params.started_at_ms < current.updated_at
        {
            return Err(conflict("Plugin Delete begin exact lifecycle CAS failed"));
        }
        validate_deletion_state_in_tx(&mut tx, &current).await?;
        ensure_no_running_plugin_operation(&mut tx, &params.plugin_product_id).await?;
        let existing_intent: Option<PluginDeletionIntentQueryRow> =
            sqlx::query_as(
                "SELECT * FROM plugin_deletion_intents
                 WHERE owner_user_id = ? AND plugin_product_id = ?",
            )
            .bind(&params.owner_user_id)
            .bind(&params.plugin_product_id)
            .fetch_optional(&mut *tx)
            .await?;
        if existing_intent.is_some() {
            return Err(conflict("Plugin deletion intent already exists"));
        }
        sqlx::query(
            "INSERT INTO product_operations (
                operation_id, kind, owner_kind, owner_id, state,
                progress_percent, bounded_log_tail_json, started_at_ms
             ) VALUES (?, 'plugin_permanent_delete', 'plugin', ?, 'running',
                       NULL, '[]', ?)",
        )
        .bind(&params.operation_id)
        .bind(&params.plugin_product_id)
        .bind(params.started_at_ms)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        sqlx::query(
            "INSERT INTO plugin_deletion_intents (
                plugin_product_id, owner_user_id, operation_id, started_at_ms, last_error_code
             ) VALUES (?, ?, ?, ?, NULL)",
        )
        .bind(&params.plugin_product_id)
        .bind(&params.owner_user_id)
        .bind(&params.operation_id)
        .bind(params.started_at_ms)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let changed = sqlx::query(
            "UPDATE plugin_products
             SET product_revision = product_revision + 1,
                 lifecycle = 'deleting', updated_at = ?
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND product_revision = ? AND pointer_revision = ?
               AND lifecycle = 'trashed' AND updated_at <= ?",
        )
        .bind(params.started_at_ms)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(params.expected_product_revision)
        .bind(params.expected_pointer_revision)
        .bind(params.started_at_ms)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if changed.rows_affected() != 1 {
            return Err(conflict("Plugin Delete begin product CAS failed"));
        }
        revoke_catalog_publication(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        revoke_surface_session(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        bump_library_revision(&mut tx, &params.owner_user_id, params.started_at_ms).await?;
        let snapshot = fetch_snapshot_in_tx(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
        )
        .await?
        .ok_or_else(|| DbError::Init("Plugin Delete begin lost Product".into()))?;
        tx.commit().await?;
        Ok(snapshot)
    }

    async fn fail_delete(
        &self,
        params: &FailPluginRuntimeDeleteParams,
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&params.operation_id, "operation_id")?;
        validate_product_operation_error_code(Some(&params.error_code))?;
        if params.expected_operation_revision != 1 || params.updated_at <= 0 {
            return Err(conflict("Plugin Delete failure CAS expectations are invalid"));
        }
        let mut tx = self.pool.begin().await?;
        let product =
            lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        if product.lifecycle != "deleting" {
            return Err(conflict("Plugin Delete failure requires deleting lifecycle"));
        }
        let intent = fetch_deletion_intent_in_tx(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
            &params.operation_id,
        )
        .await?;
        let operation =
            fetch_plugin_operation_in_tx(&mut tx, &params.plugin_product_id, &params.operation_id)
                .await?
                .ok_or_else(|| DbError::NotFound(format!("Plugin operation {}", params.operation_id)))?;
        if operation.state != "running"
            || operation.kind != "plugin_permanent_delete"
            || operation.progress_percent.is_some()
            || operation.started_at_ms != intent.started_at_ms
            || operation.last_error_code.is_some()
        {
            return Err(conflict("Plugin Delete failure operation CAS failed"));
        }
        if params.updated_at < operation.started_at_ms {
            return Err(conflict("Plugin Delete failure timestamp predates operation"));
        }
        let changed = sqlx::query(
            "UPDATE product_operations
             SET state = 'failed', last_error_code = ?, finished_at_ms = ?
             WHERE operation_id = ? AND owner_kind = 'plugin'
               AND owner_id = ? AND state = 'running'
               AND progress_percent IS NULL",
        )
        .bind(&params.error_code)
        .bind(params.updated_at)
        .bind(&params.operation_id)
        .bind(&params.plugin_product_id)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if changed.rows_affected() != 1 {
            return Err(conflict("Plugin Delete failure operation update CAS failed"));
        }
        let changed = sqlx::query(
            "UPDATE plugin_deletion_intents
             SET last_error_code = ?
             WHERE owner_user_id = ? AND plugin_product_id = ? AND operation_id = ?
               AND last_error_code IS NULL",
        )
        .bind(&params.error_code)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(&params.operation_id)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if changed.rows_affected() != 1 {
            return Err(conflict("Plugin Delete failure intent update CAS failed"));
        }
        bump_library_revision(&mut tx, &params.owner_user_id, params.updated_at).await?;
        let snapshot = fetch_snapshot_in_tx(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
        )
        .await?
        .ok_or_else(|| DbError::Init("Plugin Delete failure lost Product".into()))?;
        tx.commit().await?;
        Ok(snapshot)
    }

    async fn restart_delete(
        &self,
        params: &RestartPluginRuntimeDeleteParams,
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(
            &params.expected_failed_operation_id,
            "expected_failed_operation_id",
        )?;
        validate_uuid(&params.new_operation_id, "new_operation_id")?;
        if params.started_at_ms <= 0
            || params.expected_failed_operation_id == params.new_operation_id
        {
            return Err(conflict("Plugin Delete restart identities are invalid"));
        }
        let mut tx = self.pool.begin().await?;
        let product =
            lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        if product.lifecycle != "deleting" || params.started_at_ms < product.updated_at {
            return Err(conflict("Plugin Delete restart lifecycle is invalid"));
        }
        let intent = fetch_deletion_intent_in_tx(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
            &params.expected_failed_operation_id,
        )
        .await?;
        let old = fetch_plugin_operation_in_tx(
            &mut tx,
            &params.plugin_product_id,
            &params.expected_failed_operation_id,
        )
        .await?
        .ok_or_else(|| {
            DbError::NotFound(format!(
                "Plugin operation {}",
                params.expected_failed_operation_id
            ))
        })?;
        if old.state != "failed"
            || old.kind != "plugin_permanent_delete"
            || old.progress_percent.is_some()
            || old.started_at_ms != intent.started_at_ms
            || old.last_error_code != intent.last_error_code
            || old
                .finished_at_ms
                .is_none_or(|finished_at_ms| params.started_at_ms < finished_at_ms)
        {
            return Err(conflict("Plugin Delete restart requires the exact failed operation"));
        }
        ensure_no_running_plugin_operation(&mut tx, &params.plugin_product_id).await?;
        sqlx::query(
            "INSERT INTO product_operations (
                operation_id, kind, owner_kind, owner_id, state,
                progress_percent, bounded_log_tail_json, started_at_ms
             ) VALUES (?, 'plugin_permanent_delete', 'plugin', ?, 'running',
                       NULL, '[]', ?)",
        )
        .bind(&params.new_operation_id)
        .bind(&params.plugin_product_id)
        .bind(params.started_at_ms)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let changed = sqlx::query(
            "UPDATE plugin_deletion_intents
             SET operation_id = ?, started_at_ms = ?, last_error_code = NULL
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND operation_id = ? AND last_error_code IS NOT NULL",
        )
        .bind(&params.new_operation_id)
        .bind(params.started_at_ms)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(&params.expected_failed_operation_id)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if changed.rows_affected() != 1 {
            return Err(conflict("Plugin Delete restart intent CAS failed"));
        }
        bump_library_revision(&mut tx, &params.owner_user_id, params.started_at_ms).await?;
        let snapshot = fetch_snapshot_in_tx(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
        )
        .await?
        .ok_or_else(|| DbError::Init("Plugin Delete restart lost Product".into()))?;
        tx.commit().await?;
        Ok(snapshot)
    }

    async fn finalize_delete(
        &self,
        params: &FinalizePluginRuntimeDeleteParams,
    ) -> Result<i64, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&params.operation_id, "operation_id")?;
        if params.expected_operation_revision != 1 || params.finished_at_ms <= 0 {
            return Err(conflict("Plugin Delete finalize CAS expectations are invalid"));
        }
        let mut tx = self.pool.begin().await?;
        let product =
            lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        if product.lifecycle != "deleting" {
            return Err(conflict("Plugin Delete finalize requires deleting lifecycle"));
        }
        let intent = fetch_deletion_intent_in_tx(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
            &params.operation_id,
        )
        .await?;
        let operation =
            fetch_plugin_operation_in_tx(&mut tx, &params.plugin_product_id, &params.operation_id)
                .await?
                .ok_or_else(|| DbError::NotFound(format!("Plugin operation {}", params.operation_id)))?;
        if operation.state != "running"
            || operation.kind != "plugin_permanent_delete"
            || operation.progress_percent.is_some()
            || operation.started_at_ms != intent.started_at_ms
        {
            return Err(conflict("Plugin Delete finalize operation CAS failed"));
        }
        if params.finished_at_ms < operation.started_at_ms {
            return Err(conflict("Plugin Delete finalize timestamp predates operation"));
        }
        let artifact_ids: Vec<String> = sqlx::query_scalar(
            "SELECT artifact_id FROM plugin_releases
             WHERE owner_user_id = ? AND plugin_product_id = ?",
        )
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .fetch_all(&mut *tx)
        .await?;
        let changed = sqlx::query(
            "UPDATE product_operations
             SET state = 'succeeded', last_error_code = NULL,
                 finished_at_ms = ?
             WHERE operation_id = ? AND owner_kind = 'plugin'
               AND owner_id = ? AND state = 'running'
               AND progress_percent IS NULL",
        )
        .bind(params.finished_at_ms)
        .bind(&params.operation_id)
        .bind(&params.plugin_product_id)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if changed.rows_affected() != 1 {
            return Err(conflict("Plugin Delete finalize operation update CAS failed"));
        }
        revoke_catalog_publication(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        revoke_surface_session(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        for table in [
            "plugin_publish_authorizations",
            "plugin_credential_bindings",
            "plugin_kv",
            "plugin_build_operation_lineage",
            "plugin_service_test_receipts",
            "plugin_releases",
            "plugin_projects",
        ] {
            let sql = format!(
                "DELETE FROM {table} WHERE owner_user_id = ? AND plugin_product_id = ?"
            );
            sqlx::query(&sql)
                .bind(&params.owner_user_id)
                .bind(&params.plugin_product_id)
                .execute(&mut *tx)
                .await
                .map_err(query_error)?;
        }
        for artifact_id in artifact_ids {
            sqlx::query(
                "DELETE FROM plugin_release_artifacts
                 WHERE owner_user_id = ? AND artifact_id = ?
                   AND NOT EXISTS (
                       SELECT 1 FROM plugin_releases release
                       WHERE release.artifact_id = plugin_release_artifacts.artifact_id
                   )",
            )
            .bind(&params.owner_user_id)
            .bind(artifact_id)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?;
        }
        sqlx::query(
            "DELETE FROM plugin_deletion_intents
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND operation_id = ?",
        )
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(&params.operation_id)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        crate::plugin_product_documents::remove_plugin_documents(
            &mut tx, &params.owner_user_id, &params.plugin_product_id, params.finished_at_ms,
        ).await?;
        let deleted = sqlx::query(
            "DELETE FROM plugin_products
             WHERE owner_user_id = ? AND plugin_product_id = ? AND lifecycle = 'deleting'",
        )
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if deleted.rows_affected() != 1 {
            return Err(conflict("Plugin Delete finalize Product cleanup failed"));
        }
        bump_library_revision(&mut tx, &params.owner_user_id, params.finished_at_ms).await?;
        let revision: i64 = sqlx::query_scalar(
            "SELECT revision FROM plugin_library_state WHERE owner_user_id = ?",
        )
        .bind(&params.owner_user_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(revision)
    }

    async fn get_plugin_operation(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
        operation_id: &str,
    ) -> Result<Option<ProductOperationRow>, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        validate_uuid(plugin_product_id, "plugin_product_id")?;
        validate_uuid(operation_id, "operation_id")?;
        ensure_owner(&self.pool, owner_user_id).await?;
        let mut tx = self.pool.begin().await?;
        let Some(product) =
            fetch_owned_product_in_tx(&mut tx, owner_user_id, plugin_product_id).await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        validate_deletion_state_in_tx(&mut tx, &product).await?;
        let operation =
            fetch_plugin_operation_in_tx(&mut tx, plugin_product_id, operation_id).await?;
        tx.commit().await?;
        Ok(operation)
    }

    async fn list_plugin_operations(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
    ) -> Result<Vec<ProductOperationRow>, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        validate_uuid(plugin_product_id, "plugin_product_id")?;
        ensure_owner(&self.pool, owner_user_id).await?;
        let mut tx = self.pool.begin().await?;
        let Some(product) =
            fetch_owned_product_in_tx(&mut tx, owner_user_id, plugin_product_id).await?
        else {
            tx.commit().await?;
            return Ok(Vec::new());
        };
        validate_deletion_state_in_tx(&mut tx, &product).await?;
        let operations = sqlx::query_as::<_, ProductOperationRow>(
            "SELECT operation.* FROM product_operations operation
             WHERE operation.owner_kind = 'plugin'
               AND operation.owner_id = ?
             ORDER BY operation.started_at_ms ASC, operation.operation_id ASC",
        )
        .bind(plugin_product_id)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(operations)
    }

    async fn set_auto_publish_cas(
        &self,
        params: &SetPluginRuntimeAutoPublishParams,
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&params.authorization_id, "authorization_id")?;
        if params.expected_product_revision < 1
            || params.expected_pointer_revision < 1
            || params.expected_authorization_revision.is_some_and(|value| value < 1)
            || params.user_authorized_at_ms <= 0
            || params.updated_at <= 0
        {
            return Err(conflict(
                "Plugin auto Publish authorization CAS expectations are invalid",
            ));
        }
        let mut tx = self.pool.begin().await?;
        let current =
            lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        if matches!(current.lifecycle.as_str(), "trashed" | "deleting")
            || current.product_revision != params.expected_product_revision
            || current.pointer_revision != params.expected_pointer_revision
        {
            return Err(conflict(
                "Plugin auto Publish authorization Product/pointer CAS failed",
            ));
        }
        if params.enabled && current.active_release_id.is_none() {
            return Err(conflict(
                "auto Publish can be enabled only after the first manual Publish",
            ));
        }
        if params.updated_at < current.updated_at
            || params.user_authorized_at_ms > params.updated_at
        {
            return Err(conflict(
                "Plugin auto Publish authorization timestamp is invalid",
            ));
        }
        if params.enabled {
            let artifact = sqlx::query_as::<_, PluginRuntimeReleaseArtifactRow>(
                "SELECT a.* FROM plugin_release_artifacts a
                 JOIN plugin_releases r ON r.artifact_id = a.artifact_id AND r.owner_user_id = a.owner_user_id
                 WHERE r.owner_user_id = ? AND r.plugin_product_id = ? AND r.release_id = ?",
            )
            .bind(&params.owner_user_id)
            .bind(&params.plugin_product_id)
            .bind(&current.active_release_id)
            .fetch_optional(&mut *tx).await?
            .ok_or_else(|| conflict("auto Publish requires an Active Release Artifact"))?;
            if !validate_artifact(&artifact)?.manifest.payload.is_ui_only() {
                return Err(conflict("auto Publish requires a UI-only Active Release"));
            }
            ensure_no_running_build(&mut tx, &params.plugin_product_id).await?;
        }
        let existing = sqlx::query_as::<_, PluginRuntimePublishAuthorizationRow>(
            "SELECT * FROM plugin_publish_authorizations
             WHERE owner_user_id = ? AND plugin_product_id = ?",
        )
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .fetch_optional(&mut *tx)
        .await?;
        match existing {
            Some(existing) => {
                if params.expected_authorization_revision != Some(existing.revision)
                    || params.authorization_id != existing.authorization_id
                {
                    return Err(conflict(
                        "Plugin auto Publish authorization revision CAS failed",
                    ));
                }
                let changed = sqlx::query(
                    "UPDATE plugin_publish_authorizations
                     SET revision = revision + 1, enabled = ?,
                         user_authorized_at_ms = ?
                     WHERE owner_user_id = ? AND plugin_product_id = ?
                       AND authorization_id = ? AND revision = ?",
                )
                .bind(params.enabled)
                .bind(params.user_authorized_at_ms)
                .bind(&params.owner_user_id)
                .bind(&params.plugin_product_id)
                .bind(&params.authorization_id)
                .bind(existing.revision)
                .execute(&mut *tx)
                .await
                .map_err(query_error)?;
                if changed.rows_affected() != 1 {
                    return Err(conflict(
                        "Plugin auto Publish authorization update CAS failed",
                    ));
                }
            }
            None => {
                if params.expected_authorization_revision.is_some() || !params.enabled {
                    return Err(conflict(
                        "Plugin auto Publish authorization creation CAS failed",
                    ));
                }
                sqlx::query(
                    "INSERT INTO plugin_publish_authorizations (
                        authorization_id, plugin_product_id, owner_user_id,
                        revision, enabled, user_authorized_at_ms
                     ) VALUES (?, ?, ?, 1, 1, ?)",
                )
                .bind(&params.authorization_id)
                .bind(&params.plugin_product_id)
                .bind(&params.owner_user_id)
                .bind(params.user_authorized_at_ms)
                .execute(&mut *tx)
                .await
                .map_err(query_error)?;
            }
        }
        let updated = sqlx::query(
            "UPDATE plugin_products
             SET product_revision = product_revision + 1, updated_at = ?
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND product_revision = ? AND pointer_revision = ?
               AND updated_at <= ?",
        )
        .bind(params.updated_at)
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(params.expected_product_revision)
        .bind(params.expected_pointer_revision)
        .bind(params.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if updated.rows_affected() != 1 {
            return Err(conflict(
                "Plugin auto Publish authorization Product CAS failed",
            ));
        }
        bump_library_revision(&mut tx, &params.owner_user_id, params.updated_at).await?;
        let snapshot = fetch_snapshot_in_tx(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
        )
            .await?
            .ok_or_else(|| {
                DbError::Init("Plugin auto Publish authorization lost Product".into())
            })?;
        tx.commit().await?;
        Ok(snapshot)
    }

    async fn open_surface_session_cas(
        &self,
        params: &OpenPluginRuntimeSurfaceSessionParams,
    ) -> Result<PluginRuntimeSurfaceSessionRow, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&params.surface_session_id, "surface_session_id")?;
        if let Some(id) = &params.conversation_id {
            validate_uuid(id, "conversation_id")?;
        }
        validate_uuid(
            &params.expected_active_release_id,
            "expected_active_release_id",
        )?;
        validate_digest(&params.capability_digest, "capability_digest")?;
        validate_digest(
            &params.expected_active_release_digest,
            "expected_active_release_digest",
        )?;
        if params.expected_product_revision < 1
            || params.expected_pointer_revision < 1
            || params.expected_active_release_epoch < 1
            || params.issued_at_ms <= 0
        {
            return Err(conflict("Plugin Surface session guard is invalid"));
        }

        let mut tx = self.pool.begin().await?;
        let product =
            lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        if let Some(id) = &params.conversation_id {
            let owned: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM agent_sessions \
                 WHERE agent_session_id = ? AND state = 'live' \
                   AND json_extract(owner_ref_json, '$.principal_kind') = 'user' \
                   AND json_extract(owner_ref_json, '$.principal_id') = ?)",
            ).bind(id).bind(&params.owner_user_id).fetch_one(&mut *tx).await?;
            if !owned {
                return Err(conflict("Plugin Surface Session grant target is not owned"));
            }
        }
        if product.lifecycle != "enabled"
            || product.product_revision != params.expected_product_revision
            || product.pointer_revision != params.expected_pointer_revision
            || product.active_release_id.as_deref()
                != Some(params.expected_active_release_id.as_str())
            || product.active_release_digest.as_deref()
                != Some(params.expected_active_release_digest.as_str())
            || product.active_release_epoch != params.expected_active_release_epoch
        {
            return Err(conflict(
                "Plugin Surface open lost its exact enabled Active Release",
            ));
        }
        if params.issued_at_ms < product.updated_at {
            return Err(conflict(
                "Plugin Surface session timestamp predates Product state",
            ));
        }
        require_pointer_release(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
            Some(&params.expected_active_release_id),
            Some(&params.expected_active_release_digest),
            "Surface Active Release",
        )
        .await?;
        let changed = sqlx::query(
            "INSERT INTO plugin_surface_sessions (
                surface_session_id, plugin_product_id, owner_user_id, generation,
                capability_digest, active_release_id, active_release_digest,
                active_release_epoch, issued_at_ms, conversation_id
             ) VALUES (?, ?, ?, 1, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(plugin_product_id) DO UPDATE SET
                surface_session_id = excluded.surface_session_id,
                owner_user_id = excluded.owner_user_id,
                generation = plugin_surface_sessions.generation + 1,
                capability_digest = excluded.capability_digest,
                active_release_id = excluded.active_release_id,
                active_release_digest = excluded.active_release_digest,
                active_release_epoch = excluded.active_release_epoch,
                issued_at_ms = excluded.issued_at_ms,
                conversation_id = excluded.conversation_id
             WHERE plugin_surface_sessions.owner_user_id = excluded.owner_user_id
               AND plugin_surface_sessions.generation < 9223372036854775807",
        )
        .bind(&params.surface_session_id)
        .bind(&params.plugin_product_id)
        .bind(&params.owner_user_id)
        .bind(&params.capability_digest)
        .bind(&params.expected_active_release_id)
        .bind(&params.expected_active_release_digest)
        .bind(params.expected_active_release_epoch)
        .bind(params.issued_at_ms)
        .bind(&params.conversation_id)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if changed.rows_affected() != 1 {
            return Err(conflict("Plugin Surface session generation overflow"));
        }
        let session = sqlx::query_as::<_, PluginRuntimeSurfaceSessionRow>(
            "SELECT * FROM plugin_surface_sessions
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND surface_session_id = ? AND capability_digest = ?",
        )
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(&params.surface_session_id)
        .bind(&params.capability_digest)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(session)
    }

    async fn resolve_surface_session(
        &self,
        params: &ResolvePluginRuntimeSurfaceSessionParams,
    ) -> Result<Option<PluginRuntimeSurfaceSessionRow>, DbError> {
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_digest(&params.capability_digest, "capability_digest")?;
        validate_digest(
            &params.expected_active_release_digest,
            "expected_active_release_digest",
        )?;
        if params.expected_active_release_epoch < 1 {
            return Err(conflict(
                "Plugin Surface session epoch must be positive",
            ));
        }
        sqlx::query_as::<_, PluginRuntimeSurfaceSessionRow>(
            "SELECT session.*
             FROM plugin_surface_sessions session
             JOIN plugin_products product
               ON product.owner_user_id = session.owner_user_id
              AND product.plugin_product_id = session.plugin_product_id
             WHERE session.plugin_product_id = ?
               AND session.capability_digest = ?
               AND session.active_release_digest = ?
               AND session.active_release_epoch = ?
               AND product.lifecycle = 'enabled'
               AND product.active_release_id = session.active_release_id
               AND product.active_release_digest = session.active_release_digest
               AND product.active_release_epoch = session.active_release_epoch",
        )
        .bind(&params.plugin_product_id)
        .bind(&params.capability_digest)
        .bind(&params.expected_active_release_digest)
        .bind(params.expected_active_release_epoch)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Query)
    }


    async fn close_surface_session_cas(
        &self,
        params: &ClosePluginRuntimeSurfaceSessionParams,
    ) -> Result<bool, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&params.surface_session_id, "surface_session_id")?;
        validate_digest(&params.capability_digest, "capability_digest")?;
        let mut tx = self.pool.begin().await?;
        lock_product_for_update(&mut tx, &params.owner_user_id, &params.plugin_product_id).await?;
        let deleted = sqlx::query(
            "DELETE FROM plugin_surface_sessions
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND surface_session_id = ? AND capability_digest = ?",
        )
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(&params.surface_session_id)
        .bind(&params.capability_digest)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?
        .rows_affected();
        tx.commit().await?;
        Ok(deleted == 1)
    }

    async fn revoke_agent_session_surfaces(
        &self,
        owner_user_id: &str,
        agent_session_id: &str,
    ) -> Result<u64, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        validate_uuid(agent_session_id, "agent_session_id")?;
        sqlx::query(
            "DELETE FROM plugin_surface_sessions \
             WHERE owner_user_id = ? AND conversation_id = ?",
        )
        .bind(owner_user_id)
        .bind(agent_session_id)
        .execute(&self.pool)
        .await
        .map(|result| result.rows_affected())
        .map_err(query_error)
    }

    async fn revoke_all_surface_sessions_on_startup(&self) -> Result<u64, DbError> {
        sqlx::query("DELETE FROM plugin_surface_sessions")
            .execute(&self.pool)
            .await
            .map(|result| result.rows_affected())
            .map_err(query_error)
    }

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
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        validate_uuid(plugin_product_id, "plugin_product_id")?;
        validate_json_object(expected_config_schema_json, "expected_config_schema_json")?;
        validate_json_object(config_json, "config_json")?;
        if expected_product_revision < 1
            || expected_pointer_revision < 1
            || expected_config_revision < 1
            || updated_at < 0
        {
            return Err(conflict("Plugin config CAS expectations are invalid"));
        }

        let mut tx = self.pool.begin().await?;
        let product = lock_product(&mut tx, owner_user_id, plugin_product_id).await?;
        if product.product_revision != expected_product_revision
            || product.pointer_revision != expected_pointer_revision
            || product.config_revision != expected_config_revision
            || product.config_schema_json != expected_config_schema_json
        {
            return Err(conflict(
                "Plugin config Product/pointer/revision/schema exact CAS failed",
            ));
        }
        ensure_no_running_build(&mut tx, plugin_product_id).await?;
        if updated_at < product.updated_at {
            return Err(conflict(
                "Plugin config timestamp predates the Product state",
            ));
        }
        if product.config_json == config_json {
            let snapshot = fetch_snapshot_in_tx(&mut tx, owner_user_id, plugin_product_id)
                .await?
                .ok_or_else(|| DbError::Init("Plugin config read lost Product".into()))?;
            tx.commit().await?;
            return Ok(snapshot);
        }
        let next_product_revision = product
            .product_revision
            .checked_add(1)
            .ok_or_else(|| conflict("Plugin Product revision overflow"))?;
        let next_config_revision = product
            .config_revision
            .checked_add(1)
            .ok_or_else(|| conflict("Plugin config revision overflow"))?;
        let changed = sqlx::query(
            "UPDATE plugin_products
             SET product_revision = ?, config_json = ?, config_revision = ?,
                 updated_at = ?
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND product_revision = ? AND pointer_revision = ?
               AND config_revision = ? AND config_schema_json = ?
               AND updated_at <= ?",
        )
        .bind(next_product_revision)
        .bind(config_json)
        .bind(next_config_revision)
        .bind(updated_at)
        .bind(owner_user_id)
        .bind(plugin_product_id)
        .bind(expected_product_revision)
        .bind(expected_pointer_revision)
        .bind(expected_config_revision)
        .bind(expected_config_schema_json)
        .bind(updated_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if changed.rows_affected() != 1 {
            return Err(conflict("Plugin config exact CAS failed"));
        }
        bump_library_revision(&mut tx, owner_user_id, updated_at).await?;
        let snapshot = fetch_snapshot_in_tx(&mut tx, owner_user_id, plugin_product_id)
            .await?
            .ok_or_else(|| DbError::Init("Plugin config commit lost Product".into()))?;
        tx.commit().await?;
        Ok(snapshot)
    }

    async fn replace_credential_bindings_cas(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
        expected_product_revision: i64,
        expected_pointer_revision: i64,
        expected_bindings_revision: i64,
        bindings: &BTreeMap<String, String>,
        updated_at: i64,
    ) -> Result<PluginRuntimeSnapshot, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        validate_uuid(plugin_product_id, "plugin_product_id")?;
        if expected_product_revision < 1
            || expected_pointer_revision < 1
            || expected_bindings_revision < 1
            || updated_at < 0
        {
            return Err(conflict(
                "Plugin Credential binding CAS expectations are invalid",
            ));
        }
        for (slot_key, credential_id) in bindings {
            validate_visible_ascii_key(slot_key, "credential slot_key", 128)?;
            validate_visible_ascii_key(credential_id, "credential_id", 512)?;
        }

        let mut tx = self.pool.begin().await?;
        let product = lock_product(&mut tx, owner_user_id, plugin_product_id).await?;
        if product.product_revision != expected_product_revision
            || product.pointer_revision != expected_pointer_revision
            || product.credential_bindings_revision != expected_bindings_revision
        {
            return Err(conflict(
                "Plugin Credential binding Product/pointer/revision exact CAS failed",
            ));
        }
        ensure_no_running_build(&mut tx, plugin_product_id).await?;
        if updated_at < product.updated_at {
            return Err(conflict(
                "Plugin Credential binding timestamp predates the Product state",
            ));
        }
        let current_rows = sqlx::query_as::<_, PluginRuntimeCredentialBindingRow>(
            "SELECT * FROM plugin_credential_bindings
             WHERE owner_user_id = ? AND plugin_product_id = ? ORDER BY slot_key",
        )
        .bind(owner_user_id)
        .bind(plugin_product_id)
        .fetch_all(&mut *tx)
        .await?;
        let current = current_rows
            .iter()
            .map(|row| (row.slot_key.clone(), row.credential_id.clone()))
            .collect::<BTreeMap<_, _>>();
        if &current == bindings {
            let snapshot = fetch_snapshot_in_tx(&mut tx, owner_user_id, plugin_product_id)
                .await?
                .ok_or_else(|| {
                    DbError::Init("Plugin Credential binding read lost Product".into())
                })?;
            tx.commit().await?;
            return Ok(snapshot);
        }
        let next_product_revision = product
            .product_revision
            .checked_add(1)
            .ok_or_else(|| conflict("Plugin Product revision overflow"))?;
        let next_bindings_revision = product
            .credential_bindings_revision
            .checked_add(1)
            .ok_or_else(|| conflict("Plugin Credential binding revision overflow"))?;

        sqlx::query(
            "DELETE FROM plugin_credential_bindings
             WHERE owner_user_id = ? AND plugin_product_id = ?",
        )
        .bind(owner_user_id)
        .bind(plugin_product_id)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        for (slot_key, credential_id) in bindings {
            sqlx::query(
                "INSERT INTO plugin_credential_bindings
                 (plugin_product_id, owner_user_id, slot_key, credential_id,
                  created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(plugin_product_id)
            .bind(owner_user_id)
            .bind(slot_key)
            .bind(credential_id)
            .bind(updated_at)
            .bind(updated_at)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?;
        }
        let changed = sqlx::query(
            "UPDATE plugin_products
             SET product_revision = ?, credential_bindings_revision = ?,
                 updated_at = ?
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND product_revision = ? AND pointer_revision = ?
               AND credential_bindings_revision = ? AND updated_at <= ?",
        )
        .bind(next_product_revision)
        .bind(next_bindings_revision)
        .bind(updated_at)
        .bind(owner_user_id)
        .bind(plugin_product_id)
        .bind(expected_product_revision)
        .bind(expected_pointer_revision)
        .bind(expected_bindings_revision)
        .bind(updated_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if changed.rows_affected() != 1 {
            return Err(conflict(
                "Plugin Credential binding exact CAS failed",
            ));
        }
        bump_library_revision(&mut tx, owner_user_id, updated_at).await?;
        let snapshot = fetch_snapshot_in_tx(&mut tx, owner_user_id, plugin_product_id)
            .await?
            .ok_or_else(|| {
                DbError::Init("Plugin Credential binding commit lost Product".into())
            })?;
        tx.commit().await?;
        Ok(snapshot)
    }

    async fn get_kv(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
        namespace: &str,
        key: &str,
    ) -> Result<Option<PluginRuntimeKvRow>, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        validate_uuid(plugin_product_id, "plugin_product_id")?;
        validate_visible_ascii_key(namespace, "Plugin KV namespace", 128)?;
        validate_visible_ascii_key(key, "Plugin KV key", 256)?;
        let mut tx = self.pool.begin().await?;
        lock_product(&mut tx, owner_user_id, plugin_product_id).await?;
        let row = sqlx::query_as::<_, PluginRuntimeKvRow>(
            "SELECT * FROM plugin_kv
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND namespace = ? AND key = ?",
        )
        .bind(owner_user_id)
        .bind(plugin_product_id)
        .bind(namespace)
        .bind(key)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = &row {
            validate_kv_row(row)?;
        }
        tx.commit().await?;
        Ok(row.filter(|row| !row.is_tombstone))
    }

    async fn execute_surface_kv(
        &self,
        params: &ExecutePluginRuntimeSurfaceKvParams,
    ) -> Result<PluginRuntimeSurfaceKvResult, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.plugin_product_id, "plugin_product_id")?;
        validate_uuid(&params.surface_session_id, "surface_session_id")?;
        validate_digest(
            &params.expected_capability_digest,
            "expected_capability_digest",
        )?;
        validate_digest(
            &params.expected_active_release_digest,
            "expected_active_release_digest",
        )?;
        validate_visible_ascii_key(&params.namespace, "Plugin KV namespace", 128)?;
        validate_visible_ascii_key(&params.key, "Plugin KV key", 256)?;
        if params.expected_surface_generation < 1
            || params.expected_active_release_epoch < 1
            || params.updated_at <= 0
        {
            return Err(conflict(
                "Plugin Surface KV epoch/timestamp is invalid",
            ));
        }
        let mut tx = self.pool.begin().await?;
        let product = lock_product_for_update(
            &mut tx,
            &params.owner_user_id,
            &params.plugin_product_id,
        )
        .await?;
        if product.lifecycle != "enabled"
            || product.active_release_epoch != params.expected_active_release_epoch
            || product.active_release_digest.as_deref()
                != Some(params.expected_active_release_digest.as_str())
        {
            return Err(conflict(
                "Plugin Surface KV session is stale for the Active Release",
            ));
        }
        if params.updated_at < product.updated_at {
            return Err(conflict(
                "Plugin Surface KV timestamp predates Product state",
            ));
        }
        let session = sqlx::query_as::<_, PluginRuntimeSurfaceSessionRow>(
            "SELECT * FROM plugin_surface_sessions
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND surface_session_id = ? AND generation = ?
               AND capability_digest = ?
               AND active_release_digest = ? AND active_release_epoch = ?",
        )
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(&params.surface_session_id)
        .bind(params.expected_surface_generation)
        .bind(&params.expected_capability_digest)
        .bind(&params.expected_active_release_digest)
        .bind(params.expected_active_release_epoch)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            conflict("Plugin Surface KV session capability was revoked")
        })?;
        if product.active_release_id.as_deref()
            != Some(session.active_release_id.as_str())
        {
            return Err(conflict(
                "Plugin Surface KV session no longer binds the Active Release",
            ));
        }
        let current = sqlx::query_as::<_, PluginRuntimeKvRow>(
            "SELECT * FROM plugin_kv
             WHERE owner_user_id = ? AND plugin_product_id = ?
               AND namespace = ? AND key = ?",
        )
        .bind(&params.owner_user_id)
        .bind(&params.plugin_product_id)
        .bind(&params.namespace)
        .bind(&params.key)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = &current {
            validate_kv_row(row)?;
        }

        let result = match &params.operation {
            PluginRuntimeSurfaceKvOperation::Get => {
                let (value, revision) = match current.as_ref() {
                    Some(row) if !row.is_tombstone => (
                        Some(serde_json::from_str(&row.value_json).map_err(|error| {
                            DbError::Init(format!("Plugin KV contains invalid JSON: {error}"))
                        })?),
                        Some(row.revision),
                    ),
                    Some(row) => (None, Some(row.revision)),
                    None => (None, None),
                };
                PluginRuntimeSurfaceKvResult::Value { value, revision }
            }
            PluginRuntimeSurfaceKvOperation::Set { value } => {
                let value_json = serde_json::to_string(value)
                    .map_err(|error| conflict(format!("Plugin KV value is invalid: {error}")))?;
                let revision = if let Some(row) = current.as_ref() {
                    if params.updated_at < row.updated_at {
                        return Err(conflict(
                            "Plugin Surface KV Set timestamp predates the existing key",
                        ));
                    }
                    let next_revision = row
                        .revision
                        .checked_add(1)
                        .ok_or_else(|| conflict("Plugin KV revision overflow"))?;
                    let changed = sqlx::query(
                        "UPDATE plugin_kv
                         SET value_json = ?, revision = ?, is_tombstone = 0,
                             updated_at = ?
                         WHERE owner_user_id = ? AND plugin_product_id = ?
                           AND namespace = ? AND key = ? AND revision = ?
                           AND updated_at <= ?",
                    )
                    .bind(value_json)
                    .bind(next_revision)
                    .bind(params.updated_at)
                    .bind(&params.owner_user_id)
                    .bind(&params.plugin_product_id)
                    .bind(&params.namespace)
                    .bind(&params.key)
                    .bind(row.revision)
                    .bind(params.updated_at)
                    .execute(&mut *tx)
                    .await
                    .map_err(query_error)?
                    .rows_affected();
                    if changed != 1 {
                        return Err(conflict("Plugin Surface KV Set lost its revision CAS"));
                    }
                    next_revision
                } else {
                    let changed = sqlx::query(
                        "INSERT INTO plugin_kv (
                            plugin_product_id, owner_user_id, namespace, key, value_json,
                            revision, key_generation, is_tombstone,
                            created_at, updated_at
                         ) VALUES (?, ?, ?, ?, ?, 1, 1, 0, ?, ?)",
                    )
                    .bind(&params.plugin_product_id)
                    .bind(&params.owner_user_id)
                    .bind(&params.namespace)
                    .bind(&params.key)
                    .bind(value_json)
                    .bind(params.updated_at)
                    .bind(params.updated_at)
                    .execute(&mut *tx)
                    .await
                    .map_err(query_error)?
                    .rows_affected();
                    if changed != 1 {
                        return Err(conflict("Plugin Surface KV Set failed"));
                    }
                    1
                };
                PluginRuntimeSurfaceKvResult::Written { revision }
            }
            PluginRuntimeSurfaceKvOperation::Delete => {
                match current.as_ref() {
                    None => PluginRuntimeSurfaceKvResult::Deleted { existed: false },
                    Some(row) if row.is_tombstone => {
                        PluginRuntimeSurfaceKvResult::Deleted { existed: false }
                    }
                    Some(row) => {
                        if params.updated_at < row.updated_at {
                            return Err(conflict(
                                "Plugin Surface KV delete timestamp predates the existing key",
                            ));
                        }
                        let next_revision = row
                            .revision
                            .checked_add(1)
                            .ok_or_else(|| conflict("Plugin KV revision overflow"))?;
                        let next_generation = row
                            .key_generation
                            .checked_add(1)
                            .ok_or_else(|| conflict("Plugin KV key generation overflow"))?;
                        let deleted = sqlx::query(
                            "UPDATE plugin_kv
                             SET value_json = 'null', revision = ?, key_generation = ?,
                                 is_tombstone = 1, updated_at = ?
                             WHERE owner_user_id = ? AND plugin_product_id = ?
                               AND namespace = ? AND key = ? AND revision = ?
                               AND key_generation = ? AND is_tombstone = 0
                               AND updated_at <= ?",
                        )
                        .bind(next_revision)
                        .bind(next_generation)
                        .bind(params.updated_at)
                        .bind(&params.owner_user_id)
                        .bind(&params.plugin_product_id)
                        .bind(&params.namespace)
                        .bind(&params.key)
                        .bind(row.revision)
                        .bind(row.key_generation)
                        .bind(params.updated_at)
                        .execute(&mut *tx)
                        .await
                        .map_err(query_error)?
                        .rows_affected();
                        if deleted != 1 {
                            return Err(conflict(
                                "Plugin Surface KV delete lost its revision CAS",
                            ));
                        }
                        PluginRuntimeSurfaceKvResult::Deleted { existed: true }
                    }
                }
            }
            PluginRuntimeSurfaceKvOperation::CompareAndSwap {
                expected_revision,
                value,
            } => {
                let observed = current.as_ref().map(|row| row.revision);
                if observed != *expected_revision {
                    PluginRuntimeSurfaceKvResult::CompareAndSwap {
                        applied: false,
                        current_revision: observed,
                    }
                } else {
                    match value {
                        Some(value) => {
                            let value_json = serde_json::to_string(value).map_err(|error| {
                                conflict(format!("Plugin KV value is invalid: {error}"))
                            })?;
                            let written = match current.as_ref() {
                                Some(row) => {
                                    write_live_kv(&mut tx, row, &value_json, params.updated_at)
                                        .await?
                                }
                                None => {
                                    insert_live_kv(
                                        &mut tx,
                                        &params.owner_user_id,
                                        &params.plugin_product_id,
                                        &params.namespace,
                                        &params.key,
                                        &value_json,
                                        params.updated_at,
                                    )
                                    .await?
                                }
                            };
                            PluginRuntimeSurfaceKvResult::CompareAndSwap {
                                applied: true,
                                current_revision: Some(written.revision),
                            }
                        }
                        None => match current.as_ref() {
                            None => PluginRuntimeSurfaceKvResult::CompareAndSwap {
                                applied: true,
                                current_revision: None,
                            },
                            Some(row) if row.is_tombstone => {
                                PluginRuntimeSurfaceKvResult::CompareAndSwap {
                                    applied: true,
                                    current_revision: Some(row.revision),
                                }
                            }
                            Some(row) => {
                                let tombstone = tombstone_kv(&mut tx, row, params.updated_at)
                                    .await?
                                    .ok_or_else(|| {
                                        DbError::Init(
                                            "Plugin KV tombstone write lost its key".into(),
                                        )
                                    })?;
                                PluginRuntimeSurfaceKvResult::CompareAndSwap {
                                    applied: true,
                                    current_revision: Some(tombstone.revision),
                                }
                            }
                        },
                    }
                }
            }
        };
        tx.commit().await?;
        Ok(result)
    }

    async fn put_kv_cas(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
        namespace: &str,
        key: &str,
        value: &serde_json::Value,
        expected_revision: Option<i64>,
        updated_at: i64,
    ) -> Result<PluginRuntimeKvRow, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        validate_uuid(plugin_product_id, "plugin_product_id")?;
        validate_visible_ascii_key(namespace, "Plugin KV namespace", 128)?;
        validate_visible_ascii_key(key, "Plugin KV key", 256)?;
        if expected_revision.is_some_and(|revision| revision < 1) || updated_at < 0 {
            return Err(conflict("Plugin KV CAS expectation/timestamp is invalid"));
        }
        let value_json = serde_json::to_string(value)
            .map_err(|error| conflict(format!("Plugin KV value is invalid: {error}")))?;
        let mut tx = self.pool.begin().await?;
        let product = lock_product_for_update(&mut tx, owner_user_id, plugin_product_id).await?;
        if updated_at < product.created_at {
            return Err(conflict(
                "Plugin KV timestamp predates the Product creation",
            ));
        }
        let current = fetch_kv_row(&mut tx, owner_user_id, plugin_product_id, namespace, key).await?;
        let row = match (current, expected_revision) {
            (None, None) => {
                insert_live_kv(
                    &mut tx,
                    owner_user_id,
                    plugin_product_id,
                    namespace,
                    key,
                    &value_json,
                    updated_at,
                )
                .await?
            }
            (None, Some(_)) => {
                return Err(conflict("Plugin KV revision CAS found no logical key"));
            }
            (Some(_), None) => {
                return Err(conflict(
                    "Plugin KV expected_revision is required for an existing logical key",
                ));
            }
            (Some(row), Some(expected_revision)) => {
                if row.revision != expected_revision {
                    return Err(conflict("Plugin KV revision CAS failed"));
                }
                write_live_kv(&mut tx, &row, &value_json, updated_at).await?
            }
        };
        tx.commit().await?;
        Ok(row)
    }

    async fn delete_kv_cas(
        &self,
        owner_user_id: &str,
        plugin_product_id: &str,
        namespace: &str,
        key: &str,
        expected_revision: i64,
        updated_at: i64,
    ) -> Result<bool, DbError> {
        validate_uuid(owner_user_id, "owner_user_id")?;
        validate_uuid(plugin_product_id, "plugin_product_id")?;
        validate_visible_ascii_key(namespace, "Plugin KV namespace", 128)?;
        validate_visible_ascii_key(key, "Plugin KV key", 256)?;
        if expected_revision < 1 || updated_at < 0 {
            return Err(conflict(
                "Plugin KV delete CAS expectation/timestamp is invalid",
            ));
        }
        let mut tx = self.pool.begin().await?;
        let product = lock_product_for_update(&mut tx, owner_user_id, plugin_product_id).await?;
        if updated_at < product.created_at {
            return Err(conflict(
                "Plugin KV timestamp predates the Product creation",
            ));
        }
        let current = fetch_kv_row(&mut tx, owner_user_id, plugin_product_id, namespace, key).await?;
        let deleted = match current {
            None => false,
            Some(row) => {
                if row.revision != expected_revision {
                    return Err(conflict("Plugin KV delete revision CAS failed"));
                }
                if row.is_tombstone {
                    false
                } else {
                    tombstone_kv(&mut tx, &row, updated_at).await?;
                    true
                }
            }
        };
        tx.commit().await?;
        Ok(deleted)
    }
}
