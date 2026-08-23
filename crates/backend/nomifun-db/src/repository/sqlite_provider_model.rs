use nomifun_common::{now_ms, ProviderId};
use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::{NewProviderModel, ProviderModelRow};
use crate::repository::provider_model::{CoordinatedProviderModelDelete, IProviderModelRepository};
use crate::repository::sqlite_provider_model_capability::{
    bump_provider_config_revision_tx, replace_for_model_tx,
};

#[derive(Clone, Debug)]
pub struct SqliteProviderModelRepository {
    pool: SqlitePool,
}

impl SqliteProviderModelRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

async fn lock_parent_provider(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    provider_id: &str,
    expected_config_revision: i64,
) -> Result<ProviderId, DbError> {
    let provider_id = ProviderId::parse(provider_id).map_err(|error| {
        DbError::Conflict(format!(
            "Provider model provider_id '{provider_id}' is not a canonical UUIDv7: {error}"
        ))
    })?;
    let parent = sqlx::query(
        "UPDATE providers SET config_revision = config_revision \
         WHERE provider_id = ? AND config_revision = ?",
    )
    .bind(provider_id.as_str())
    .bind(expected_config_revision)
    .execute(&mut **transaction)
    .await?;
    if parent.rows_affected() == 0 {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM providers WHERE provider_id = ?)")
                .bind(provider_id.as_str())
                .fetch_one(&mut **transaction)
                .await?;
        return if exists {
            Err(DbError::Conflict(format!(
                "provider invocation graph changed while saving model; expected revision {expected_config_revision}"
            )))
        } else {
            Err(DbError::Conflict(format!(
                "Provider model provider '{provider_id}' does not exist"
            )))
        };
    }
    Ok(provider_id)
}

pub(crate) async fn fetch_row(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    provider_id: &str,
    model: &str,
) -> Result<ProviderModelRow, DbError> {
    Ok(sqlx::query_as::<_, ProviderModelRow>(
        "SELECT * FROM provider_models WHERE provider_id = ? AND model = ?",
    )
    .bind(provider_id)
    .bind(model)
    .fetch_one(&mut **transaction)
    .await?)
}

pub(crate) async fn save_model_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    provider_id: &str,
    row: &NewProviderModel<'_>,
    now: i64,
) -> Result<(ProviderModelRow, bool), DbError> {
    if row.capabilities.is_empty() {
        return Err(DbError::Conflict(
            "provider model must have at least one capability".into(),
        ));
    }
    let existing_enabled: Option<bool> = sqlx::query_scalar(
        "SELECT enabled FROM provider_models WHERE provider_id = ? AND model = ?",
    )
    .bind(provider_id)
    .bind(row.model)
    .fetch_optional(&mut **transaction)
    .await?;
    let model_enabled_changed = existing_enabled.is_some_and(|enabled| enabled != row.enabled);

    sqlx::query(
        "INSERT INTO provider_models \
            (provider_id, model, enabled, sort_order, description, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(provider_id, model) DO UPDATE SET \
            enabled = excluded.enabled, sort_order = excluded.sort_order, \
            description = excluded.description, updated_at = excluded.updated_at",
    )
    .bind(provider_id)
    .bind(row.model)
    .bind(row.enabled)
    .bind(row.sort_order)
    .bind(row.description)
    .bind(now)
    .bind(now)
    .execute(&mut **transaction)
    .await?;
    let capability_changed =
        replace_for_model_tx(transaction, provider_id, row.model, row.capabilities, now).await?;
    let stored = fetch_row(transaction, provider_id, row.model).await?;
    Ok((stored, model_enabled_changed || capability_changed))
}

#[async_trait::async_trait]
impl IProviderModelRepository for SqliteProviderModelRepository {
    async fn list(&self) -> Result<Vec<ProviderModelRow>, DbError> {
        Ok(sqlx::query_as::<_, ProviderModelRow>(
            "SELECT * FROM provider_models ORDER BY provider_id ASC, sort_order ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    async fn list_for_provider(&self, provider_id: &str) -> Result<Vec<ProviderModelRow>, DbError> {
        Ok(sqlx::query_as::<_, ProviderModelRow>(
            "SELECT * FROM provider_models WHERE provider_id = ? \
             ORDER BY sort_order ASC, id ASC",
        )
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn get(
        &self,
        provider_id: &str,
        model: &str,
    ) -> Result<Option<ProviderModelRow>, DbError> {
        Ok(sqlx::query_as::<_, ProviderModelRow>(
            "SELECT * FROM provider_models WHERE provider_id = ? AND model = ?",
        )
        .bind(provider_id)
        .bind(model)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn save(
        &self,
        provider_id: &str,
        expected_config_revision: i64,
        row: &NewProviderModel<'_>,
    ) -> Result<ProviderModelRow, DbError> {
        let mut transaction = self.pool.begin().await?;
        let provider_id =
            lock_parent_provider(&mut transaction, provider_id, expected_config_revision).await?;
        let (stored, configuration_changed) =
            save_model_tx(&mut transaction, provider_id.as_str(), row, now_ms()).await?;
        if configuration_changed {
            bump_provider_config_revision_tx(&mut transaction, provider_id.as_str()).await?;
        }
        transaction.commit().await?;
        Ok(stored)
    }

    async fn set_display_name(
        &self,
        provider_id: &str,
        model: &str,
        display_name: Option<&str>,
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE provider_models SET display_name = ?, updated_at = ? \
             WHERE provider_id = ? AND model = ?",
        )
        .bind(display_name)
        .bind(now_ms())
        .bind(provider_id)
        .bind(model)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_coordinated(
        &self,
        plan: &CoordinatedProviderModelDelete,
    ) -> Result<bool, DbError> {
        let mut transaction = self.pool.begin().await?;
        let provider_id = lock_parent_provider(
            &mut transaction,
            &plan.provider_id,
            plan.expected_config_revision,
        )
        .await?;

        let model_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM provider_models WHERE provider_id = ? AND model = ?)",
        )
        .bind(provider_id.as_str())
        .bind(&plan.model)
        .fetch_one(&mut *transaction)
        .await?;
        if !model_exists {
            transaction.commit().await?;
            return Ok(false);
        }

        let live_creation_task: Option<String> = sqlx::query_scalar(
            "SELECT creation_task_id FROM creation_tasks \
             WHERE provider_id = ? AND model = ? AND status IN ('queued', 'running') \
             ORDER BY submitted_at ASC, creation_task_id ASC LIMIT 1",
        )
        .bind(provider_id.as_str())
        .bind(&plan.model)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(task_id) = live_creation_task {
            return Err(DbError::Conflict(format!(
                "provider model '{}/{}' has live creation task '{task_id}'",
                provider_id, plan.model
            )));
        }

        let live_workflow_run: Option<String> = sqlx::query_scalar(
            "SELECT workflow_run_id FROM creative_studio_workflow_runs AS run \
             WHERE run.status IN ('requested', 'awaiting-review', 'queued', 'running') \
               AND EXISTS (\
                   SELECT 1 FROM json_each(run.aggregate_json, '$.workflowSnapshot.steps') AS step \
                   WHERE (\
                       json_extract(step.value, '$.kind') = 'generate-images' \
                       AND json_extract(step.value, '$.generation.model.providerId') = ?1 \
                       AND json_extract(step.value, '$.generation.model.model') = ?2\
                   ) OR (\
                       json_extract(step.value, '$.kind') = 'draft-prompts' \
                       AND json_extract(step.value, '$.planning.model.providerId') = ?1 \
                       AND json_extract(step.value, '$.planning.model.model') = ?2\
                   )\
               ) \
             ORDER BY run.updated_at ASC, run.workflow_run_id ASC LIMIT 1",
        )
        .bind(provider_id.as_str())
        .bind(&plan.model)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(workflow_run_id) = live_workflow_run {
            return Err(DbError::Conflict(format!(
                "provider model '{}/{}' is pinned by nonterminal workflow run '{workflow_run_id}'",
                provider_id, plan.model
            )));
        }

        for cleanup in &plan.cleanup.projects {
            let updated = sqlx::query(
                "UPDATE creative_studio_projects \
                 SET document_json = ?, node_count = ?, connection_count = ?, \
                     revision = revision + 1, updated_at = ? \
                 WHERE project_id = ? AND revision = ?",
            )
            .bind(&cleanup.document_json)
            .bind(cleanup.node_count)
            .bind(cleanup.connection_count)
            .bind(cleanup.updated_at)
            .bind(&cleanup.project_id)
            .bind(cleanup.expected_revision)
            .execute(&mut *transaction)
            .await?;
            if updated.rows_affected() != 1 {
                return Err(DbError::Conflict(format!(
                    "creative studio project '{}' changed during provider model cleanup; expected revision {}",
                    cleanup.project_id, cleanup.expected_revision
                )));
            }
        }

        for cleanup in &plan.cleanup.workflows {
            let replacement = &cleanup.replacement;
            if replacement.workflow_id != cleanup.workflow_id
                || replacement.revision != cleanup.expected_revision + 1
            {
                return Err(DbError::Conflict(format!(
                    "creative studio workflow '{}' cleanup replacement must preserve its ID and increment revision once",
                    cleanup.workflow_id
                )));
            }
            let updated = sqlx::query(
                "UPDATE creative_studio_workflows \
                 SET revision = ?, name = ?, description = ?, category = ?, visibility = ?, \
                     definition_json = ?, updated_at = ? \
                 WHERE workflow_id = ? AND revision = ?",
            )
            .bind(replacement.revision)
            .bind(&replacement.name)
            .bind(&replacement.description)
            .bind(&replacement.category)
            .bind(&replacement.visibility)
            .bind(&replacement.definition_json)
            .bind(replacement.updated_at)
            .bind(&cleanup.workflow_id)
            .bind(cleanup.expected_revision)
            .execute(&mut *transaction)
            .await?;
            if updated.rows_affected() != 1 {
                return Err(DbError::Conflict(format!(
                    "creative studio workflow '{}' changed during provider model cleanup; expected revision {}",
                    cleanup.workflow_id, cleanup.expected_revision
                )));
            }
        }

        sqlx::query(
            "DELETE FROM provider_model_capabilities WHERE provider_id = ? AND model = ?",
        )
        .bind(provider_id.as_str())
        .bind(&plan.model)
        .execute(&mut *transaction)
        .await?;
        let deleted = sqlx::query(
            "DELETE FROM provider_models WHERE provider_id = ? AND model = ?",
        )
        .bind(provider_id.as_str())
        .bind(&plan.model)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if deleted != 1 {
            return Err(DbError::Conflict(format!(
                "provider model '{}/{}' changed during coordinated deletion",
                provider_id, plan.model
            )));
        }
        bump_provider_config_revision_tx(&mut transaction, provider_id.as_str()).await?;
        transaction.commit().await?;
        Ok(true)
    }
}
