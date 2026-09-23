use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    DigestHex, PluginArtifact, PluginDraftId, PluginId, PluginManifest, PluginMutationId,
};
use nomifun_db::sqlx::{self, Row, SqlitePool};
use serde_json::Value;
use thiserror::Error;

use crate::model::{
    InstallCommit, PluginDraftMessage, PluginDraftRecord, PluginDraftStatus, PluginGrant,
    PluginInventory, PluginLibraryState, PluginMutationKind, PluginMutationPhase,
    PluginCredentialBinding, PluginMutationRecord, PluginRecord, StoredArtifactRecord,
};

#[derive(Debug, Error)]
pub enum PluginRepositoryError {
    #[error("Plugin was not found")]
    NotFound,
    #[error("Plugin revision conflict")]
    Conflict,
    #[error("Plugin package identity already exists for this owner")]
    PackageConflict,
    #[error("Plugin repository data is invalid: {0}")]
    InvalidData(String),
    #[error("Plugin repository query failed: {0}")]
    Query(#[from] sqlx::Error),
}

pub type PluginRepositoryResult<T> = Result<T, PluginRepositoryError>;

#[async_trait]
pub trait PluginRepository: Send + Sync {
    async fn list_plugins(&self, owner_user_id: &str) -> PluginRepositoryResult<Vec<PluginRecord>>;
    async fn get_plugin(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
    ) -> PluginRepositoryResult<Option<PluginRecord>>;
    async fn inventory(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
    ) -> PluginRepositoryResult<Option<PluginInventory>>;
    async fn get_artifact(
        &self,
        digest: &DigestHex,
    ) -> PluginRepositoryResult<Option<StoredArtifactRecord>>;
    async fn put_artifact(&self, artifact: &StoredArtifactRecord) -> PluginRepositoryResult<()>;

    async fn create_draft(&self, draft: &PluginDraftRecord) -> PluginRepositoryResult<()>;
    async fn list_drafts(
        &self,
        owner_user_id: &str,
    ) -> PluginRepositoryResult<Vec<PluginDraftRecord>>;
    async fn get_draft(
        &self,
        owner_user_id: &str,
        draft_id: &PluginDraftId,
    ) -> PluginRepositoryResult<Option<PluginDraftRecord>>;
    async fn update_draft(
        &self,
        draft: &PluginDraftRecord,
        expected_revision: u64,
    ) -> PluginRepositoryResult<PluginDraftRecord>;
    async fn delete_draft(
        &self,
        owner_user_id: &str,
        draft_id: &PluginDraftId,
    ) -> PluginRepositoryResult<bool>;

    async fn begin_mutation(&self, mutation: &PluginMutationRecord) -> PluginRepositoryResult<()>;
    async fn update_mutation_phase(
        &self,
        mutation_id: &PluginMutationId,
        phase: PluginMutationPhase,
        error: Option<&str>,
        now_ms: i64,
    ) -> PluginRepositoryResult<()>;
    async fn list_mutations(&self) -> PluginRepositoryResult<Vec<PluginMutationRecord>>;
    async fn finish_mutation(&self, mutation_id: &PluginMutationId) -> PluginRepositoryResult<()>;
    async fn commit_install(&self, commit: &InstallCommit) -> PluginRepositoryResult<PluginRecord>;
    async fn rollback_install(
        &self,
        mutation_id: &PluginMutationId,
        owner_user_id: &str,
        plugin_id: &PluginId,
        now_ms: i64,
    ) -> PluginRepositoryResult<PluginRecord>;

    async fn set_enabled(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
        expected_revision: u64,
        enabled: bool,
        now_ms: i64,
    ) -> PluginRepositoryResult<PluginRecord>;
    async fn set_last_error(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
        error: Option<&str>,
        now_ms: i64,
    ) -> PluginRepositoryResult<PluginRecord>;
    async fn set_config(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
        expected_revision: u64,
        config: &Value,
        credential_bindings: &BTreeMap<String, String>,
        grants: &BTreeMap<String, bool>,
        now_ms: i64,
    ) -> PluginRepositoryResult<PluginRecord>;
    async fn trash(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
        expected_revision: u64,
        trashed: bool,
        now_ms: i64,
    ) -> PluginRepositoryResult<PluginRecord>;
    async fn restore_previous(
        &self,
        mutation: &PluginMutationRecord,
        expected_revision: u64,
        restore_data: bool,
        now_ms: i64,
    ) -> PluginRepositoryResult<PluginRecord>;
    async fn delete_plugin_rows(
        &self,
        mutation_id: &PluginMutationId,
        owner_user_id: &str,
        plugin_id: &PluginId,
        expected_revision: u64,
    ) -> PluginRepositoryResult<()>;
    async fn list_library_states(
        &self,
        owner_user_id: &str,
    ) -> PluginRepositoryResult<Vec<PluginLibraryState>>;
    async fn replace_library_states(
        &self,
        owner_user_id: &str,
        expected_revision: u64,
        states: &[PluginLibraryState],
    ) -> PluginRepositoryResult<u64>;
}

#[derive(Clone)]
pub struct SqlitePluginRepository {
    pool: Arc<SqlitePool>,
}

impl std::fmt::Debug for SqlitePluginRepository {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SqlitePluginRepository")
            .finish_non_exhaustive()
    }
}

impl SqlitePluginRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool: Arc::new(pool) }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

#[derive(Debug)]
struct MutationRollbackSnapshot {
    previous_artifact_digest: Option<String>,
    previous_data_generation: Option<String>,
    config_json: String,
    credential_bindings_json: String,
    grants_json: String,
}

#[async_trait]
impl PluginRepository for SqlitePluginRepository {
    async fn list_plugins(&self, owner_user_id: &str) -> PluginRepositoryResult<Vec<PluginRecord>> {
        let sql = format!(
            "{PLUGIN_SELECT} WHERE owner_user_id = ? ORDER BY updated_at_ms DESC, plugin_id"
        );
        let rows = sqlx::query(&sql)
            .bind(owner_user_id)
            .fetch_all(self.pool())
            .await?;
        rows.into_iter().map(plugin_from_row).collect()
    }

    async fn get_plugin(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
    ) -> PluginRepositoryResult<Option<PluginRecord>> {
        let sql = format!("{PLUGIN_SELECT} WHERE owner_user_id = ? AND plugin_id = ?");
        sqlx::query(&sql)
            .bind(owner_user_id)
            .bind(plugin_id.as_ref())
            .fetch_optional(self.pool())
            .await?
            .map(plugin_from_row)
            .transpose()
    }

    async fn inventory(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
    ) -> PluginRepositoryResult<Option<PluginInventory>> {
        let Some(plugin) = self.get_plugin(owner_user_id, plugin_id).await? else {
            return Ok(None);
        };
        let artifact = self
            .get_artifact(&plugin.active_artifact_digest)
            .await?
            .ok_or_else(|| PluginRepositoryError::InvalidData("active Artifact is missing".into()))?;
        let previous_artifact = match &plugin.previous_artifact_digest {
            Some(digest) => Some(
                self.get_artifact(digest)
                    .await?
                    .ok_or_else(|| PluginRepositoryError::InvalidData("previous Artifact is missing".into()))?,
            ),
            None => None,
        };
        let credential_bindings = credential_bindings(self.pool(), owner_user_id, plugin_id).await?;
        let grants = grants(self.pool(), owner_user_id, plugin_id).await?;
        let library = library_state(self.pool(), owner_user_id, plugin_id).await?;
        Ok(Some(PluginInventory {
            plugin,
            artifact,
            previous_artifact,
            credential_bindings,
            grants,
            library,
        }))
    }

    async fn get_artifact(
        &self,
        digest: &DigestHex,
    ) -> PluginRepositoryResult<Option<StoredArtifactRecord>> {
        sqlx::query(
            "SELECT artifact_digest, manifest_json, files_json, artifact_root, created_at_ms \
             FROM plugin_artifacts WHERE artifact_digest = ?",
        )
        .bind(digest.as_ref())
        .fetch_optional(self.pool())
        .await?
        .map(artifact_from_row)
        .transpose()
    }

    async fn put_artifact(&self, artifact: &StoredArtifactRecord) -> PluginRepositoryResult<()> {
        artifact.artifact.validate().map_err(|error| {
            PluginRepositoryError::InvalidData(format!("invalid Artifact: {error}"))
        })?;
        let manifest_json = serde_json::to_string(&artifact.artifact.manifest)
            .map_err(|error| PluginRepositoryError::InvalidData(error.to_string()))?;
        let files_json = serde_json::to_string(&artifact.artifact.files)
            .map_err(|error| PluginRepositoryError::InvalidData(error.to_string()))?;
        sqlx::query(
            "INSERT INTO plugin_artifacts \
             (artifact_digest, package_id, version, manifest_json, files_json, artifact_root, \
              has_ui, has_service, data_version, created_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(artifact_digest) DO NOTHING",
        )
        .bind(artifact.artifact.artifact_digest.as_ref())
        .bind(&artifact.artifact.manifest.id)
        .bind(&artifact.artifact.manifest.version)
        .bind(&manifest_json)
        .bind(&files_json)
        .bind(&artifact.artifact_root)
        .bind(artifact.artifact.manifest.has_ui())
        .bind(artifact.artifact.manifest.has_service())
        .bind(i64::from(artifact.artifact.manifest.data_version))
        .bind(artifact.created_at_ms)
        .execute(self.pool())
        .await?;
        let stored = self
            .get_artifact(&artifact.artifact.artifact_digest)
            .await?
            .ok_or_else(|| PluginRepositoryError::InvalidData("stored Artifact disappeared".into()))?;
        if stored.artifact != artifact.artifact || stored.artifact_root != artifact.artifact_root {
            return Err(PluginRepositoryError::InvalidData(
                "Artifact digest is already bound to different content".into(),
            ));
        }
        Ok(())
    }

    async fn create_draft(&self, draft: &PluginDraftRecord) -> PluginRepositoryResult<()> {
        sqlx::query(
            "INSERT INTO plugin_drafts \
             (draft_id, owner_user_id, plugin_id, base_revision, revision, name, workspace_path, \
              messages_json, status, last_error, created_at_ms, updated_at_ms) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(draft.draft_id.as_ref())
        .bind(&draft.owner_user_id)
        .bind(draft.plugin_id.as_ref().map(AsRef::as_ref))
        .bind(draft.base_revision.map(u64_to_i64).transpose()?)
        .bind(u64_to_i64(draft.revision)?)
        .bind(&draft.name)
        .bind(&draft.workspace_path)
        .bind(canonical_json(&draft.messages)?)
        .bind(draft.status.as_str())
        .bind(&draft.last_error)
        .bind(draft.created_at_ms)
        .bind(draft.updated_at_ms)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    async fn list_drafts(
        &self,
        owner_user_id: &str,
    ) -> PluginRepositoryResult<Vec<PluginDraftRecord>> {
        let rows = sqlx::query(
            "SELECT draft_id, owner_user_id, plugin_id, base_revision, revision, name, workspace_path, \
                    messages_json, status, last_error, created_at_ms, updated_at_ms \
             FROM plugin_drafts WHERE owner_user_id = ? ORDER BY updated_at_ms DESC, draft_id",
        )
        .bind(owner_user_id)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(draft_from_row).collect()
    }

    async fn get_draft(
        &self,
        owner_user_id: &str,
        draft_id: &PluginDraftId,
    ) -> PluginRepositoryResult<Option<PluginDraftRecord>> {
        sqlx::query(
            "SELECT draft_id, owner_user_id, plugin_id, base_revision, revision, name, workspace_path, \
                    messages_json, status, last_error, created_at_ms, updated_at_ms \
             FROM plugin_drafts WHERE owner_user_id = ? AND draft_id = ?",
        )
        .bind(owner_user_id)
        .bind(draft_id.as_ref())
        .fetch_optional(self.pool())
        .await?
        .map(draft_from_row)
        .transpose()
    }

    async fn update_draft(
        &self,
        draft: &PluginDraftRecord,
        expected_revision: u64,
    ) -> PluginRepositoryResult<PluginDraftRecord> {
        let result = sqlx::query(
            "UPDATE plugin_drafts SET plugin_id = ?, base_revision = ?, revision = revision + 1, name = ?, \
                    workspace_path = ?, messages_json = ?, status = ?, last_error = ?, updated_at_ms = ? \
             WHERE owner_user_id = ? AND draft_id = ? AND revision = ?",
        )
        .bind(draft.plugin_id.as_ref().map(AsRef::as_ref))
        .bind(draft.base_revision.map(u64_to_i64).transpose()?)
        .bind(&draft.name)
        .bind(&draft.workspace_path)
        .bind(canonical_json(&draft.messages)?)
        .bind(draft.status.as_str())
        .bind(&draft.last_error)
        .bind(draft.updated_at_ms)
        .bind(&draft.owner_user_id)
        .bind(draft.draft_id.as_ref())
        .bind(u64_to_i64(expected_revision)?)
        .execute(self.pool())
        .await?;
        if result.rows_affected() != 1 {
            return Err(PluginRepositoryError::Conflict);
        }
        self.get_draft(&draft.owner_user_id, &draft.draft_id)
            .await?
            .ok_or(PluginRepositoryError::NotFound)
    }

    async fn delete_draft(
        &self,
        owner_user_id: &str,
        draft_id: &PluginDraftId,
    ) -> PluginRepositoryResult<bool> {
        Ok(sqlx::query("DELETE FROM plugin_drafts WHERE owner_user_id = ? AND draft_id = ?")
            .bind(owner_user_id)
            .bind(draft_id.as_ref())
            .execute(self.pool())
            .await?
            .rows_affected()
            == 1)
    }

    async fn begin_mutation(&self, mutation: &PluginMutationRecord) -> PluginRepositoryResult<()> {
        let mut transaction = self.pool().begin().await?;
        insert_mutation_tx(&mut transaction, mutation).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn update_mutation_phase(
        &self,
        mutation_id: &PluginMutationId,
        phase: PluginMutationPhase,
        error: Option<&str>,
        now_ms: i64,
    ) -> PluginRepositoryResult<()> {
        let result = sqlx::query(
            "UPDATE plugin_mutations SET phase = ?, error = ?, updated_at_ms = ? WHERE mutation_id = ?",
        )
        .bind(phase.as_str())
        .bind(error)
        .bind(now_ms)
        .bind(mutation_id.as_ref())
        .execute(self.pool())
        .await?;
        if result.rows_affected() != 1 {
            return Err(PluginRepositoryError::NotFound);
        }
        Ok(())
    }

    async fn list_mutations(&self) -> PluginRepositoryResult<Vec<PluginMutationRecord>> {
        let rows = sqlx::query(
            "SELECT mutation_id, owner_user_id, plugin_id, kind, phase, old_artifact_digest, \
                    new_artifact_digest, old_data_generation, new_data_generation, expected_revision, \
                    error, created_at_ms, updated_at_ms FROM plugin_mutations ORDER BY created_at_ms",
        )
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(mutation_from_row).collect()
    }

    async fn finish_mutation(&self, mutation_id: &PluginMutationId) -> PluginRepositoryResult<()> {
        let result = sqlx::query("DELETE FROM plugin_mutations WHERE mutation_id = ?")
            .bind(mutation_id.as_ref())
            .execute(self.pool())
            .await?;
        if result.rows_affected() != 1 {
            return Err(PluginRepositoryError::NotFound);
        }
        Ok(())
    }

    async fn commit_install(&self, commit: &InstallCommit) -> PluginRepositoryResult<PluginRecord> {
        let mut transaction = self.pool().begin().await?;
        let mutation = sqlx::query(
            "SELECT phase, old_artifact_digest, old_data_generation FROM plugin_mutations \
             WHERE mutation_id = ? AND owner_user_id = ? AND plugin_id = ?",
        )
        .bind(commit.mutation_id.as_ref())
        .bind(&commit.owner_user_id)
        .bind(commit.plugin_id.as_ref())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(PluginRepositoryError::NotFound)?;
        let phase: String = mutation.try_get("phase")?;
        if phase != PluginMutationPhase::Prepared.as_str() {
            return Err(PluginRepositoryError::Conflict);
        }

        put_artifact_tx(&mut transaction, &commit.artifact).await?;
        let current = get_plugin_tx(&mut transaction, &commit.owner_user_id, &commit.plugin_id).await?;
        match current {
            Some(current) => {
                let expected = commit.expected_revision.ok_or(PluginRepositoryError::Conflict)?;
                if current.revision != expected || current.trashed_at_ms.is_some() {
                    return Err(PluginRepositoryError::Conflict);
                }
                let next_revision = expected.checked_add(1).ok_or_else(|| {
                    PluginRepositoryError::InvalidData("Plugin revision overflow".into())
                })?;
                let changed_generation = current.data_generation != commit.data_generation;
                let result = sqlx::query(
                    "UPDATE plugins SET name = ?, description = ?, previous_artifact_digest = active_artifact_digest, \
                            active_artifact_digest = ?, previous_data_generation = ?, data_generation = ?, \
                            revision = ?, config_json = ?, last_error = NULL, updated_at_ms = ? \
                     WHERE owner_user_id = ? AND plugin_id = ? AND revision = ? AND trashed_at_ms IS NULL",
                )
                .bind(&commit.artifact.artifact.manifest.name)
                .bind(&commit.artifact.artifact.manifest.description)
                .bind(commit.artifact.artifact.artifact_digest.as_ref())
                .bind(changed_generation.then_some(current.data_generation.as_str()))
                .bind(&commit.data_generation)
                .bind(u64_to_i64(next_revision)?)
                .bind(canonical_json(&commit.config)?)
                .bind(commit.now_ms)
                .bind(&commit.owner_user_id)
                .bind(commit.plugin_id.as_ref())
                .bind(u64_to_i64(expected)?)
                .execute(&mut *transaction)
                .await?;
                if result.rows_affected() != 1 {
                    return Err(PluginRepositoryError::Conflict);
                }
            }
            None => {
                if commit.expected_revision.is_some() {
                    return Err(PluginRepositoryError::Conflict);
                }
                let result = sqlx::query(
                    "INSERT INTO plugins \
                     (plugin_id, owner_user_id, package_id, name, description, enabled, trashed_at_ms, \
                      active_artifact_digest, previous_artifact_digest, data_generation, previous_data_generation, \
                      revision, config_json, last_error, created_at_ms, updated_at_ms) \
                     VALUES (?, ?, ?, ?, ?, 1, NULL, ?, NULL, ?, NULL, 1, ?, NULL, ?, ?)",
                )
                .bind(commit.plugin_id.as_ref())
                .bind(&commit.owner_user_id)
                .bind(&commit.package_id)
                .bind(&commit.artifact.artifact.manifest.name)
                .bind(&commit.artifact.artifact.manifest.description)
                .bind(commit.artifact.artifact.artifact_digest.as_ref())
                .bind(&commit.data_generation)
                .bind(canonical_json(&commit.config)?)
                .bind(commit.now_ms)
                .bind(commit.now_ms)
                .execute(&mut *transaction)
                .await;
                if let Err(error) = result {
                    if is_unique_violation(&error) {
                        return Err(PluginRepositoryError::PackageConflict);
                    }
                    return Err(error.into());
                }
            }
        }

        sqlx::query(
            "DELETE FROM plugin_credential_bindings WHERE owner_user_id = ? AND plugin_id = ?",
        )
        .bind(&commit.owner_user_id)
        .bind(commit.plugin_id.as_ref())
        .execute(&mut *transaction)
        .await?;
        for (slot, credential_id) in &commit.credential_bindings {
            sqlx::query(
                "INSERT INTO plugin_credential_bindings \
                 (owner_user_id, plugin_id, slot, credential_id, updated_at_ms) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&commit.owner_user_id)
            .bind(commit.plugin_id.as_ref())
            .bind(slot)
            .bind(credential_id)
            .bind(commit.now_ms)
            .execute(&mut *transaction)
            .await?;
        }

        sqlx::query("DELETE FROM plugin_grants WHERE owner_user_id = ? AND plugin_id = ?")
            .bind(&commit.owner_user_id)
            .bind(commit.plugin_id.as_ref())
            .execute(&mut *transaction)
            .await?;
        for (permission, granted) in &commit.grants {
            sqlx::query(
                "INSERT INTO plugin_grants \
                 (owner_user_id, plugin_id, permission, granted, confirmed_artifact_digest, updated_at_ms) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&commit.owner_user_id)
            .bind(commit.plugin_id.as_ref())
            .bind(permission)
            .bind(*granted)
            .bind(commit.artifact.artifact.artifact_digest.as_ref())
            .bind(commit.now_ms)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query(
            "INSERT INTO plugin_library_state \
             (owner_user_id, plugin_id, pinned, collection, custom_name, last_opened_at_ms, revision) \
             VALUES (?, ?, 0, NULL, NULL, NULL, \
                     COALESCE((SELECT MAX(revision) FROM plugin_library_state WHERE owner_user_id = ?), 1)) \
             ON CONFLICT(owner_user_id, plugin_id) DO NOTHING",
        )
        .bind(&commit.owner_user_id)
        .bind(commit.plugin_id.as_ref())
        .bind(&commit.owner_user_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE plugin_mutations SET phase = 'committed', updated_at_ms = ? WHERE mutation_id = ?",
        )
        .bind(commit.now_ms)
        .bind(commit.mutation_id.as_ref())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.get_plugin(&commit.owner_user_id, &commit.plugin_id)
            .await?
            .ok_or_else(|| PluginRepositoryError::InvalidData("committed Plugin disappeared".into()))
    }

    async fn rollback_install(
        &self,
        mutation_id: &PluginMutationId,
        owner_user_id: &str,
        plugin_id: &PluginId,
        now_ms: i64,
    ) -> PluginRepositoryResult<PluginRecord> {
        let mut transaction = self.pool().begin().await?;
        let mutation = sqlx::query(
            "SELECT phase, kind, old_artifact_digest, old_data_generation, expected_revision, \
                    old_previous_artifact_digest, old_previous_data_generation, old_config_json, \
                    old_credential_bindings_json, old_grants_json \
             FROM plugin_mutations WHERE mutation_id = ? AND owner_user_id = ? AND plugin_id = ?",
        )
        .bind(mutation_id.as_ref())
        .bind(owner_user_id)
        .bind(plugin_id.as_ref())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(PluginRepositoryError::NotFound)?;
        let phase: String = mutation.try_get("phase")?;
        let kind: String = mutation.try_get("kind")?;
        if !matches!(
            phase.as_str(),
            "committed" | "rolling_back" | "failed"
        ) {
            return Err(PluginRepositoryError::Conflict);
        }
        if kind == PluginMutationKind::Install.as_str() {
            sqlx::query("DELETE FROM plugins WHERE owner_user_id = ? AND plugin_id = ?")
                .bind(owner_user_id)
                .bind(plugin_id.as_ref())
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DELETE FROM plugin_mutations WHERE mutation_id = ?")
                .bind(mutation_id.as_ref())
                .execute(&mut *transaction)
                .await?;
            transaction.commit().await?;
            return Err(PluginRepositoryError::NotFound);
        }
        let old_artifact: String = mutation.try_get("old_artifact_digest")?;
        let old_generation: String = mutation.try_get("old_data_generation")?;
        let old_previous_artifact: Option<String> =
            mutation.try_get("old_previous_artifact_digest")?;
        let old_previous_generation: Option<String> =
            mutation.try_get("old_previous_data_generation")?;
        let old_config_json: String = mutation.try_get("old_config_json")?;
        let old_credential_bindings_json: String =
            mutation.try_get("old_credential_bindings_json")?;
        let old_grants_json: String = mutation.try_get("old_grants_json")?;
        let old_credential_bindings = serde_json::from_str::<Vec<PluginCredentialBinding>>(
            &old_credential_bindings_json,
        )
        .map_err(|error| {
            PluginRepositoryError::InvalidData(format!(
                "plugin_mutations.old_credential_bindings_json: {error}"
            ))
        })?;
        let old_grants = serde_json::from_str::<Vec<PluginGrant>>(&old_grants_json).map_err(
            |error| {
                PluginRepositoryError::InvalidData(format!(
                    "plugin_mutations.old_grants_json: {error}"
                ))
            },
        )?;
        if old_credential_bindings
            .iter()
            .any(|binding| &binding.plugin_id != plugin_id)
            || old_grants
                .iter()
                .any(|grant| &grant.plugin_id != plugin_id)
        {
            return Err(PluginRepositoryError::InvalidData(
                "Plugin mutation rollback snapshot identity mismatch".into(),
            ));
        }
        let old_manifest_json = sqlx::query_scalar::<_, String>(
            "SELECT manifest_json FROM plugin_artifacts WHERE artifact_digest = ?",
        )
        .bind(&old_artifact)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| {
            PluginRepositoryError::InvalidData(
                "Plugin mutation old Artifact is absent from the Artifact Store".into(),
            )
        })?;
        let old_manifest = serde_json::from_str::<PluginManifest>(&old_manifest_json).map_err(
            |error| {
                PluginRepositoryError::InvalidData(format!(
                    "plugin_mutations old Artifact manifest: {error}"
                ))
            },
        )?;
        let expected_revision: i64 = mutation.try_get("expected_revision")?;
        let committed_revision = expected_revision.checked_add(1).ok_or_else(|| {
            PluginRepositoryError::InvalidData("Plugin revision overflow".into())
        })?;
        let result = sqlx::query(
            "UPDATE plugins SET name = ?, description = ?, active_artifact_digest = ?, \
                    data_generation = ?, previous_artifact_digest = ?, previous_data_generation = ?, \
                    config_json = ?, revision = revision + 1, last_error = ?, updated_at_ms = ? \
             WHERE owner_user_id = ? AND plugin_id = ? AND revision = ?",
        )
        .bind(&old_manifest.name)
        .bind(&old_manifest.description)
        .bind(old_artifact)
        .bind(old_generation)
        .bind(old_previous_artifact)
        .bind(old_previous_generation)
        .bind(old_config_json)
        .bind("new Plugin runtime failed; restored previous Artifact and DataRoot")
        .bind(now_ms)
        .bind(owner_user_id)
        .bind(plugin_id.as_ref())
        .bind(committed_revision)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() != 1 {
            return Err(PluginRepositoryError::Conflict);
        }
        sqlx::query(
            "DELETE FROM plugin_credential_bindings WHERE owner_user_id = ? AND plugin_id = ?",
        )
        .bind(owner_user_id)
        .bind(plugin_id.as_ref())
        .execute(&mut *transaction)
        .await?;
        for binding in old_credential_bindings {
            sqlx::query(
                "INSERT INTO plugin_credential_bindings \
                 (owner_user_id, plugin_id, slot, credential_id, updated_at_ms) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(owner_user_id)
            .bind(plugin_id.as_ref())
            .bind(binding.slot)
            .bind(binding.credential_id)
            .bind(binding.updated_at_ms)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query("DELETE FROM plugin_grants WHERE owner_user_id = ? AND plugin_id = ?")
            .bind(owner_user_id)
            .bind(plugin_id.as_ref())
            .execute(&mut *transaction)
            .await?;
        for grant in old_grants {
            sqlx::query(
                "INSERT INTO plugin_grants \
                 (owner_user_id, plugin_id, permission, granted, confirmed_artifact_digest, updated_at_ms) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(owner_user_id)
            .bind(plugin_id.as_ref())
            .bind(grant.permission)
            .bind(grant.granted)
            .bind(grant.confirmed_artifact_digest.as_ref())
            .bind(grant.updated_at_ms)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query("DELETE FROM plugin_mutations WHERE mutation_id = ?")
            .bind(mutation_id.as_ref())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        self.get_plugin(owner_user_id, plugin_id)
            .await?
            .ok_or(PluginRepositoryError::NotFound)
    }

    async fn set_enabled(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
        expected_revision: u64,
        enabled: bool,
        now_ms: i64,
    ) -> PluginRepositoryResult<PluginRecord> {
        let result = sqlx::query(
            "UPDATE plugins SET enabled = ?, revision = revision + 1, updated_at_ms = ? \
             WHERE owner_user_id = ? AND plugin_id = ? AND revision = ? AND trashed_at_ms IS NULL",
        )
        .bind(enabled)
        .bind(now_ms)
        .bind(owner_user_id)
        .bind(plugin_id.as_ref())
        .bind(u64_to_i64(expected_revision)?)
        .execute(self.pool())
        .await?;
        if result.rows_affected() != 1 {
            return Err(PluginRepositoryError::Conflict);
        }
        self.get_plugin(owner_user_id, plugin_id)
            .await?
            .ok_or(PluginRepositoryError::NotFound)
    }

    async fn set_last_error(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
        error: Option<&str>,
        now_ms: i64,
    ) -> PluginRepositoryResult<PluginRecord> {
        let result = sqlx::query(
            "UPDATE plugins SET last_error = ?, updated_at_ms = ? \
             WHERE owner_user_id = ? AND plugin_id = ?",
        )
        .bind(error)
        .bind(now_ms)
        .bind(owner_user_id)
        .bind(plugin_id.as_ref())
        .execute(self.pool())
        .await?;
        if result.rows_affected() != 1 {
            return Err(PluginRepositoryError::NotFound);
        }
        self.get_plugin(owner_user_id, plugin_id)
            .await?
            .ok_or(PluginRepositoryError::NotFound)
    }

    async fn set_config(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
        expected_revision: u64,
        config: &Value,
        credential_bindings: &BTreeMap<String, String>,
        grants: &BTreeMap<String, bool>,
        now_ms: i64,
    ) -> PluginRepositoryResult<PluginRecord> {
        let mut transaction = self.pool().begin().await?;
        let result = sqlx::query(
            "UPDATE plugins SET config_json = ?, revision = revision + 1, updated_at_ms = ? \
             WHERE owner_user_id = ? AND plugin_id = ? AND revision = ? AND trashed_at_ms IS NULL",
        )
        .bind(canonical_json(config)?)
        .bind(now_ms)
        .bind(owner_user_id)
        .bind(plugin_id.as_ref())
        .bind(u64_to_i64(expected_revision)?)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() != 1 {
            return Err(PluginRepositoryError::Conflict);
        }
        let artifact_digest: String = sqlx::query_scalar(
            "SELECT active_artifact_digest FROM plugins WHERE owner_user_id = ? AND plugin_id = ?",
        )
        .bind(owner_user_id)
        .bind(plugin_id.as_ref())
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query("DELETE FROM plugin_credential_bindings WHERE owner_user_id = ? AND plugin_id = ?")
            .bind(owner_user_id)
            .bind(plugin_id.as_ref())
            .execute(&mut *transaction)
            .await?;
        for (slot, credential_id) in credential_bindings {
            sqlx::query(
                "INSERT INTO plugin_credential_bindings \
                 (owner_user_id, plugin_id, slot, credential_id, updated_at_ms) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(owner_user_id)
            .bind(plugin_id.as_ref())
            .bind(slot)
            .bind(credential_id)
            .bind(now_ms)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query("DELETE FROM plugin_grants WHERE owner_user_id = ? AND plugin_id = ?")
            .bind(owner_user_id)
            .bind(plugin_id.as_ref())
            .execute(&mut *transaction)
            .await?;
        for (permission, granted) in grants {
            sqlx::query(
                "INSERT INTO plugin_grants \
                 (owner_user_id, plugin_id, permission, granted, confirmed_artifact_digest, updated_at_ms) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(owner_user_id)
            .bind(plugin_id.as_ref())
            .bind(permission)
            .bind(*granted)
            .bind(&artifact_digest)
            .bind(now_ms)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        self.get_plugin(owner_user_id, plugin_id)
            .await?
            .ok_or(PluginRepositoryError::NotFound)
    }

    async fn trash(
        &self,
        owner_user_id: &str,
        plugin_id: &PluginId,
        expected_revision: u64,
        trashed: bool,
        now_ms: i64,
    ) -> PluginRepositoryResult<PluginRecord> {
        let result = sqlx::query(
            "UPDATE plugins SET enabled = CASE WHEN ? THEN 0 ELSE enabled END, trashed_at_ms = ?, \
                    revision = revision + 1, updated_at_ms = ? \
             WHERE owner_user_id = ? AND plugin_id = ? AND revision = ?",
        )
        .bind(trashed)
        .bind(trashed.then_some(now_ms))
        .bind(now_ms)
        .bind(owner_user_id)
        .bind(plugin_id.as_ref())
        .bind(u64_to_i64(expected_revision)?)
        .execute(self.pool())
        .await?;
        if result.rows_affected() != 1 {
            return Err(PluginRepositoryError::Conflict);
        }
        self.get_plugin(owner_user_id, plugin_id)
            .await?
            .ok_or(PluginRepositoryError::NotFound)
    }

    async fn restore_previous(
        &self,
        mutation: &PluginMutationRecord,
        expected_revision: u64,
        restore_data: bool,
        now_ms: i64,
    ) -> PluginRepositoryResult<PluginRecord> {
        let mut transaction = self.pool().begin().await?;
        insert_mutation_tx(&mut transaction, mutation).await?;
        let current = get_plugin_tx(&mut transaction, &mutation.owner_user_id, &mutation.plugin_id)
            .await?
            .ok_or(PluginRepositoryError::NotFound)?;
        if current.revision != expected_revision || current.trashed_at_ms.is_some() {
            return Err(PluginRepositoryError::Conflict);
        }
        let previous_artifact = current
            .previous_artifact_digest
            .clone()
            .ok_or(PluginRepositoryError::Conflict)?;
        let restored_generation = if restore_data {
            current
                .previous_data_generation
                .clone()
                .ok_or(PluginRepositoryError::Conflict)?
        } else {
            current.data_generation.clone()
        };
        if mutation.kind != PluginMutationKind::Restore
            || mutation.phase != PluginMutationPhase::Prepared
            || mutation.expected_revision != Some(expected_revision)
            || mutation.old_artifact_digest.as_ref()
                != Some(&current.active_artifact_digest)
            || mutation.new_artifact_digest.as_ref() != Some(&previous_artifact)
            || mutation.old_data_generation.as_deref()
                != Some(current.data_generation.as_str())
            || mutation.new_data_generation.as_deref()
                != Some(restored_generation.as_str())
        {
            return Err(PluginRepositoryError::Conflict);
        }
        let result = sqlx::query(
            "UPDATE plugins SET active_artifact_digest = ?, previous_artifact_digest = active_artifact_digest, \
                    data_generation = ?, previous_data_generation = CASE WHEN ? = data_generation THEN NULL ELSE data_generation END, \
                    revision = revision + 1, last_error = NULL, updated_at_ms = ? \
             WHERE owner_user_id = ? AND plugin_id = ? AND revision = ?",
        )
        .bind(previous_artifact.as_ref())
        .bind(&restored_generation)
        .bind(&restored_generation)
        .bind(now_ms)
        .bind(&mutation.owner_user_id)
        .bind(mutation.plugin_id.as_ref())
        .bind(u64_to_i64(expected_revision)?)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() != 1 {
            return Err(PluginRepositoryError::Conflict);
        }
        sqlx::query("UPDATE plugin_mutations SET phase = 'committed', updated_at_ms = ? WHERE mutation_id = ?")
            .bind(now_ms)
            .bind(mutation.mutation_id.as_ref())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        self.get_plugin(&mutation.owner_user_id, &mutation.plugin_id)
            .await?
            .ok_or(PluginRepositoryError::NotFound)
    }

    async fn delete_plugin_rows(
        &self,
        mutation_id: &PluginMutationId,
        owner_user_id: &str,
        plugin_id: &PluginId,
        expected_revision: u64,
    ) -> PluginRepositoryResult<()> {
        let mut transaction = self.pool().begin().await?;
        let current = get_plugin_tx(&mut transaction, owner_user_id, plugin_id)
            .await?
            .ok_or(PluginRepositoryError::NotFound)?;
        if current.revision != expected_revision || current.trashed_at_ms.is_none() {
            return Err(PluginRepositoryError::Conflict);
        }
        let mutation = sqlx::query(
            "SELECT phase, kind, old_artifact_digest, new_artifact_digest, \
                    old_data_generation, new_data_generation, expected_revision, updated_at_ms \
             FROM plugin_mutations \
             WHERE mutation_id = ? AND owner_user_id = ? AND plugin_id = ?",
        )
        .bind(mutation_id.as_ref())
        .bind(owner_user_id)
        .bind(plugin_id.as_ref())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(PluginRepositoryError::NotFound)?;
        let phase: String = mutation.try_get("phase")?;
        let kind: String = mutation.try_get("kind")?;
        let old_artifact: Option<String> = mutation.try_get("old_artifact_digest")?;
        let new_artifact: Option<String> = mutation.try_get("new_artifact_digest")?;
        let old_generation: Option<String> = mutation.try_get("old_data_generation")?;
        let new_generation: Option<String> = mutation.try_get("new_data_generation")?;
        let journal_revision: Option<i64> = mutation.try_get("expected_revision")?;
        let journal_updated_at_ms: i64 = mutation.try_get("updated_at_ms")?;
        if phase != PluginMutationPhase::Prepared.as_str()
            || kind != PluginMutationKind::PermanentDelete.as_str()
            || old_artifact.as_deref() != Some(current.active_artifact_digest.as_ref())
            || new_artifact.is_some()
            || old_generation.as_deref() != Some(current.data_generation.as_str())
            || new_generation.is_some()
            || journal_revision != Some(u64_to_i64(expected_revision)?)
        {
            return Err(PluginRepositoryError::Conflict);
        }
        sqlx::query(
            "UPDATE plugin_drafts SET plugin_id = NULL, base_revision = NULL, \
                    revision = revision + 1, updated_at_ms = ? \
             WHERE owner_user_id = ? AND plugin_id = ?",
        )
        .bind(journal_updated_at_ms)
        .bind(owner_user_id)
        .bind(plugin_id.as_ref())
        .execute(&mut *transaction)
        .await?;
        let result = sqlx::query("DELETE FROM plugins WHERE owner_user_id = ? AND plugin_id = ? AND revision = ?")
            .bind(owner_user_id)
            .bind(plugin_id.as_ref())
            .bind(u64_to_i64(expected_revision)?)
            .execute(&mut *transaction)
            .await?;
        if result.rows_affected() != 1 {
            return Err(PluginRepositoryError::Conflict);
        }
        sqlx::query("UPDATE plugin_mutations SET phase = 'committed' WHERE mutation_id = ?")
            .bind(mutation_id.as_ref())
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn list_library_states(
        &self,
        owner_user_id: &str,
    ) -> PluginRepositoryResult<Vec<PluginLibraryState>> {
        let rows = sqlx::query(
            "SELECT plugin_id, pinned, collection, custom_name, last_opened_at_ms, revision \
             FROM plugin_library_state WHERE owner_user_id = ? ORDER BY plugin_id",
        )
        .bind(owner_user_id)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(library_state_from_row).collect()
    }

    async fn replace_library_states(
        &self,
        owner_user_id: &str,
        expected_revision: u64,
        states: &[PluginLibraryState],
    ) -> PluginRepositoryResult<u64> {
        let mut transaction = self.pool().begin().await?;
        let current: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(revision), 0) FROM plugin_library_state WHERE owner_user_id = ?",
        )
        .bind(owner_user_id)
        .fetch_one(&mut *transaction)
        .await?;
        if nonnegative_u64(current, "plugin_library_state.revision")? != expected_revision {
            return Err(PluginRepositoryError::Conflict);
        }
        let installed = sqlx::query_scalar::<_, String>(
            "SELECT plugin_id FROM plugins WHERE owner_user_id = ? ORDER BY plugin_id",
        )
        .bind(owner_user_id)
        .fetch_all(&mut *transaction)
        .await?
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
        let supplied = states
            .iter()
            .map(|state| state.plugin_id.as_ref().to_owned())
            .collect::<std::collections::BTreeSet<_>>();
        if supplied.len() != states.len() {
            return Err(PluginRepositoryError::InvalidData(
                "library state contains a duplicate Plugin".into(),
            ));
        }
        if supplied != installed {
            return Err(PluginRepositoryError::Conflict);
        }
        if states.is_empty() {
            transaction.commit().await?;
            return Ok(expected_revision);
        }
        let next = expected_revision.checked_add(1).ok_or_else(|| {
            PluginRepositoryError::InvalidData("library revision overflow".into())
        })?;
        for state in states {
            if state.collection.as_ref().is_some_and(|value| value.trim().is_empty())
                || state.custom_name.as_ref().is_some_and(|value| value.trim().is_empty())
                || state.last_opened_at_ms.is_some_and(|value| value <= 0)
            {
                return Err(PluginRepositoryError::InvalidData(
                    "library state contains an invalid value".into(),
                ));
            }
            let result = sqlx::query(
                "UPDATE plugin_library_state SET pinned = ?, collection = ?, custom_name = ?, \
                        last_opened_at_ms = ?, revision = ? \
                 WHERE owner_user_id = ? AND plugin_id = ?",
            )
            .bind(state.pinned)
            .bind(&state.collection)
            .bind(&state.custom_name)
            .bind(state.last_opened_at_ms)
            .bind(u64_to_i64(next)?)
            .bind(owner_user_id)
            .bind(state.plugin_id.as_ref())
            .execute(&mut *transaction)
            .await?;
            if result.rows_affected() != 1 {
                return Err(PluginRepositoryError::Conflict);
            }
        }
        transaction.commit().await?;
        Ok(next)
    }
}

const PLUGIN_SELECT: &str =
    "SELECT owner_user_id, plugin_id, package_id, name, description, enabled, trashed_at_ms, \
            active_artifact_digest, previous_artifact_digest, data_generation, previous_data_generation, \
            revision, config_json, last_error, created_at_ms, updated_at_ms FROM plugins";

fn plugin_from_row(row: sqlx::sqlite::SqliteRow) -> PluginRepositoryResult<PluginRecord> {
    Ok(PluginRecord {
        owner_user_id: row.try_get("owner_user_id")?,
        plugin_id: PluginId::from(row.try_get::<String, _>("plugin_id")?),
        package_id: row.try_get("package_id")?,
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        enabled: row.try_get("enabled")?,
        trashed_at_ms: row.try_get("trashed_at_ms")?,
        active_artifact_digest: DigestHex::from(row.try_get::<String, _>("active_artifact_digest")?),
        previous_artifact_digest: row
            .try_get::<Option<String>, _>("previous_artifact_digest")?
            .map(DigestHex::from),
        data_generation: row.try_get("data_generation")?,
        previous_data_generation: row.try_get("previous_data_generation")?,
        revision: positive_u64(row.try_get("revision")?, "plugins.revision")?,
        config: parse_json(&row.try_get::<String, _>("config_json")?, "plugins.config_json")?,
        last_error: row.try_get("last_error")?,
        created_at_ms: row.try_get("created_at_ms")?,
        updated_at_ms: row.try_get("updated_at_ms")?,
    })
}

fn artifact_from_row(row: sqlx::sqlite::SqliteRow) -> PluginRepositoryResult<StoredArtifactRecord> {
    let manifest = serde_json::from_str(&row.try_get::<String, _>("manifest_json")?)
        .map_err(|error| PluginRepositoryError::InvalidData(error.to_string()))?;
    let files = serde_json::from_str(&row.try_get::<String, _>("files_json")?)
        .map_err(|error| PluginRepositoryError::InvalidData(error.to_string()))?;
    let artifact = PluginArtifact {
        artifact_digest: DigestHex::from(row.try_get::<String, _>("artifact_digest")?),
        manifest,
        files,
    };
    artifact
        .validate()
        .map_err(|error| PluginRepositoryError::InvalidData(error.to_string()))?;
    Ok(StoredArtifactRecord {
        artifact,
        artifact_root: row.try_get("artifact_root")?,
        created_at_ms: row.try_get("created_at_ms")?,
    })
}

fn draft_from_row(row: sqlx::sqlite::SqliteRow) -> PluginRepositoryResult<PluginDraftRecord> {
    let status = match row.try_get::<String, _>("status")?.as_str() {
        "ready" => PluginDraftStatus::Ready,
        "generating" => PluginDraftStatus::Generating,
        "failed" => PluginDraftStatus::Failed,
        other => return Err(PluginRepositoryError::InvalidData(format!("invalid Draft status {other}"))),
    };
    Ok(PluginDraftRecord {
        owner_user_id: row.try_get("owner_user_id")?,
        draft_id: PluginDraftId::from(row.try_get::<String, _>("draft_id")?),
        revision: positive_u64(row.try_get("revision")?, "plugin_drafts.revision")?,
        plugin_id: row.try_get::<Option<String>, _>("plugin_id")?.map(PluginId::from),
        base_revision: row
            .try_get::<Option<i64>, _>("base_revision")?
            .map(|value| positive_u64(value, "plugin_drafts.base_revision"))
            .transpose()?,
        name: row.try_get("name")?,
        workspace_path: row.try_get("workspace_path")?,
        messages: serde_json::from_str::<Vec<PluginDraftMessage>>(
            &row.try_get::<String, _>("messages_json")?,
        )
        .map_err(|error| PluginRepositoryError::InvalidData(error.to_string()))?,
        status,
        last_error: row.try_get("last_error")?,
        created_at_ms: row.try_get("created_at_ms")?,
        updated_at_ms: row.try_get("updated_at_ms")?,
    })
}

fn mutation_from_row(row: sqlx::sqlite::SqliteRow) -> PluginRepositoryResult<PluginMutationRecord> {
    let kind = match row.try_get::<String, _>("kind")?.as_str() {
        "install" => PluginMutationKind::Install,
        "update" => PluginMutationKind::Update,
        "restore" => PluginMutationKind::Restore,
        "permanent_delete" => PluginMutationKind::PermanentDelete,
        other => return Err(PluginRepositoryError::InvalidData(format!("invalid mutation kind {other}"))),
    };
    let phase = match row.try_get::<String, _>("phase")?.as_str() {
        "staging" => PluginMutationPhase::Staging,
        "prepared" => PluginMutationPhase::Prepared,
        "committed" => PluginMutationPhase::Committed,
        "rolling_back" => PluginMutationPhase::RollingBack,
        "failed" => PluginMutationPhase::Failed,
        other => return Err(PluginRepositoryError::InvalidData(format!("invalid mutation phase {other}"))),
    };
    Ok(PluginMutationRecord {
        mutation_id: PluginMutationId::from(row.try_get::<String, _>("mutation_id")?),
        owner_user_id: row.try_get("owner_user_id")?,
        plugin_id: PluginId::from(row.try_get::<String, _>("plugin_id")?),
        kind,
        phase,
        old_artifact_digest: row.try_get::<Option<String>, _>("old_artifact_digest")?.map(DigestHex::from),
        new_artifact_digest: row.try_get::<Option<String>, _>("new_artifact_digest")?.map(DigestHex::from),
        old_data_generation: row.try_get("old_data_generation")?,
        new_data_generation: row.try_get("new_data_generation")?,
        expected_revision: row
            .try_get::<Option<i64>, _>("expected_revision")?
            .map(|value| nonnegative_u64(value, "plugin_mutations.expected_revision"))
            .transpose()?,
        error: row.try_get("error")?,
        created_at_ms: row.try_get("created_at_ms")?,
        updated_at_ms: row.try_get("updated_at_ms")?,
    })
}

async fn credential_bindings(
    pool: &SqlitePool,
    owner_user_id: &str,
    plugin_id: &PluginId,
) -> PluginRepositoryResult<BTreeMap<String, String>> {
    let rows = sqlx::query(
        "SELECT slot, credential_id FROM plugin_credential_bindings \
         WHERE owner_user_id = ? AND plugin_id = ? ORDER BY slot",
    )
    .bind(owner_user_id)
    .bind(plugin_id.as_ref())
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| Ok((row.try_get("slot")?, row.try_get("credential_id")?)))
        .collect()
}

async fn grants(
    pool: &SqlitePool,
    owner_user_id: &str,
    plugin_id: &PluginId,
) -> PluginRepositoryResult<BTreeMap<String, PluginGrant>> {
    let rows = sqlx::query(
        "SELECT permission, granted, confirmed_artifact_digest, updated_at_ms FROM plugin_grants \
         WHERE owner_user_id = ? AND plugin_id = ? ORDER BY permission",
    )
    .bind(owner_user_id)
    .bind(plugin_id.as_ref())
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            let permission: String = row.try_get("permission")?;
            Ok((
                permission.clone(),
                PluginGrant {
                    plugin_id: plugin_id.clone(),
                    permission,
                    granted: row.try_get("granted")?,
                    confirmed_artifact_digest: DigestHex::from(
                        row.try_get::<String, _>("confirmed_artifact_digest")?,
                    ),
                    updated_at_ms: row.try_get("updated_at_ms")?,
                },
            ))
        })
        .collect()
}

async fn library_state(
    pool: &SqlitePool,
    owner_user_id: &str,
    plugin_id: &PluginId,
) -> PluginRepositoryResult<PluginLibraryState> {
    let row = sqlx::query(
        "SELECT plugin_id, pinned, collection, custom_name, last_opened_at_ms, revision FROM plugin_library_state \
         WHERE owner_user_id = ? AND plugin_id = ?",
    )
    .bind(owner_user_id)
    .bind(plugin_id.as_ref())
    .fetch_optional(pool)
    .await?;
    Ok(match row {
        Some(row) => library_state_from_row(row)?,
        None => PluginLibraryState {
            plugin_id: plugin_id.clone(),
            pinned: false,
            collection: None,
            custom_name: None,
            last_opened_at_ms: None,
            revision: 1,
        },
    })
}

fn library_state_from_row(
    row: sqlx::sqlite::SqliteRow,
) -> PluginRepositoryResult<PluginLibraryState> {
    Ok(PluginLibraryState {
        plugin_id: PluginId::from(row.try_get::<String, _>("plugin_id")?),
        pinned: row.try_get("pinned")?,
        collection: row.try_get("collection")?,
        custom_name: row.try_get("custom_name")?,
        last_opened_at_ms: row.try_get("last_opened_at_ms")?,
        revision: positive_u64(row.try_get("revision")?, "plugin_library_state.revision")?,
    })
}

async fn put_artifact_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    artifact: &StoredArtifactRecord,
) -> PluginRepositoryResult<()> {
    artifact.artifact.validate().map_err(|error| {
        PluginRepositoryError::InvalidData(format!("invalid Artifact: {error}"))
    })?;
    sqlx::query(
        "INSERT INTO plugin_artifacts \
         (artifact_digest, package_id, version, manifest_json, files_json, artifact_root, \
          has_ui, has_service, data_version, created_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(artifact_digest) DO NOTHING",
    )
    .bind(artifact.artifact.artifact_digest.as_ref())
    .bind(&artifact.artifact.manifest.id)
    .bind(&artifact.artifact.manifest.version)
    .bind(canonical_json(&artifact.artifact.manifest)?)
    .bind(canonical_json(&artifact.artifact.files)?)
    .bind(&artifact.artifact_root)
    .bind(artifact.artifact.manifest.has_ui())
    .bind(artifact.artifact.manifest.has_service())
    .bind(i64::from(artifact.artifact.manifest.data_version))
    .bind(artifact.created_at_ms)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn get_plugin_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    owner_user_id: &str,
    plugin_id: &PluginId,
) -> PluginRepositoryResult<Option<PluginRecord>> {
    let sql = format!("{PLUGIN_SELECT} WHERE owner_user_id = ? AND plugin_id = ?");
    sqlx::query(&sql)
        .bind(owner_user_id)
        .bind(plugin_id.as_ref())
        .fetch_optional(&mut **transaction)
        .await?
        .map(plugin_from_row)
        .transpose()
}

async fn insert_mutation_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    mutation: &PluginMutationRecord,
) -> PluginRepositoryResult<()> {
    let rollback = mutation_rollback_snapshot_tx(
        transaction,
        &mutation.owner_user_id,
        &mutation.plugin_id,
    )
    .await?;
    sqlx::query(
        "INSERT INTO plugin_mutations \
         (mutation_id, owner_user_id, plugin_id, kind, phase, old_artifact_digest, new_artifact_digest, \
          old_data_generation, old_previous_artifact_digest, old_previous_data_generation, \
          old_config_json, old_credential_bindings_json, old_grants_json, new_data_generation, \
          expected_revision, error, created_at_ms, updated_at_ms) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(mutation.mutation_id.as_ref())
    .bind(&mutation.owner_user_id)
    .bind(mutation.plugin_id.as_ref())
    .bind(mutation.kind.as_str())
    .bind(mutation.phase.as_str())
    .bind(mutation.old_artifact_digest.as_ref().map(AsRef::as_ref))
    .bind(mutation.new_artifact_digest.as_ref().map(AsRef::as_ref))
    .bind(&mutation.old_data_generation)
    .bind(
        rollback
            .as_ref()
            .and_then(|snapshot| snapshot.previous_artifact_digest.as_deref()),
    )
    .bind(
        rollback
            .as_ref()
            .and_then(|snapshot| snapshot.previous_data_generation.as_deref()),
    )
    .bind(rollback.as_ref().map(|snapshot| snapshot.config_json.as_str()))
    .bind(
        rollback
            .as_ref()
            .map(|snapshot| snapshot.credential_bindings_json.as_str()),
    )
    .bind(rollback.as_ref().map(|snapshot| snapshot.grants_json.as_str()))
    .bind(&mutation.new_data_generation)
    .bind(mutation.expected_revision.map(u64_to_i64).transpose()?)
    .bind(&mutation.error)
    .bind(mutation.created_at_ms)
    .bind(mutation.updated_at_ms)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn mutation_rollback_snapshot_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    owner_user_id: &str,
    plugin_id: &PluginId,
) -> PluginRepositoryResult<Option<MutationRollbackSnapshot>> {
    let plugin = sqlx::query(
        "SELECT previous_artifact_digest, previous_data_generation, config_json \
         FROM plugins WHERE owner_user_id = ? AND plugin_id = ?",
    )
    .bind(owner_user_id)
    .bind(plugin_id.as_ref())
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(plugin) = plugin else {
        return Ok(None);
    };

    let credential_bindings = sqlx::query(
        "SELECT slot, credential_id, updated_at_ms FROM plugin_credential_bindings \
         WHERE owner_user_id = ? AND plugin_id = ? ORDER BY slot",
    )
    .bind(owner_user_id)
    .bind(plugin_id.as_ref())
    .fetch_all(&mut **transaction)
    .await?
    .into_iter()
    .map(|row| {
        Ok(PluginCredentialBinding {
            plugin_id: plugin_id.clone(),
            slot: row.try_get("slot")?,
            credential_id: row.try_get("credential_id")?,
            updated_at_ms: row.try_get("updated_at_ms")?,
        })
    })
    .collect::<PluginRepositoryResult<Vec<_>>>()?;
    let grants = sqlx::query(
        "SELECT permission, granted, confirmed_artifact_digest, updated_at_ms FROM plugin_grants \
         WHERE owner_user_id = ? AND plugin_id = ? ORDER BY permission",
    )
    .bind(owner_user_id)
    .bind(plugin_id.as_ref())
    .fetch_all(&mut **transaction)
    .await?
    .into_iter()
    .map(|row| {
        Ok(PluginGrant {
            plugin_id: plugin_id.clone(),
            permission: row.try_get("permission")?,
            granted: row.try_get("granted")?,
            confirmed_artifact_digest: DigestHex::from(
                row.try_get::<String, _>("confirmed_artifact_digest")?,
            ),
            updated_at_ms: row.try_get("updated_at_ms")?,
        })
    })
    .collect::<PluginRepositoryResult<Vec<_>>>()?;

    Ok(Some(MutationRollbackSnapshot {
        previous_artifact_digest: plugin.try_get("previous_artifact_digest")?,
        previous_data_generation: plugin.try_get("previous_data_generation")?,
        config_json: plugin.try_get("config_json")?,
        credential_bindings_json: canonical_json(&credential_bindings)?,
        grants_json: canonical_json(&grants)?,
    }))
}

fn canonical_json<T: serde::Serialize>(value: &T) -> PluginRepositoryResult<String> {
    let bytes = nomifun_agent_contracts::canonical_json_bytes(value)
        .map_err(|error| PluginRepositoryError::InvalidData(error.to_string()))?;
    String::from_utf8(bytes).map_err(|error| PluginRepositoryError::InvalidData(error.to_string()))
}

fn parse_json(value: &str, field: &str) -> PluginRepositoryResult<Value> {
    serde_json::from_str(value)
        .map_err(|error| PluginRepositoryError::InvalidData(format!("{field}: {error}")))
}

fn positive_u64(value: i64, field: &str) -> PluginRepositoryResult<u64> {
    if value <= 0 {
        return Err(PluginRepositoryError::InvalidData(format!("{field} must be positive")));
    }
    Ok(value as u64)
}

fn nonnegative_u64(value: i64, field: &str) -> PluginRepositoryResult<u64> {
    if value < 0 {
        return Err(PluginRepositoryError::InvalidData(format!("{field} must be non-negative")));
    }
    Ok(value as u64)
}

fn u64_to_i64(value: u64) -> PluginRepositoryResult<i64> {
    i64::try_from(value)
        .map_err(|_| PluginRepositoryError::InvalidData("integer exceeds SQLite range".into()))
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|error| error.is_unique_violation())
}
