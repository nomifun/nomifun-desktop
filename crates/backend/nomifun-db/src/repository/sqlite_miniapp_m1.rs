use std::collections::BTreeMap;

use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::DbError;
use crate::models::{
    MiniAppCredentialBindingRow, MiniAppLibraryStateRow,
    MiniAppM1LibrarySnapshot, MiniAppM1Snapshot, MiniAppProductRow,
    MiniAppProjectRow, MiniAppReleaseRow,
};
use crate::repository::miniapp_m1::{
    CommitMiniAppM1PointerStateParams, CreateMiniAppM1Params,
    IMiniAppM1Repository, RecordMiniAppM1ReadyReleaseParams,
    UpdateMiniAppM1ProjectSourceParams, conflict, query_error,
    validate_artifact, validate_digest, validate_json_object,
    validate_project_source, validate_release, validate_uuid,
    validate_pointer_state,
};

#[derive(Clone, Debug)]
pub struct SqliteMiniAppM1Repository {
    pool: SqlitePool,
}

impl SqliteMiniAppM1Repository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
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
            "MiniApp owner {owner_user_id}"
        )));
    }
    Ok(())
}

async fn library_row(
    pool: &SqlitePool,
    owner_user_id: &str,
) -> Result<MiniAppLibraryStateRow, DbError> {
    if let Some(row) = sqlx::query_as::<_, MiniAppLibraryStateRow>(
        "SELECT * FROM miniapp_library_state WHERE owner_user_id = ?",
    )
    .bind(owner_user_id)
    .fetch_optional(pool)
    .await?
    {
        return Ok(row);
    }
    Ok(MiniAppLibraryStateRow {
        id: 0,
        singleton_key: "miniapp_m1".to_owned(),
        owner_user_id: owner_user_id.to_owned(),
        revision: 0,
        updated_at: 0,
    })
}

async fn fetch_snapshot(
    pool: &SqlitePool,
    owner_user_id: &str,
    miniapp_id: &str,
) -> Result<Option<MiniAppM1Snapshot>, DbError> {
    let Some(product) = sqlx::query_as::<_, MiniAppProductRow>(
        "SELECT * FROM miniapp_products
         WHERE owner_user_id = ? AND miniapp_id = ?",
    )
    .bind(owner_user_id)
    .bind(miniapp_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let project = sqlx::query_as::<_, MiniAppProjectRow>(
        "SELECT * FROM miniapp_projects
         WHERE owner_user_id = ? AND miniapp_id = ?",
    )
    .bind(owner_user_id)
    .bind(miniapp_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        DbError::Init(format!(
            "MiniApp {miniapp_id} has no exact owner-scoped Project"
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
        let release = sqlx::query_as::<_, MiniAppReleaseRow>(
            "SELECT * FROM miniapp_releases
             WHERE owner_user_id = ? AND release_id = ?",
        )
        .bind(owner_user_id)
        .bind(release_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| {
            DbError::Init(format!(
                "MiniApp pointer references missing Release {release_id}"
            ))
        })?;
        releases.insert(release_id.to_owned(), release);
    }
    let credentials = sqlx::query_as::<_, MiniAppCredentialBindingRow>(
        "SELECT * FROM miniapp_credential_bindings
         WHERE owner_user_id = ? AND miniapp_id = ? ORDER BY slot_key",
    )
    .bind(owner_user_id)
    .bind(miniapp_id)
    .fetch_all(pool)
    .await?;
    Ok(Some(MiniAppM1Snapshot {
        library_revision: library_row(pool, owner_user_id).await?.revision,
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
        product,
        project,
        credential_bindings: credentials,
    }))
}

async fn lock_product(
    tx: &mut Transaction<'_, Sqlite>,
    owner_user_id: &str,
    miniapp_id: &str,
) -> Result<MiniAppProductRow, DbError> {
    sqlx::query_as::<_, MiniAppProductRow>(
        "SELECT * FROM miniapp_products
         WHERE owner_user_id = ? AND miniapp_id = ?",
    )
    .bind(owner_user_id)
    .bind(miniapp_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| DbError::NotFound(format!("MiniApp {miniapp_id}")))
}

async fn bump_library_revision(
    tx: &mut Transaction<'_, Sqlite>,
    owner_user_id: &str,
    updated_at: i64,
) -> Result<(), DbError> {
    let changed = sqlx::query(
        "UPDATE miniapp_library_state
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
            "MiniApp owner {owner_user_id} has no library state"
        )));
    }
    Ok(())
}

async fn require_pointer_release(
    tx: &mut Transaction<'_, Sqlite>,
    owner_user_id: &str,
    miniapp_id: &str,
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
        "SELECT COUNT(*) FROM miniapp_releases
         WHERE owner_user_id = ? AND miniapp_id = ?
           AND release_id = ? AND release_digest = ?",
    )
    .bind(owner_user_id)
    .bind(miniapp_id)
    .bind(release_id)
    .bind(release_digest)
    .fetch_one(&mut **tx)
    .await?;
    if count != 1 {
        return Err(DbError::Conflict(format!(
            "{label} does not bind an exact Release owned by this MiniApp"
        )));
    }
    Ok(())
}

#[async_trait::async_trait]
impl IMiniAppM1Repository for SqliteMiniAppM1Repository {
    async fn library(
        &self,
        owner_user_id: &str,
    ) -> Result<MiniAppM1LibrarySnapshot, DbError> {
        nomifun_common::validate_uuidv7(owner_user_id)
            .map_err(|error| DbError::Conflict(error.to_string()))?;
        ensure_owner(&self.pool, owner_user_id).await?;
        let library = library_row(&self.pool, owner_user_id).await?;
        let products = sqlx::query_as::<_, MiniAppProductRow>(
            "SELECT * FROM miniapp_products
             WHERE owner_user_id = ? ORDER BY updated_at DESC, id DESC",
        )
        .bind(owner_user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(MiniAppM1LibrarySnapshot { library, products })
    }

    async fn get(
        &self,
        owner_user_id: &str,
        miniapp_id: &str,
    ) -> Result<Option<MiniAppM1Snapshot>, DbError> {
        nomifun_common::validate_uuidv7(owner_user_id)
            .map_err(|error| DbError::Conflict(error.to_string()))?;
        nomifun_common::validate_uuidv7(miniapp_id)
            .map_err(|error| DbError::Conflict(error.to_string()))?;
        ensure_owner(&self.pool, owner_user_id).await?;
        fetch_snapshot(&self.pool, owner_user_id, miniapp_id).await
    }

    async fn create(
        &self,
        params: &CreateMiniAppM1Params,
    ) -> Result<MiniAppM1Snapshot, DbError> {
        nomifun_common::validate_uuidv7(&params.owner_user_id)
            .map_err(|error| DbError::Conflict(error.to_string()))?;
        nomifun_common::validate_uuidv7(&params.miniapp_id)
            .map_err(|error| DbError::Conflict(error.to_string()))?;
        nomifun_common::validate_uuidv7(&params.project_id)
            .map_err(|error| DbError::Conflict(error.to_string()))?;
        if params.expected_library_revision < 0 || params.created_at < 0 {
            return Err(DbError::Conflict(
                "MiniApp create CAS/timestamp is invalid".to_owned(),
            ));
        }
        if params.display_name.trim().is_empty()
            || params.display_name.chars().count() > 255
        {
            return Err(DbError::Conflict(
                "MiniApp display_name must contain 1 to 255 characters".to_owned(),
            ));
        }
        validate_json_object(&params.config_schema_json, "config_schema_json")?;
        validate_json_object(&params.config_json, "config_json")?;
        validate_digest(
            &params.materialized_catalog_digest,
            "materialized_catalog_digest",
        )?;
        ensure_owner(&self.pool, &params.owner_user_id).await?;
        let mut tx = self.pool.begin().await?;
        let current = sqlx::query_as::<_, MiniAppLibraryStateRow>(
            "SELECT * FROM miniapp_library_state
             WHERE owner_user_id = ?",
        )
        .bind(&params.owner_user_id)
        .fetch_optional(&mut *tx)
        .await?;
        let current_revision = current.as_ref().map_or(0, |row| row.revision);
        if current_revision != params.expected_library_revision {
            return Err(conflict(format!(
                "MiniApp library revision changed from expected {} to {}",
                params.expected_library_revision, current_revision
            )));
        }
        let next_library_revision = current_revision + 1;
        if current.is_none() {
            sqlx::query(
                "INSERT INTO miniapp_library_state
                 (singleton_key, owner_user_id, revision, updated_at)
                 VALUES ('miniapp_m1', ?, ?, ?)",
            )
            .bind(&params.owner_user_id)
            .bind(next_library_revision)
            .bind(params.created_at)
            .execute(&mut *tx)
            .await
            .map_err(query_error)?;
        } else {
            let changed = sqlx::query(
                "UPDATE miniapp_library_state SET revision = ?, updated_at = ?
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
                    "MiniApp library create CAS or timestamp check failed",
                ));
            }
        }
        sqlx::query(
            "INSERT INTO miniapp_products
             (miniapp_id, owner_user_id, display_name, description,
              icon_asset_id, kind, materialized_catalog_digest,
              config_schema_json, config_json, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&params.miniapp_id)
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
        sqlx::query(
            "INSERT INTO miniapp_projects
             (project_id, miniapp_id, owner_user_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&params.project_id)
        .bind(&params.miniapp_id)
        .bind(&params.owner_user_id)
        .bind(params.created_at)
        .bind(params.created_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        tx.commit().await?;
        fetch_snapshot(&self.pool, &params.owner_user_id, &params.miniapp_id)
            .await?
            .ok_or_else(|| DbError::Init("MiniApp create lost its Product".into()))
    }

    async fn update_project_source_cas(
        &self,
        params: &UpdateMiniAppM1ProjectSourceParams,
    ) -> Result<MiniAppProjectRow, DbError> {
        validate_uuid(&params.owner_user_id, "owner_user_id")?;
        validate_uuid(&params.miniapp_id, "miniapp_id")?;
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
        let changed = sqlx::query(
            "UPDATE miniapp_projects
             SET project_revision = project_revision + 1,
                 source_state = ?, managed_source_path = ?,
                 source_head_digest = ?, dependency_lock_digest = ?,
                 build_profile_version = ?, build_generation = ?,
                 updated_at = ?
             WHERE owner_user_id = ? AND miniapp_id = ? AND project_id = ?
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
        .bind(&params.miniapp_id)
        .bind(&params.project_id)
        .bind(params.expected_project_revision)
        .bind(params.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if changed.rows_affected() != 1 {
            return Err(DbError::Conflict(
                "MiniApp Project source CAS failed".to_owned(),
            ));
        }
        bump_library_revision(
            &mut tx,
            &params.owner_user_id,
            params.updated_at,
        )
        .await?;
        let project = sqlx::query_as(
            "SELECT * FROM miniapp_projects
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
        params: &RecordMiniAppM1ReadyReleaseParams,
    ) -> Result<MiniAppM1Snapshot, DbError> {
        validate_artifact(&params.artifact)?;
        validate_release(&params.release)?;
        if params.release.owner_user_id != params.owner_user_id
            || params.release.miniapp_id != params.miniapp_id
            || params.release.project_id.as_deref() != Some(params.project_id.as_str())
            || params.release.artifact_id != params.artifact.artifact_id
            || params.release.artifact_digest != params.artifact.artifact_digest
            || params.release.manifest_digest != params.artifact.manifest_digest
        {
            return Err(DbError::Conflict(
                "MiniApp Ready Release does not bind its exact owner/Product/Project/Artifact"
                    .to_owned(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        let product = lock_product(&mut tx, &params.owner_user_id, &params.miniapp_id).await?;
        if product.product_revision != params.expected_product_revision
            || product.pointer_revision != params.expected_pointer_revision
        {
            return Err(DbError::Conflict(
                "MiniApp Ready Release product CAS failed".to_owned(),
            ));
        }
        let project: MiniAppProjectRow = sqlx::query_as(
            "SELECT * FROM miniapp_projects
             WHERE owner_user_id = ? AND miniapp_id = ? AND project_id = ?",
        )
        .bind(&params.owner_user_id)
        .bind(&params.miniapp_id)
        .bind(&params.project_id)
        .fetch_one(&mut *tx)
        .await?;
        if project.project_revision != params.expected_project_revision
            || project.build_generation != params.expected_build_generation
        {
            return Err(DbError::Conflict(
                "MiniApp Ready Release project CAS failed".to_owned(),
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
                "MiniApp Ready Release source lineage differs from the exact Project head"
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
                    || owner_kind != "miniapp"
                    || owner_id != &params.miniapp_id
                    || state != "succeeded"
            },
        ) {
            return Err(DbError::Conflict(
                "MiniApp Ready Release requires an exact successful owner Operation"
                    .to_owned(),
            ));
        }
        sqlx::query(
            "INSERT INTO miniapp_release_artifacts
             (artifact_id, owner_user_id, artifact_digest, manifest_digest,
              artifact_record_json, managed_path, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (artifact_digest) DO NOTHING",
        )
        .bind(&params.artifact.artifact_id)
        .bind(&params.artifact.owner_user_id)
        .bind(&params.artifact.artifact_digest)
        .bind(&params.artifact.manifest_digest)
        .bind(&params.artifact.artifact_record_json)
        .bind(&params.artifact.managed_path)
        .bind(params.artifact.created_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let artifact_match: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM miniapp_release_artifacts
             WHERE owner_user_id = ? AND artifact_id = ?
               AND artifact_digest = ? AND manifest_digest = ?",
        )
        .bind(&params.owner_user_id)
        .bind(&params.artifact.artifact_id)
        .bind(&params.artifact.artifact_digest)
        .bind(&params.artifact.manifest_digest)
        .fetch_one(&mut *tx)
        .await?;
        if artifact_match != 1 {
            return Err(DbError::Conflict(
                "MiniApp Artifact digest is already bound to different metadata".into(),
            ));
        }
        sqlx::query(
            "INSERT INTO miniapp_releases
             (release_id, miniapp_id, owner_user_id, artifact_id,
              artifact_digest, manifest_digest, release_digest, origin_kind,
              origin_operation_id, source_kind, project_id, source_snapshot_digest,
              dependency_lock_digest, build_profile_version, build_generation,
              release_record_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&params.release.release_id)
        .bind(&params.release.miniapp_id)
        .bind(&params.release.owner_user_id)
        .bind(&params.release.artifact_id)
        .bind(&params.release.artifact_digest)
        .bind(&params.release.manifest_digest)
        .bind(&params.release.release_digest)
        .bind(&params.release.origin_kind)
        .bind(&params.release.origin_operation_id)
        .bind(&params.release.source_kind)
        .bind(&params.release.project_id)
        .bind(&params.release.source_snapshot_digest)
        .bind(&params.release.dependency_lock_digest)
        .bind(&params.release.build_profile_version)
        .bind(params.release.build_generation)
        .bind(&params.release.release_record_json)
        .bind(params.release.created_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        let updated = sqlx::query(
            "UPDATE miniapp_products
             SET product_revision = product_revision + 1,
                 pointer_revision = pointer_revision + 1,
                 ready_release_id = ?, ready_release_digest = ?,
                 updated_at = ?
             WHERE owner_user_id = ? AND miniapp_id = ?
               AND product_revision = ? AND pointer_revision = ?
               AND updated_at <= ?",
        )
        .bind(&params.release.release_id)
        .bind(&params.release.release_digest)
        .bind(params.updated_at)
        .bind(&params.owner_user_id)
        .bind(&params.miniapp_id)
        .bind(params.expected_product_revision)
        .bind(params.expected_pointer_revision)
        .bind(params.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if updated.rows_affected() != 1 {
            return Err(DbError::Conflict(
                "MiniApp Ready pointer CAS failed".to_owned(),
            ));
        }
        bump_library_revision(
            &mut tx,
            &params.owner_user_id,
            params.updated_at,
        )
        .await?;
        tx.commit().await?;
        fetch_snapshot(&self.pool, &params.owner_user_id, &params.miniapp_id)
            .await?
            .ok_or_else(|| DbError::Init("MiniApp Ready commit lost Product".into()))
    }

    async fn commit_pointer_state_cas(
        &self,
        params: &CommitMiniAppM1PointerStateParams,
    ) -> Result<MiniAppM1Snapshot, DbError> {
        validate_pointer_state(params)?;
        let mut tx = self.pool.begin().await?;
        let current =
            lock_product(&mut tx, &params.owner_user_id, &params.miniapp_id)
                .await?;
        if current.product_revision != params.expected_product_revision
            || current.pointer_revision != params.expected_pointer_revision
            || current.active_release_epoch
                != params.expected_active_release_epoch
        {
            return Err(DbError::Conflict(
                "MiniApp pointer CAS failed".to_owned(),
            ));
        }
        for (id, digest, label) in [
            (
                params.ready_release_id.as_deref(),
                params.ready_release_digest.as_deref(),
                "Ready Release",
            ),
            (
                params.active_release_id.as_deref(),
                params.active_release_digest.as_deref(),
                "Active Release",
            ),
            (
                params.previous_release_id.as_deref(),
                params.previous_release_digest.as_deref(),
                "Previous Release",
            ),
        ] {
            require_pointer_release(
                &mut tx,
                &params.owner_user_id,
                &params.miniapp_id,
                id,
                digest,
                label,
            )
            .await?;
        }
        let updated = sqlx::query(
            "UPDATE miniapp_products
             SET product_revision = product_revision + 1,
                 pointer_revision = pointer_revision + 1,
                 active_release_epoch = ?,
                 ready_release_id = ?, ready_release_digest = ?,
                 active_release_id = ?, active_release_digest = ?,
                 previous_release_id = ?, previous_release_digest = ?,
                 materialized_catalog_digest = ?, updated_at = ?
             WHERE owner_user_id = ? AND miniapp_id = ?
               AND product_revision = ? AND pointer_revision = ?
               AND active_release_epoch = ? AND updated_at <= ?",
        )
        .bind(params.active_release_epoch)
        .bind(&params.ready_release_id)
        .bind(&params.ready_release_digest)
        .bind(&params.active_release_id)
        .bind(&params.active_release_digest)
        .bind(&params.previous_release_id)
        .bind(&params.previous_release_digest)
        .bind(&params.materialized_catalog_digest)
        .bind(params.updated_at)
        .bind(&params.owner_user_id)
        .bind(&params.miniapp_id)
        .bind(params.expected_product_revision)
        .bind(params.expected_pointer_revision)
        .bind(params.expected_active_release_epoch)
        .bind(params.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(query_error)?;
        if updated.rows_affected() != 1 {
            return Err(DbError::Conflict(
                "MiniApp pointer CAS failed".to_owned(),
            ));
        }
        bump_library_revision(
            &mut tx,
            &params.owner_user_id,
            params.updated_at,
        )
        .await?;
        tx.commit().await?;
        fetch_snapshot(&self.pool, &params.owner_user_id, &params.miniapp_id)
            .await?
            .ok_or_else(|| DbError::Init("MiniApp pointer commit lost Product".into()))
    }
}
