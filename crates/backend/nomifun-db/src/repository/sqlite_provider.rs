use std::collections::HashSet;

use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::{
    NewProviderModel, Provider, ProviderConnectionRow, ProviderModelRow,
    UpsertProviderConnectionParams,
};
use crate::repository::provider::{CreateProviderParams, UpdateProviderParams};
use crate::repository::sqlite_provider_model::save_model_tx;
use crate::repository::sqlite_provider_model_capability::{
    bump_provider_config_revision_tx, clear_health_for_connection_role_tx,
};
use crate::repository::{
    IProviderRepository, ProviderPreferenceDeleteAction, provider_preference_delete_action,
};

const PROVIDER_HARD_BINDING_DELETE_CONFLICT: &str =
    "provider is still referenced by an executable Agent binding";

async fn prune_missing_provider_preference(
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
        let now = nomifun_common::now_ms();
        let mut transaction = self.pool.begin().await?;
        let provider = insert_provider_row_tx(&mut transaction, &params, now).await?;
        let mut roles = HashSet::with_capacity(connections.len());
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
            sqlx::query(
                "INSERT INTO provider_connections \
                    (connection_id, provider_id, role, label, base_url, auth_scheme, \
                     credentials_encrypted, extra, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(nomifun_common::generate_id())
            .bind(&provider.provider_id)
            .bind(role)
            .bind(connection.label)
            .bind(base_url)
            .bind(auth_scheme)
            .bind(connection.credentials_encrypted)
            .bind(connection.extra)
            .bind(now)
            .bind(now)
            .execute(&mut *transaction)
            .await?;
        }
        let (model, _) =
            save_model_tx(&mut transaction, &provider.provider_id, initial_model, now).await?;
        transaction.commit().await?;
        Ok((provider, model))
    }

    async fn update(
        &self,
        id: &str,
        expected_config_revision: i64,
        params: UpdateProviderParams<'_>,
    ) -> Result<Provider, DbError> {
        let mut transaction = self.pool.begin().await?;
        let locked = sqlx::query(
            "UPDATE providers SET config_revision = config_revision \
             WHERE provider_id = ? AND config_revision = ?",
        )
        .bind(id)
        .bind(expected_config_revision)
        .execute(&mut *transaction)
        .await?;
        if locked.rows_affected() == 0 {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM providers WHERE provider_id = ?)")
                    .bind(id)
                    .fetch_one(&mut *transaction)
                    .await?;
            return if exists {
                Err(DbError::Conflict(format!(
                    "provider invocation graph changed while updating provider; expected revision {expected_config_revision}"
                )))
            } else {
                Err(DbError::NotFound(format!("Provider '{id}' not found")))
            };
        }
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
        let graph_changed = default_invocation_changed
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
        if graph_changed {
            bump_provider_config_revision_tx(&mut transaction, id).await?;
        }
        let provider = fetch_provider_tx(&mut transaction, id).await?;
        transaction.commit().await?;
        Ok(provider)
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
                     allow_cross_origin_credentials, provider_params, context_limit, output_limit, \
                     created_at, updated_at) \
                 SELECT ?, model, task, traits, protocol, connection_role, \
                     base_url_override, endpoint, poll_endpoint, content_endpoint, realtime_endpoint, \
                     allow_cross_origin_credentials, provider_params, context_limit, output_limit, ?, ? \
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

    async fn save_managed_graph(
        &self,
        params: CreateProviderParams<'_>,
        models: &[NewProviderModel<'_>],
    ) -> Result<Provider, DbError> {
        validate_provider_params(&params)?;
        let Some(provider_id) = params.provider_id else {
            return Err(DbError::Conflict(
                "managed provider graph requires an explicit provider_id".into(),
            ));
        };
        let provider_id = nomifun_common::ProviderId::parse(provider_id)
            .map_err(|error| {
                DbError::Conflict(format!("invalid provider_id '{provider_id}': {error}"))
            })?
            .into_string();
        if models.is_empty() {
            return Err(DbError::Conflict(
                "managed provider graph must contain at least one model".into(),
            ));
        }
        let mut names = HashSet::with_capacity(models.len());
        for model in models {
            if !names.insert(model.model) {
                return Err(DbError::Conflict(format!(
                    "managed provider model '{}' is duplicated",
                    model.model
                )));
            }
        }

        let mut transaction = self.pool.begin().await?;
        let now = nomifun_common::now_ms();
        // Managed reconciliation validates and writes inside this transaction,
        // so serialize it on the same provider graph lock as user mutations.
        let parent_lock = sqlx::query(
            "UPDATE providers SET config_revision = config_revision WHERE provider_id = ?",
        )
        .bind(&provider_id)
        .execute(&mut *transaction)
        .await?;
        let existing = if parent_lock.rows_affected() > 0 {
            sqlx::query_as::<_, Provider>("SELECT * FROM providers WHERE provider_id = ?")
                .bind(&provider_id)
                .fetch_optional(&mut *transaction)
                .await?
        } else {
            None
        };
        let provider_existed = existing.is_some();
        let mut graph_changed = false;
        let mut default_invocation_changed = false;
        if let Some(existing) = existing {
            default_invocation_changed = params.platform != existing.platform
                || params.base_url != existing.base_url
                || params.auth_scheme.trim() != existing.auth_scheme
                || params.credentials_encrypted != existing.credentials_encrypted
                || params.bedrock_config != existing.bedrock_config.as_deref();
            graph_changed = default_invocation_changed || params.enabled != existing.enabled;
            sqlx::query(
                "UPDATE providers SET platform = ?, name = ?, base_url = ?, auth_scheme = ?, \
                 credentials_encrypted = ?, enabled = ?, bedrock_config = ?, sort_order = ?, \
                 updated_at = ? WHERE provider_id = ?",
            )
            .bind(params.platform)
            .bind(params.name)
            .bind(params.base_url)
            .bind(params.auth_scheme.trim())
            .bind(params.credentials_encrypted)
            .bind(params.enabled)
            .bind(params.bedrock_config)
            .bind(params.sort_order.unwrap_or(existing.sort_order))
            .bind(now)
            .bind(&provider_id)
            .execute(&mut *transaction)
            .await?;
        } else {
            let normalized = CreateProviderParams {
                provider_id: Some(&provider_id),
                ..params
            };
            insert_provider_row_tx(&mut transaction, &normalized, now).await?;
        }

        for model in models {
            let (_, changed) = save_model_tx(&mut transaction, &provider_id, model, now).await?;
            graph_changed |= changed;
        }
        if default_invocation_changed {
            clear_health_for_connection_role_tx(&mut transaction, &provider_id, "default").await?;
        }
        let placeholders = vec!["?"; models.len()].join(", ");
        let delete_caps_sql = format!(
            "DELETE FROM provider_model_capabilities WHERE provider_id = ? \
             AND model NOT IN ({placeholders})"
        );
        let mut delete_caps = sqlx::query(&delete_caps_sql).bind(&provider_id);
        for model in models {
            delete_caps = delete_caps.bind(model.model);
        }
        let deleted_capabilities = delete_caps
            .execute(&mut *transaction)
            .await?
            .rows_affected();
        graph_changed |= deleted_capabilities > 0;

        let delete_models_sql = format!(
            "DELETE FROM provider_models WHERE provider_id = ? AND model NOT IN ({placeholders})"
        );
        let mut delete_models = sqlx::query(&delete_models_sql).bind(&provider_id);
        for model in models {
            delete_models = delete_models.bind(model.model);
        }
        let deleted_models = delete_models
            .execute(&mut *transaction)
            .await?
            .rows_affected();
        graph_changed |= deleted_models > 0;

        if provider_existed && graph_changed {
            bump_provider_config_revision_tx(&mut transaction, &provider_id).await?;
        }

        let provider = fetch_provider_tx(&mut transaction, &provider_id).await?;
        transaction.commit().await?;
        Ok(provider)
    }

    async fn delete(&self, id: &str) -> Result<(), DbError> {
        let mut transaction = self.pool.begin().await?;

        // Acquire SQLite's writer lock before inspecting logical references.
        // This keeps the guard, provider deletion, and soft-reference cleanup
        // in one application-owned transaction without a physical FK/trigger.
        let locked =
            sqlx::query("UPDATE providers SET updated_at = updated_at WHERE provider_id = ?")
                .bind(id)
                .execute(&mut *transaction)
                .await?;

        if locked.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("Provider '{id}' not found")));
        }

        let hard_binding_exists: bool = sqlx::query_scalar(
            "SELECT \
                EXISTS(\
                    SELECT 1 FROM conversations \
                    WHERE json_extract(model, '$.provider_id') = ?1\
                ) \
                OR EXISTS(\
                    SELECT 1 FROM agent_execution_template_participants \
                    WHERE provider_id = ?1\
                ) \
                OR EXISTS(\
                    SELECT 1 \
                    FROM agent_execution_participants participant \
                    JOIN agent_executions execution \
                      ON execution.execution_id = participant.execution_id \
                    WHERE participant.provider_id = ?1 \
                      AND participant.retired_in_revision IS NULL \
                      AND execution.status <> 'cancelled' \
                      AND execution.deleted_at IS NULL\
                ) \
                OR EXISTS(\
                    SELECT 1 FROM creation_tasks WHERE provider_id = ?1\
                ) \
                OR EXISTS(\
                    SELECT 1 FROM cron_jobs \
                    WHERE agent_type = 'nomi' \
                      AND agent_config IS NOT NULL \
                      AND CASE \
                            WHEN NOT json_valid(agent_config) THEN 1 \
                            ELSE json_extract(agent_config, '$.provider_id') = ?1 \
                          END\
                )",
        )
        .bind(id)
        .fetch_one(&mut *transaction)
        .await?;
        if hard_binding_exists {
            return Err(DbError::Conflict(
                PROVIDER_HARD_BINDING_DELETE_CONFLICT.to_owned(),
            ));
        }

        // Client preferences are a generic key/value store, so their Provider
        // references are enforced by the centralized registry in the
        // client-preference repository rather than by SQL FK/trigger logic.
        // Resolve every registered preference before deleting the parent:
        // arrays are filtered in order; defaults are deleted and optional
        // references are set to null. No registered preference RESTRICTs the
        // delete any more.
        let preference_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT key, value FROM client_preferences \
             WHERE key = 'agent.model_failover' \
                OR key = 'nomi.collaborationModels' \
                OR key = 'nomi.defaultModel' \
                OR key = 'knowledge.autogenModel' \
                OR key = 'knowledge.retrieval' \
                OR key = 'models.default.imageGeneration' \
                OR key = 'tools.speechToText' \
                OR key = 'tools.textToSpeech' \
                OR key LIKE 'channels.%.defaultModel'",
        )
        .fetch_all(&mut *transaction)
        .await?;
        let mut preference_actions = Vec::new();
        for (key, value) in preference_rows {
            match provider_preference_delete_action(&key, &value, id)? {
                ProviderPreferenceDeleteAction::Keep => {}
                action => preference_actions.push((key, action)),
            }
        }

        sqlx::query("DELETE FROM providers WHERE provider_id = ?")
            .bind(id)
            .execute(&mut *transaction)
            .await?;

        // Soft logical references are repaired explicitly in the same
        // transaction. SQLite owns no cascade or relation semantics.
        sqlx::query(
            "UPDATE conversations \
             SET execution_model_pool = CASE \
                    WHEN json_extract(execution_model_pool, '$.mode') = 'single' \
                        THEN NULL \
                    ELSE (\
                        SELECT CASE \
                            WHEN COUNT(*) = 0 THEN NULL \
                            ELSE json_object(\
                                'mode', 'range', \
                                'models', json(json_group_array(json(item.value)))\
                            ) \
                        END \
                        FROM json_each(conversations.execution_model_pool, '$.models') item \
                        WHERE json_extract(item.value, '$.provider_id') <> ?1 \
                          AND EXISTS (\
                              SELECT 1 FROM providers provider \
                              WHERE provider.provider_id = json_extract(item.value, '$.provider_id')\
                          )\
                    ) \
                 END, \
                 updated_at = MAX(updated_at, ?2) \
             WHERE execution_model_pool IS NOT NULL \
               AND (\
                    (json_extract(execution_model_pool, '$.mode') = 'single' \
                     AND json_extract(execution_model_pool, '$.model.provider_id') = ?1) \
                    OR \
                    (json_extract(execution_model_pool, '$.mode') = 'range' \
                     AND EXISTS (\
                         SELECT 1 FROM json_each(execution_model_pool, '$.models') target \
                         WHERE json_extract(target.value, '$.provider_id') = ?1\
                     ))\
               )",
        )
        .bind(id)
        .bind(nomifun_common::now_ms())
        .execute(&mut *transaction)
        .await?;

        let now = nomifun_common::now_ms();
        sqlx::query(
            "UPDATE conversations \
             SET extra = json_remove(\
                    extra, \
                    CASE \
                        WHEN json_extract(extra, '$.idmm.fault_watch.bypass_model.provider_id') = ?1 \
                        THEN '$.idmm.fault_watch.bypass_model' \
                        ELSE '$.__nomifun_noop_idmm_fault_bypass' \
                    END, \
                    CASE \
                        WHEN json_extract(extra, '$.idmm.decision_watch.bypass_model.provider_id') = ?1 \
                        THEN '$.idmm.decision_watch.bypass_model' \
                        ELSE '$.__nomifun_noop_idmm_decision_bypass' \
                    END\
                 ), \
                 updated_at = MAX(updated_at, ?2) \
             WHERE json_valid(extra) \
               AND (\
                    json_extract(extra, '$.idmm.fault_watch.bypass_model.provider_id') = ?1 \
                    OR json_extract(extra, '$.idmm.decision_watch.bypass_model.provider_id') = ?1\
               )",
        )
        .bind(id)
        .bind(now)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE terminal_sessions \
             SET idmm = json_remove(\
                    idmm, \
                    CASE \
                        WHEN json_extract(idmm, '$.fault_watch.bypass_model.provider_id') = ?1 \
                        THEN '$.fault_watch.bypass_model' \
                        ELSE '$.__nomifun_noop_idmm_fault_bypass' \
                    END, \
                    CASE \
                        WHEN json_extract(idmm, '$.decision_watch.bypass_model.provider_id') = ?1 \
                        THEN '$.decision_watch.bypass_model' \
                        ELSE '$.__nomifun_noop_idmm_decision_bypass' \
                    END\
                 ), \
                 updated_at = MAX(updated_at, ?2) \
             WHERE idmm IS NOT NULL \
               AND json_valid(idmm) \
               AND (\
                    json_extract(idmm, '$.fault_watch.bypass_model.provider_id') = ?1 \
                    OR json_extract(idmm, '$.decision_watch.bypass_model.provider_id') = ?1\
               )",
        )
        .bind(id)
        .bind(now)
        .execute(&mut *transaction)
        .await?;

        for (key, action) in preference_actions {
            match action {
                ProviderPreferenceDeleteAction::Delete => {
                    sqlx::query("DELETE FROM client_preferences WHERE key = ?")
                        .bind(&key)
                        .execute(&mut *transaction)
                        .await?;
                }
                ProviderPreferenceDeleteAction::Update(value) => {
                    let value =
                        prune_missing_provider_preference(&key, value, &mut transaction).await?;
                    sqlx::query(
                        "UPDATE client_preferences \
                         SET value = ?, updated_at = MAX(updated_at, ?) \
                         WHERE key = ?",
                    )
                    .bind(value)
                    .bind(now)
                    .bind(&key)
                    .execute(&mut *transaction)
                    .await?;
                }
                ProviderPreferenceDeleteAction::Keep => unreachable!(),
            }
        }

        // Cascade the provider delete to the catalog tables in the same
        // transaction.
        sqlx::query("DELETE FROM provider_model_capabilities WHERE provider_id = ?")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM provider_models WHERE provider_id = ?")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM provider_connections WHERE provider_id = ?")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE preset_model_preferences SET provider_id = NULL WHERE provider_id = ?")
            .bind(id)
            .execute(&mut *transaction)
            .await?;

        transaction.commit().await?;
        Ok(())
    }
}
