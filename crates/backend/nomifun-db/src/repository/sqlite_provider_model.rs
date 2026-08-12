use nomifun_common::{now_ms, ProviderId};
use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::{NewProviderModel, ProviderModelRow};
use crate::repository::provider_model::IProviderModelRepository;
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

    async fn delete(&self, provider_id: &str, model: &str) -> Result<bool, DbError> {
        let mut transaction = self.pool.begin().await?;
        let provider_id = ProviderId::parse(provider_id).map_err(|error| {
            DbError::Conflict(format!(
                "Provider model provider_id '{provider_id}' is not a canonical UUIDv7: {error}"
            ))
        })?;
        let parent = sqlx::query(
            "UPDATE providers SET config_revision = config_revision WHERE provider_id = ?",
        )
        .bind(provider_id.as_str())
        .execute(&mut *transaction)
        .await?;
        if parent.rows_affected() == 0 {
            return Err(DbError::Conflict(format!(
                "Provider model provider '{provider_id}' does not exist"
            )));
        }
        let deleted_capabilities = sqlx::query(
            "DELETE FROM provider_model_capabilities WHERE provider_id = ? AND model = ?",
        )
        .bind(provider_id.as_str())
        .bind(model)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        let deleted =
            sqlx::query("DELETE FROM provider_models WHERE provider_id = ? AND model = ?")
                .bind(provider_id.as_str())
                .bind(model)
                .execute(&mut *transaction)
                .await?
                .rows_affected()
                > 0;
        if deleted || deleted_capabilities > 0 {
            bump_provider_config_revision_tx(&mut transaction, provider_id.as_str()).await?;
        }
        transaction.commit().await?;
        Ok(deleted)
    }
}
