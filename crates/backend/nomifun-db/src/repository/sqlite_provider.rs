use std::collections::HashSet;

use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::{
    NewProviderModel, Provider, ProviderConnectionRow, ProviderModelRow,
    UpsertProviderConnectionParams,
};
use crate::repository::provider::{CreateProviderParams, UpdateProviderParams};
use crate::repository::sqlite_provider_model::{fetch_row, save_model_tx};
use crate::repository::sqlite_provider_model_capability::{
    bump_provider_config_revision_tx, clear_health_for_connection_role_tx,
};
use crate::repository::{
    IProviderRepository,
};

pub(crate) async fn prune_missing_provider_preference(
    key: &str,
    value: String,
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<String, DbError> {
    let mut parsed: serde_json::Value = serde_json::from_str(&value).map_err(|error| {
        DbError::Conflict(format!("invalid client preference '{key}': {error}"))
    })?;
    let items = match key {
        "agent.model_failover" => parsed
            .as_object_mut()
            .and_then(|object| object.get_mut("queue"))
            .and_then(serde_json::Value::as_array_mut),
        "nomi.collaborationModels" => parsed.as_array_mut(),
        _ => None,
    };
    let Some(items) = items else {
        return Ok(value);
    };

    let mut retained = Vec::with_capacity(items.len());
    for item in std::mem::take(items) {
        let Some(provider_id) = item.get("provider_id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM providers WHERE provider_id = ?)")
                .bind(provider_id)
                .fetch_one(&mut **transaction)
                .await?;
        if exists {
            retained.push(item);
        }
    }
    *items = retained;
    Ok(parsed.to_string())
}

#[derive(Clone, Debug)]
pub struct SqliteProviderRepository {
    pool: SqlitePool,
}

impl SqliteProviderRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    async fn delete_graph(&self, id: &str, expected_revision: Option<i64>, cleanup: &crate::ProviderModelCleanupPlan) -> Result<(),DbError> {
        let mut transaction = self.pool.begin().await?;
        // The writer lock fences cleanup, reference writers and catalog removal.
        let locked = sqlx::query("UPDATE providers SET config_revision=config_revision WHERE provider_id=? AND (? IS NULL OR config_revision=?)")
            .bind(id).bind(expected_revision).bind(expected_revision).execute(&mut *transaction).await?;
        if locked.rows_affected()==0 {
            let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM providers WHERE provider_id=?)")
                .bind(id).fetch_one(&mut *transaction).await?;
            return Err(if exists {DbError::Conflict("provider invocation graph changed during deletion".into())}
                else {DbError::NotFound(format!("Provider '{id}' not found"))});
        }
        crate::provider_cleanup::apply_cleanup_plan_tx(&mut transaction,id,None,cleanup).await?;
        for table in ["provider_model_capabilities","provider_models","provider_connections","providers"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE provider_id=?")).bind(id).execute(&mut *transaction).await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}

fn validate_provider_params(params: &CreateProviderParams<'_>) -> Result<(), DbError> {
    if params.auth_scheme.trim().is_empty() {
        return Err(DbError::Conflict(
            "provider auth_scheme must not be blank".into(),
        ));
    }
    Ok(())
}

async fn insert_provider_row_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    params: &CreateProviderParams<'_>,
    now: i64,
) -> Result<Provider, DbError> {
    validate_provider_params(params)?;
    let provider_id = match params.provider_id {
        Some(provider_id) => nomifun_common::ProviderId::parse(provider_id)
            .map(nomifun_common::ProviderId::into_string)
            .map_err(|error| {
                DbError::Conflict(format!("invalid provider_id '{provider_id}': {error}"))
            })?,
        None => nomifun_common::ProviderId::new().into_string(),
    };
    let result = sqlx::query(
        "INSERT INTO providers \
            (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, enabled, \
             bedrock_config, sort_order, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, \
                 COALESCE(?, (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM providers)), ?, ?)",
    )
    .bind(&provider_id)
    .bind(params.platform)
    .bind(params.name)
    .bind(params.base_url)
    .bind(params.auth_scheme.trim())
    .bind(params.credentials_encrypted)
    .bind(params.enabled)
    .bind(params.bedrock_config)
    .bind(params.sort_order)
    .bind(now)
    .bind(now)
    .execute(&mut **transaction)
    .await;
    match result {
        Ok(_) => {}
        Err(sqlx::Error::Database(error))
            if error
                .code()
                .is_some_and(|code| code == "2067" || code == "1555") =>
        {
            return Err(DbError::Conflict(format!(
                "Provider with id '{provider_id}' already exists"
            )));
        }
        Err(error) => return Err(DbError::Query(error)),
    }
    Ok(
        sqlx::query_as::<_, Provider>("SELECT * FROM providers WHERE provider_id = ?")
            .bind(&provider_id)
            .fetch_one(&mut **transaction)
            .await?,
    )
}

async fn fetch_provider_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    provider_id: &str,
) -> Result<Provider, DbError> {
    Ok(
        sqlx::query_as::<_, Provider>("SELECT * FROM providers WHERE provider_id = ?")
            .bind(provider_id)
            .fetch_one(&mut **transaction)
            .await?,
    )
}

async fn lock_provider_revision_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    provider_id: &str,
    expected_config_revision: i64,
) -> Result<(), DbError> {
    let locked = sqlx::query(
        "UPDATE providers SET config_revision = config_revision \
         WHERE provider_id = ? AND config_revision = ?",
    )
    .bind(provider_id)
    .bind(expected_config_revision)
    .execute(&mut **transaction)
    .await?;
    if locked.rows_affected() == 0 {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM providers WHERE provider_id = ?)")
                .bind(provider_id)
                .fetch_one(&mut **transaction)
                .await?;
        return if exists {
            Err(DbError::Conflict(format!(
                "provider invocation graph changed while updating provider; expected revision {expected_config_revision}"
            )))
        } else {
            Err(DbError::NotFound(format!("Provider '{provider_id}' not found")))
        };
    }
    Ok(())
}

fn validate_graph_models(
    models: &[NewProviderModel<'_>],
    display_names: &[Option<String>],
) -> Result<(), DbError> {
    if models.is_empty() {
        return Err(DbError::Conflict("provider graph must have at least one model".into()));
    }
    if models.len() != display_names.len() {
        return Err(DbError::Conflict("provider graph model display names must match the model count".into()));
    }
    let mut identities = HashSet::with_capacity(models.len());
    for model in models {
        if model.model.trim().is_empty() || !identities.insert(model.model) {
            return Err(DbError::Conflict(format!(
                "provider graph model '{}' is blank or duplicated", model.model
            )));
        }
    }
    Ok(())
}

async fn save_graph_models_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    provider_id: &str,
    models: &[NewProviderModel<'_>],
    display_names: &[Option<String>],
    now: i64,
) -> Result<(Vec<ProviderModelRow>, bool), DbError> {
    validate_graph_models(models, display_names)?;
    let mut stored_models = Vec::with_capacity(models.len());
    let mut configuration_changed = false;
    for (model, display_name) in models.iter().zip(display_names) {
        let (_, changed) = save_model_tx(transaction, provider_id, model, now).await?;
        configuration_changed |= changed;
        sqlx::query(
            "UPDATE provider_models SET display_name = ? \
             WHERE provider_id = ? AND model = ?",
        )
        .bind(display_name.as_deref())
        .bind(provider_id)
        .bind(model.model)
        .execute(&mut **transaction)
        .await?;
        stored_models.push(fetch_row(transaction, provider_id, model.model).await?);
    }
    Ok((stored_models, configuration_changed))
}

async fn upsert_named_connections_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    provider_id: &str,
    connections: &[UpsertProviderConnectionParams<'_>],
    now: i64,
) -> Result<bool, DbError> {
    let mut roles = HashSet::with_capacity(connections.len());
    let mut configuration_changed = false;
    for connection in connections {
        let role = connection.role.trim();
        if role.is_empty() || role == "default" || !roles.insert(role) {
            return Err(DbError::Conflict(format!(
                "provider named connection role '{role}' is invalid or duplicated"
            )));
        }
        let base_url = connection.base_url.trim();
        if base_url.is_empty() {
            return Err(DbError::Conflict(format!(
                "provider named connection role '{role}' base_url must not be blank"
            )));
        }
        let auth_scheme = connection.auth_scheme.trim();
        if auth_scheme.is_empty() {
            return Err(DbError::Conflict(format!(
                "provider named connection role '{role}' auth_scheme must not be blank"
            )));
        }
        let existing = sqlx::query_as::<_, ProviderConnectionRow>(
            "SELECT * FROM provider_connections WHERE provider_id = ? AND role = ?",
        )
        .bind(provider_id)
        .bind(role)
        .fetch_optional(&mut **transaction)
        .await?;
        let invocation_changed = existing.as_ref().map_or(true, |existing| {
            existing.base_url != base_url
                || existing.auth_scheme != auth_scheme
                || existing.credentials_encrypted != connection.credentials_encrypted
                || existing.extra != connection.extra
        });
        sqlx::query(
            "INSERT INTO provider_connections \
                (connection_id, provider_id, role, label, base_url, auth_scheme, \
                 credentials_encrypted, extra, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(provider_id, role) DO UPDATE SET \
                label = excluded.label, base_url = excluded.base_url, \
                auth_scheme = excluded.auth_scheme, \
                credentials_encrypted = excluded.credentials_encrypted, \
                extra = excluded.extra, updated_at = excluded.updated_at",
        )
        .bind(nomifun_common::generate_id())
        .bind(provider_id)
        .bind(role)
        .bind(connection.label)
        .bind(base_url)
        .bind(auth_scheme)
        .bind(connection.credentials_encrypted)
        .bind(connection.extra)
        .bind(now)
        .bind(now)
        .execute(&mut **transaction)
        .await?;
        if invocation_changed {
            clear_health_for_connection_role_tx(transaction, provider_id, role).await?;
            configuration_changed = true;
        }
    }
    Ok(configuration_changed)
}

#[async_trait::async_trait]
impl IProviderRepository for SqliteProviderRepository {
    async fn list(&self) -> Result<Vec<Provider>, DbError> {
        Ok(sqlx::query_as::<_, Provider>(
            "SELECT * FROM providers ORDER BY sort_order ASC, created_at ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    async fn find_by_id(&self, id: &str) -> Result<Option<Provider>, DbError> {
        Ok(
            sqlx::query_as::<_, Provider>("SELECT * FROM providers WHERE provider_id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    async fn create(
        &self,
        params: CreateProviderParams<'_>,
        initial_model: &NewProviderModel<'_>,
        connections: &[UpsertProviderConnectionParams<'_>],
    ) -> Result<(Provider, ProviderModelRow), DbError> {
        let (provider, mut models) = self
            .create_graph(params, std::slice::from_ref(initial_model), &[None], connections)
            .await?;
        Ok((provider, models.remove(0)))
    }

    async fn create_graph(
        &self,
        params: CreateProviderParams<'_>,
        models: &[NewProviderModel<'_>],
        display_names: &[Option<String>],
        connections: &[UpsertProviderConnectionParams<'_>],
    ) -> Result<(Provider, Vec<ProviderModelRow>), DbError> {
        validate_graph_models(models, display_names)?;
        let now = nomifun_common::now_ms();
        let mut transaction = self.pool.begin().await?;
        let provider = insert_provider_row_tx(&mut transaction, &params, now).await?;
        upsert_named_connections_tx(&mut transaction, &provider.provider_id, connections, now)
            .await?;
        let (stored_models, _) = save_graph_models_tx(
            &mut transaction, &provider.provider_id, models, display_names, now,
        )
        .await?;
        transaction.commit().await?;
        Ok((provider, stored_models))
    }

    async fn update(
        &self,
        id: &str,
        expected_config_revision: i64,
        params: UpdateProviderParams<'_>,
    ) -> Result<Provider, DbError> {
        self.update_with_connections(id, expected_config_revision, params, &[]).await
    }

    async fn update_with_connections(
        &self,
        id: &str,
        expected_config_revision: i64,
        params: UpdateProviderParams<'_>,
        connections: &[UpsertProviderConnectionParams<'_>],
    ) -> Result<Provider, DbError> {
        let mut transaction = self.pool.begin().await?;
        lock_provider_revision_tx(&mut transaction, id, expected_config_revision).await?;
        let existing = fetch_provider_tx(&mut transaction, id).await?;
        let default_invocation_changed = params
            .base_url
            .is_some_and(|value| value != existing.base_url)
            || params
                .auth_scheme
                .is_some_and(|value| value.trim() != existing.auth_scheme)
            || params
                .credentials_encrypted
                .is_some_and(|value| value != existing.credentials_encrypted)
            || params
                .bedrock_config
                .is_some_and(|value| value != existing.bedrock_config.as_deref());
        let mut graph_changed = default_invocation_changed
            || params
                .enabled
                .is_some_and(|value| value != existing.enabled);
        let auth_scheme = params.auth_scheme.unwrap_or(&existing.auth_scheme).trim();
        if auth_scheme.is_empty() {
            return Err(DbError::Conflict(
                "provider auth_scheme must not be blank".into(),
            ));
        }
        let now = nomifun_common::now_ms();
        sqlx::query(
            "UPDATE providers SET name = ?, base_url = ?, auth_scheme = ?, \
             credentials_encrypted = ?, enabled = ?, bedrock_config = ?, sort_order = ?, \
             updated_at = ? WHERE provider_id = ?",
        )
        .bind(params.name.unwrap_or(&existing.name))
        .bind(params.base_url.unwrap_or(&existing.base_url))
        .bind(auth_scheme)
        .bind(
            params
                .credentials_encrypted
                .unwrap_or(&existing.credentials_encrypted),
        )
        .bind(params.enabled.unwrap_or(existing.enabled))
        .bind(
            params
                .bedrock_config
                .map_or(existing.bedrock_config.as_deref(), |value| value),
        )
        .bind(params.sort_order.unwrap_or(existing.sort_order))
        .bind(now)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        if default_invocation_changed {
            clear_health_for_connection_role_tx(&mut transaction, id, "default").await?;
        }
        graph_changed |= upsert_named_connections_tx(&mut transaction, id, connections, now).await?;
        if graph_changed {
            bump_provider_config_revision_tx(&mut transaction, id).await?;
        }
        let provider = fetch_provider_tx(&mut transaction, id).await?;
        transaction.commit().await?;
        Ok(provider)
    }

    async fn save_graph_models(
        &self,
        provider_id: &str,
        expected_config_revision: i64,
        models: &[NewProviderModel<'_>],
        display_names: &[Option<String>],
    ) -> Result<Vec<ProviderModelRow>, DbError> {
        validate_graph_models(models, display_names)?;
        let mut transaction = self.pool.begin().await?;
        lock_provider_revision_tx(&mut transaction, provider_id, expected_config_revision).await?;
        let (stored_models, configuration_changed) = save_graph_models_tx(
            &mut transaction, provider_id, models, display_names, nomifun_common::now_ms(),
        )
        .await?;
        if configuration_changed {
            bump_provider_config_revision_tx(&mut transaction, provider_id).await?;
        }
        transaction.commit().await?;
        Ok(stored_models)
    }

    async fn clone_graph(
        &self,
        source_provider_id: &str,
        clone_name: &str,
    ) -> Result<Provider, DbError> {
        if clone_name.trim().is_empty() {
            return Err(DbError::Conflict(
                "provider clone name must not be blank".into(),
            ));
        }
        let mut transaction = self.pool.begin().await?;
        let locked =
            sqlx::query("UPDATE providers SET updated_at = updated_at WHERE provider_id = ?")
                .bind(source_provider_id)
                .execute(&mut *transaction)
                .await?;
        if locked.rows_affected() == 0 {
            return Err(DbError::NotFound(format!(
                "Provider '{source_provider_id}' not found"
            )));
        }
        let source = fetch_provider_tx(&mut transaction, source_provider_id).await?;
        let new_id = nomifun_common::ProviderId::new().into_string();
        let now = nomifun_common::now_ms();
        sqlx::query(
            "INSERT INTO providers \
                (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, enabled, \
                 bedrock_config, sort_order, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, \
                 (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM providers), ?, ?)",
        )
        .bind(&new_id)
        .bind(&source.platform)
        .bind(clone_name.trim())
        .bind(&source.base_url)
        .bind(&source.auth_scheme)
        .bind(&source.credentials_encrypted)
        .bind(source.enabled)
        .bind(&source.bedrock_config)
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await?;

        let models = sqlx::query_as::<_, ProviderModelRow>(
            "SELECT * FROM provider_models WHERE provider_id = ? ORDER BY sort_order, id",
        )
        .bind(source_provider_id)
        .fetch_all(&mut *transaction)
        .await?;
        for model in models {
            sqlx::query(
                "INSERT INTO provider_models \
                    (provider_id, model, display_name, enabled, sort_order, description, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&new_id)
            .bind(&model.model)
            .bind(&model.display_name)
            .bind(model.enabled)
            .bind(model.sort_order)
            .bind(&model.description)
            .bind(now)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "INSERT INTO provider_model_capabilities \
                    (provider_id, model, task, traits, protocol, connection_role, \
                     base_url_override, endpoint, poll_endpoint, content_endpoint, realtime_endpoint, \
                     allow_cross_origin_credentials, provider_params, context_limit, output_limit, compaction_threshold_pct, input_constraints, \
                     created_at, updated_at) \
                 SELECT ?, model, task, traits, protocol, connection_role, \
                     base_url_override, endpoint, poll_endpoint, content_endpoint, realtime_endpoint, \
                     allow_cross_origin_credentials, provider_params, context_limit, output_limit, compaction_threshold_pct, input_constraints, ?, ? \
                 FROM provider_model_capabilities WHERE provider_id = ? AND model = ?",
            )
            .bind(&new_id)
            .bind(now)
            .bind(now)
            .bind(source_provider_id)
            .bind(&model.model)
            .execute(&mut *transaction)
            .await?;
        }

        let connections = sqlx::query_as::<_, ProviderConnectionRow>(
            "SELECT * FROM provider_connections WHERE provider_id = ? ORDER BY role, id",
        )
        .bind(source_provider_id)
        .fetch_all(&mut *transaction)
        .await?;
        for connection in connections {
            sqlx::query(
                "INSERT INTO provider_connections \
                    (connection_id, provider_id, role, label, base_url, auth_scheme, \
                     credentials_encrypted, extra, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(nomifun_common::generate_id())
            .bind(&new_id)
            .bind(&connection.role)
            .bind(&connection.label)
            .bind(&connection.base_url)
            .bind(&connection.auth_scheme)
            .bind(&connection.credentials_encrypted)
            .bind(&connection.extra)
            .bind(now)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        }
        let provider = fetch_provider_tx(&mut transaction, &new_id).await?;
        transaction.commit().await?;
        Ok(provider)
    }

    async fn delete(&self, id: &str) -> Result<(), DbError> {
        self.delete_graph(id,None,&crate::ProviderModelCleanupPlan::default()).await
    }

    async fn delete_coordinated(&self, plan: &crate::CoordinatedProviderDelete) -> Result<(), DbError> {
        self.delete_graph(&plan.provider_id,Some(plan.expected_config_revision),&plan.cleanup).await
    }
}
