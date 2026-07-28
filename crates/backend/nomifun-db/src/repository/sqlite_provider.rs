use std::collections::{HashMap, HashSet};

use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::Provider;
use crate::repository::{
    provider_preference_delete_action, IProviderRepository, ProviderPreferenceDeleteAction,
};
use crate::repository::provider::{CreateProviderParams, UpdateProviderParams};

const PROVIDER_HARD_BINDING_DELETE_CONFLICT: &str =
    "provider is still referenced by an executable Agent binding";

async fn prune_missing_provider_preference(
    key: &str,
    value: String,
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<String, DbError> {
    let mut parsed: serde_json::Value = serde_json::from_str(&value)
        .map_err(|error| DbError::Conflict(format!("invalid client preference '{key}': {error}")))?;
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
        let Some(provider_id) = item
            .get("provider_id")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM providers WHERE provider_id = ?)",
        )
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

/// SQLite-backed implementation of [`IProviderRepository`].
#[derive(Clone, Debug)]
pub struct SqliteProviderRepository {
    pool: SqlitePool,
}

impl SqliteProviderRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

/// The five legacy per-model JSON map columns that dual-write mirrors into
/// typed `provider_models` columns. Profile columns (`tasks`, `traits`,
/// `params`, `source`) and `connection_role` are intentionally absent: they
/// are owned by the new-table writers and are NEVER touched by dual-write.
#[derive(Clone, Copy, Debug)]
enum ModelMapColumn {
    Enabled,
    Protocol,
    ContextLimit,
    Description,
    Health,
}

impl ModelMapColumn {
    fn name(self) -> &'static str {
        match self {
            Self::Enabled => "model_enabled",
            Self::Protocol => "model_protocols",
            Self::ContextLimit => "model_context_limits",
            Self::Description => "model_descriptions",
            Self::Health => "model_health",
        }
    }

    /// Resets the mirrored column to its default on every row of a provider
    /// (enabled → 1, all nullable columns → NULL).
    fn reset_sql(self) -> &'static str {
        match self {
            Self::Enabled => {
                "UPDATE provider_models SET enabled = 1, updated_at = ? WHERE provider_id = ?"
            }
            Self::Protocol => {
                "UPDATE provider_models SET protocol = NULL, updated_at = ? WHERE provider_id = ?"
            }
            Self::ContextLimit => {
                "UPDATE provider_models SET context_limit = NULL, updated_at = ? \
                 WHERE provider_id = ?"
            }
            Self::Description => {
                "UPDATE provider_models SET description = NULL, updated_at = ? \
                 WHERE provider_id = ?"
            }
            Self::Health => {
                "UPDATE provider_models SET health = NULL, updated_at = ? WHERE provider_id = ?"
            }
        }
    }

    /// Sets the mirrored column for one `(provider_id, model)` row.
    fn set_sql(self) -> &'static str {
        match self {
            Self::Enabled => {
                "UPDATE provider_models SET enabled = ?, updated_at = ? \
                 WHERE provider_id = ? AND model = ?"
            }
            Self::Protocol => {
                "UPDATE provider_models SET protocol = ?, updated_at = ? \
                 WHERE provider_id = ? AND model = ?"
            }
            Self::ContextLimit => {
                "UPDATE provider_models SET context_limit = ?, updated_at = ? \
                 WHERE provider_id = ? AND model = ?"
            }
            Self::Description => {
                "UPDATE provider_models SET description = ?, updated_at = ? \
                 WHERE provider_id = ? AND model = ?"
            }
            Self::Health => {
                "UPDATE provider_models SET health = ?, updated_at = ? \
                 WHERE provider_id = ? AND model = ?"
            }
        }
    }

    /// Converts one JSON map entry into the typed bind for this column,
    /// mirroring migration 014's backfill semantics: enabled coerces to a
    /// boolean (default true), protocol/description store string atoms,
    /// context_limit stores an integer, health stores minified JSON text.
    fn to_bind(self, value: &serde_json::Value) -> crate::repository::bind::BindValue {
        use crate::repository::bind::BindValue;
        match self {
            Self::Enabled => BindValue::Bool(match value {
                serde_json::Value::Bool(flag) => *flag,
                serde_json::Value::Number(number) => number.as_i64() != Some(0),
                // Any non-boolean atom falls back to the column default.
                _ => true,
            }),
            Self::Protocol | Self::Description => {
                BindValue::OptStr(value.as_str().map(String::from))
            }
            Self::ContextLimit => BindValue::OptI64(
                value
                    .as_i64()
                    .or_else(|| value.as_f64().map(|v| v as i64)),
            ),
            Self::Health => BindValue::OptStr(match value {
                serde_json::Value::Null => None,
                other => Some(other.to_string()),
            }),
        }
    }
}

/// Dual-write rule 2b (map replacement): applying a legacy per-model map to
/// `provider_models` uses whole-map replacement semantics for that column
/// across ALL of the provider's rows — a model missing from the map has the
/// column reset to its default (enabled → 1, others → NULL), exactly matching
/// the legacy wire semantics where the map column is replaced as a whole. An
/// empty map (explicit `Some(None)` clear) resets every row to the default.
/// Map entries for models without a catalog row are ignored, matching
/// migration 014's backfill, which only materializes `models` entries.
async fn apply_provider_model_map_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    provider_id: &str,
    column: ModelMapColumn,
    map: &HashMap<String, serde_json::Value>,
    now: i64,
) -> Result<(), DbError> {
    sqlx::query(column.reset_sql())
        .bind(now)
        .bind(provider_id)
        .execute(&mut **transaction)
        .await?;

    for (model, value) in map {
        let bind = column.to_bind(value);
        crate::repository::bind::bind_value(sqlx::query(column.set_sql()), &bind)
            .bind(now)
            .bind(provider_id)
            .bind(model)
            .execute(&mut **transaction)
            .await?;
    }

    Ok(())
}

/// Dual-write rules 1 and 2a (membership): mirror the legacy `models` JSON
/// array into `provider_models` rows inside the caller's transaction.
///
/// - Models present in the array get a row; a new row takes its mirrored
///   columns (enabled/protocol/context_limit/description/health) from the
///   effective per-model maps, plus tasks/traits '[]', params '{}',
///   source 'inferred', and `sort_order` = array index.
/// - Existing rows keep every column except `sort_order`/`updated_at` — the
///   profile columns (tasks/traits/params/source) and `connection_role` are
///   NEVER touched by dual-write.
/// - Rows whose model is no longer in the array are deleted.
async fn sync_provider_model_membership_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    provider_id: &str,
    models_json: &str,
    maps: &[(ModelMapColumn, HashMap<String, serde_json::Value>, bool)],
    now: i64,
) -> Result<(), DbError> {
    use crate::repository::bind::{bind_value, BindValue};

    let models: Vec<String> = serde_json::from_str(models_json).map_err(|error| {
        DbError::Conflict(format!("invalid provider models array: {error}"))
    })?;

    if models.is_empty() {
        sqlx::query("DELETE FROM provider_models WHERE provider_id = ?")
            .bind(provider_id)
            .execute(&mut **transaction)
            .await?;
        return Ok(());
    }

    let placeholders = vec!["?"; models.len()].join(", ");
    let delete_sql = format!(
        "DELETE FROM provider_models WHERE provider_id = ? AND model NOT IN ({placeholders})"
    );
    let mut delete = sqlx::query(&delete_sql).bind(provider_id);
    for model in &models {
        delete = delete.bind(model);
    }
    delete.execute(&mut **transaction).await?;

    let mut seen: HashSet<&str> = HashSet::with_capacity(models.len());
    for (index, model) in models.iter().enumerate() {
        // A duplicate entry in the legacy array keeps its first index.
        if !seen.insert(model.as_str()) {
            continue;
        }

        // Mirrored-column values for a freshly inserted row; existing rows
        // ignore these (the upsert only touches sort_order/updated_at).
        let mut enabled = BindValue::Bool(true);
        let mut protocol = BindValue::OptStr(None);
        let mut context_limit = BindValue::OptI64(None);
        let mut description = BindValue::OptStr(None);
        let mut health = BindValue::OptStr(None);
        for (column, map, _) in maps {
            if let Some(value) = map.get(model.as_str()) {
                let bind = column.to_bind(value);
                match column {
                    ModelMapColumn::Enabled => enabled = bind,
                    ModelMapColumn::Protocol => protocol = bind,
                    ModelMapColumn::ContextLimit => context_limit = bind,
                    ModelMapColumn::Description => description = bind,
                    ModelMapColumn::Health => health = bind,
                }
            }
        }

        let mut query = sqlx::query(
            "INSERT INTO provider_models \
                (provider_id, model, enabled, sort_order, tasks, traits, protocol, \
                 params, context_limit, description, source, health, created_at, updated_at) \
             VALUES (?, ?, ?, ?, '[]', '[]', ?, '{}', ?, ?, 'inferred', ?, ?, ?) \
             ON CONFLICT(provider_id, model) DO UPDATE SET \
                sort_order = excluded.sort_order, \
                updated_at = excluded.updated_at",
        )
        .bind(provider_id)
        .bind(model);
        query = bind_value(query, &enabled);
        query = query.bind(index as i64);
        query = bind_value(query, &protocol);
        query = bind_value(query, &context_limit);
        query = bind_value(query, &description);
        query = bind_value(query, &health);
        query
            .bind(now)
            .bind(now)
            .execute(&mut **transaction)
            .await?;
    }

    Ok(())
}

/// Dual-write orchestrator shared by `create` and `update`: keeps
/// `provider_models` rows in sync with the legacy `models` array and the five
/// per-model JSON map columns, inside the caller's providers transaction.
/// Direction is legacy → new only.
///
/// `maps` carries, per mirrored column, the *effective* map JSON after this
/// write (merged column value; `None` = empty map) and whether the caller
/// supplied that map param in this call:
/// - the effective map always feeds mirrored columns of freshly inserted
///   membership rows, so a re-added model picks up retained map entries;
/// - `replace = true` additionally applies whole-map replacement for that
///   column across ALL rows (see [`apply_provider_model_map_tx`]); with
///   `replace = false` existing rows keep their current column value.
///
/// Behavior spec:
/// 1. create: one row per `models` entry; enabled/protocol/context_limit/
///    description/health from the corresponding map params; tasks/traits
///    '[]', params '{}', source 'inferred', sort_order = array index.
/// 2. update: `models` `Some` syncs membership (insert new, delete removed,
///    re-index survivors); a map param `Some(...)` is a whole-map replacement
///    for that column over ALL rows (`Some(None)` = empty map → all defaults);
///    a map param `None` leaves the column of existing rows untouched.
///    Profile columns (tasks/traits/params/source) and connection_role are
///    never written by dual-write.
/// 3. delete cascades are handled directly in [`IProviderRepository::delete`].
async fn sync_provider_models_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    provider_id: &str,
    models_json: Option<&str>,
    maps: [(ModelMapColumn, Option<&str>, bool); 5],
    now: i64,
) -> Result<(), DbError> {
    // A map is only consulted when seeding freshly inserted membership rows
    // or when running its whole-map replacement pass; skip parsing (and its
    // strict-JSON failure mode) for maps this call never touches. A write
    // that changes neither membership nor any map leaves provider_models
    // fully untouched.
    let mut parsed: Vec<(ModelMapColumn, HashMap<String, serde_json::Value>, bool)> =
        Vec::with_capacity(maps.len());
    for (column, map_json, replace) in maps {
        let used = models_json.is_some() || replace;
        let map: HashMap<String, serde_json::Value> = match map_json.filter(|_| used) {
            Some(json) => serde_json::from_str(json).map_err(|error| {
                DbError::Conflict(format!("invalid provider {} map: {error}", column.name()))
            })?,
            None => HashMap::new(),
        };
        parsed.push((column, map, replace));
    }

    if let Some(models_json) = models_json {
        sync_provider_model_membership_tx(transaction, provider_id, models_json, &parsed, now)
            .await?;
    }

    for (column, map, replace) in &parsed {
        if *replace {
            apply_provider_model_map_tx(transaction, provider_id, *column, map, now).await?;
        }
    }

    Ok(())
}

#[async_trait::async_trait]
impl IProviderRepository for SqliteProviderRepository {
    async fn list(&self) -> Result<Vec<Provider>, DbError> {
        let rows = sqlx::query_as::<_, Provider>(
            "SELECT * FROM providers ORDER BY sort_order ASC, created_at ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    async fn find_by_id(&self, id: &str) -> Result<Option<Provider>, DbError> {
        let row = sqlx::query_as::<_, Provider>("SELECT * FROM providers WHERE provider_id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row)
    }

    async fn create(&self, params: CreateProviderParams<'_>) -> Result<Provider, DbError> {
        let provider_id = match params.provider_id {
            Some(provider_id) => nomifun_common::ProviderId::parse(provider_id)
                .map(nomifun_common::ProviderId::into_string)
                .map_err(|error| {
                    DbError::Conflict(format!(
                        "invalid provider_id '{provider_id}': {error}"
                    ))
                })?,
            None => nomifun_common::ProviderId::new().into_string(),
        };
        let now = nomifun_common::now_ms();
        let mut transaction = self.pool.begin().await?;
        let sort_order = match params.sort_order {
            Some(value) => value,
            None => {
                sqlx::query_scalar::<_, i64>(
                    "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM providers",
                )
                .fetch_one(&mut *transaction)
                .await?
            }
        };

        sqlx::query(
            "INSERT INTO providers \
                (provider_id, platform, name, base_url, api_key_encrypted, models, enabled, \
                 capabilities, model_context_limits, model_protocols, model_descriptions, \
                 model_enabled, model_health, bedrock_config, is_full_url, sort_order, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&provider_id)
        .bind(params.platform)
        .bind(params.name)
        .bind(params.base_url)
        .bind(params.api_key_encrypted)
        .bind(params.models)
        .bind(params.enabled)
        .bind(params.capabilities)
        .bind(params.model_context_limits.unwrap_or("{}"))
        .bind(params.model_protocols)
        // model_descriptions is NOT NULL DEFAULT '{}'; coalesce None → '{}'.
        .bind(params.model_descriptions.unwrap_or("{}"))
        .bind(params.model_enabled)
        .bind(params.model_health)
        .bind(params.bedrock_config)
        .bind(params.is_full_url)
        .bind(sort_order)
        .bind(now)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(db_err) if is_unique_violation(db_err.as_ref()) => {
                DbError::Conflict(format!("Provider with id '{provider_id}' already exists"))
            }
            _ => DbError::Query(e),
        })?;

        // Dual-write: mirror the models array + per-model maps into
        // provider_models rows in the same transaction. Every row is a fresh
        // insert seeded from the map params, so no whole-map replacement
        // pass is needed (replace = false).
        sync_provider_models_tx(
            &mut transaction,
            &provider_id,
            Some(params.models),
            [
                (ModelMapColumn::Enabled, params.model_enabled, false),
                (ModelMapColumn::Protocol, params.model_protocols, false),
                (
                    ModelMapColumn::ContextLimit,
                    params.model_context_limits,
                    false,
                ),
                (
                    ModelMapColumn::Description,
                    params.model_descriptions,
                    false,
                ),
                (ModelMapColumn::Health, params.model_health, false),
            ],
            now,
        )
        .await?;

        let id = sqlx::query_scalar("SELECT id FROM providers WHERE provider_id = ?")
            .bind(&provider_id)
            .fetch_one(&mut *transaction)
            .await?;
        transaction.commit().await?;

        Ok(Provider {
            id,
            provider_id,
            platform: params.platform.to_string(),
            name: params.name.to_string(),
            base_url: params.base_url.to_string(),
            api_key_encrypted: params.api_key_encrypted.to_string(),
            models: params.models.to_string(),
            enabled: params.enabled,
            capabilities: params.capabilities.to_string(),
            model_context_limits: params.model_context_limits.map(String::from),
            model_protocols: params.model_protocols.map(String::from),
            model_descriptions: params.model_descriptions.map(String::from),
            model_enabled: params.model_enabled.map(String::from),
            model_health: params.model_health.map(String::from),
            bedrock_config: params.bedrock_config.map(String::from),
            is_full_url: params.is_full_url,
            sort_order,
            created_at: now,
            updated_at: now,
        })
    }

    async fn update(&self, id: &str, params: UpdateProviderParams<'_>) -> Result<Provider, DbError> {
        let existing = self
            .find_by_id(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("Provider '{id}' not found")))?;

        // Capture dual-write inputs before params is consumed by the merge.
        // `models: None` keeps membership; a map param of `None` keeps that
        // column on existing rows; `Some(None)` clears the map (all rows
        // reset to the column default); `Some(Some(json))` replaces it.
        let models_json = params.models;
        let replace_enabled = params.model_enabled.is_some();
        let replace_protocols = params.model_protocols.is_some();
        let replace_limits = params.model_context_limits.is_some();
        let replace_descriptions = params.model_descriptions.is_some();
        let replace_health = params.model_health.is_some();

        let merged = merge_update(existing, params);

        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "UPDATE providers SET \
                platform = ?, name = ?, base_url = ?, api_key_encrypted = ?, \
                models = ?, enabled = ?, capabilities = ?, \
                model_context_limits = ?, model_protocols = ?, model_descriptions = ?, model_enabled = ?, \
                model_health = ?, bedrock_config = ?, is_full_url = ?, sort_order = ?, updated_at = ? \
             WHERE provider_id = ?",
        )
        .bind(&merged.platform)
        .bind(&merged.name)
        .bind(&merged.base_url)
        .bind(&merged.api_key_encrypted)
        .bind(&merged.models)
        .bind(merged.enabled)
        .bind(&merged.capabilities)
        .bind(merged.model_context_limits.as_deref().unwrap_or("{}"))
        .bind(&merged.model_protocols)
        // model_descriptions is NOT NULL DEFAULT '{}'; coalesce None → '{}'.
        .bind(merged.model_descriptions.as_deref().unwrap_or("{}"))
        .bind(&merged.model_enabled)
        .bind(&merged.model_health)
        .bind(&merged.bedrock_config)
        .bind(merged.is_full_url)
        .bind(merged.sort_order)
        .bind(merged.updated_at)
        .bind(id)
        .execute(&mut *transaction)
        .await?;

        // Dual-write: sync provider_models in the same transaction. The
        // effective (merged) map values seed mirrored columns for any newly
        // inserted membership row; whole-map replacement runs only for map
        // params the caller actually supplied.
        sync_provider_models_tx(
            &mut transaction,
            id,
            models_json,
            [
                (
                    ModelMapColumn::Enabled,
                    merged.model_enabled.as_deref(),
                    replace_enabled,
                ),
                (
                    ModelMapColumn::Protocol,
                    merged.model_protocols.as_deref(),
                    replace_protocols,
                ),
                (
                    ModelMapColumn::ContextLimit,
                    merged.model_context_limits.as_deref(),
                    replace_limits,
                ),
                (
                    ModelMapColumn::Description,
                    merged.model_descriptions.as_deref(),
                    replace_descriptions,
                ),
                (
                    ModelMapColumn::Health,
                    merged.model_health.as_deref(),
                    replace_health,
                ),
            ],
            merged.updated_at,
        )
        .await?;
        transaction.commit().await?;

        Ok(merged)
    }

    async fn delete(&self, id: &str) -> Result<(), DbError> {
        let mut transaction = self.pool.begin().await?;

        // Acquire SQLite's writer lock before inspecting logical references.
        // This keeps the guard, provider deletion, and soft-reference cleanup
        // in one application-owned transaction without a physical FK/trigger.
        let locked = sqlx::query(
            "UPDATE providers SET updated_at = updated_at WHERE provider_id = ?",
        )
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
        // IDMM backup is RESTRICT; arrays are filtered in order; defaults are
        // deleted and optional references are set to null.
        let preference_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT key, value FROM client_preferences \
             WHERE key = 'idmm_backup_provider_id' \
                OR key = 'agent.model_failover' \
                OR key = 'nomi.collaborationModels' \
                OR key = 'nomi.defaultModel' \
                OR key = 'knowledge.autogenModel' \
                OR key = 'tools.imageGenerationModel' \
                OR key = 'tools.speechToText' \
                OR key LIKE 'channels.%.defaultModel'",
        )
        .fetch_all(&mut *transaction)
        .await?;
        let mut preference_actions = Vec::new();
        for (key, value) in preference_rows {
            match provider_preference_delete_action(&key, &value, id)? {
                ProviderPreferenceDeleteAction::Keep => {}
                ProviderPreferenceDeleteAction::Restrict => {
                    return Err(DbError::Conflict(
                        "provider is still referenced by an IDMM backup preference".to_owned(),
                    ));
                }
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
                ProviderPreferenceDeleteAction::Keep
                | ProviderPreferenceDeleteAction::Restrict => unreachable!(),
            }
        }

        sqlx::query("DELETE FROM model_profiles WHERE provider_id = ?")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        // Dual-write rule 3: cascade the provider delete to the new catalog
        // tables in the same transaction.
        sqlx::query("DELETE FROM provider_models WHERE provider_id = ?")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM provider_connections WHERE provider_id = ?")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "UPDATE preset_model_preferences SET provider_id = NULL WHERE provider_id = ?",
        )
        .bind(id)
        .execute(&mut *transaction)
        .await?;

        transaction.commit().await?;
        Ok(())
    }
}

/// Detect SQLite UNIQUE constraint violation (codes 2067 / 1555).
fn is_unique_violation(err: &dyn sqlx::error::DatabaseError) -> bool {
    err.code().is_some_and(|c| c == "2067" || c == "1555")
}

/// Merge partial update params into an existing provider, returning a new instance.
fn merge_update(existing: Provider, params: UpdateProviderParams<'_>) -> Provider {
    let now = nomifun_common::now_ms();
    Provider {
        id: existing.id,
        provider_id: existing.provider_id,
        platform: params.platform.unwrap_or(&existing.platform).to_string(),
        name: params.name.unwrap_or(&existing.name).to_string(),
        base_url: params.base_url.unwrap_or(&existing.base_url).to_string(),
        api_key_encrypted: params
            .api_key_encrypted
            .unwrap_or(&existing.api_key_encrypted)
            .to_string(),
        models: params.models.unwrap_or(&existing.models).to_string(),
        enabled: params.enabled.unwrap_or(existing.enabled),
        capabilities: params.capabilities.unwrap_or(&existing.capabilities).to_string(),
        model_context_limits: params
            .model_context_limits
            .map_or(existing.model_context_limits, |v| v.map(String::from)),
        model_protocols: params
            .model_protocols
            .map_or(existing.model_protocols, |v| v.map(String::from)),
        model_descriptions: params
            .model_descriptions
            .map_or(existing.model_descriptions, |v| v.map(String::from)),
        model_enabled: params
            .model_enabled
            .map_or(existing.model_enabled, |v| v.map(String::from)),
        model_health: params
            .model_health
            .map_or(existing.model_health, |v| v.map(String::from)),
        bedrock_config: params
            .bedrock_config
            .map_or(existing.bedrock_config, |v| v.map(String::from)),
        is_full_url: params.is_full_url.unwrap_or(existing.is_full_url),
        sort_order: params.sort_order.unwrap_or(existing.sort_order),
        created_at: existing.created_at,
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;

    const CALLER_PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000010";
    const DUPLICATE_PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000011";

    async fn setup() -> (SqliteProviderRepository, crate::Database) {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteProviderRepository::new(db.pool().clone());
        (repo, db)
    }

    fn sample_params() -> CreateProviderParams<'static> {
        CreateProviderParams {
            provider_id: None,
            platform: "anthropic",
            name: "Anthropic",
            base_url: "https://api.anthropic.com",
            api_key_encrypted: "encrypted_key_data",
            models: r#"["claude-sonnet-4-20250514"]"#,
            enabled: true,
            capabilities: r#"[{"type":"text"}]"#,
            model_context_limits: None,
            model_protocols: None,
            model_descriptions: None,
            model_enabled: None,
            model_health: None,
            bedrock_config: None,
            is_full_url: false,
            sort_order: None,
        }
    }

    #[tokio::test]
    async fn list_empty() {
        let (repo, _db) = setup().await;
        let providers = repo.list().await.unwrap();
        assert!(providers.is_empty());
    }

    #[tokio::test]
    async fn create_returns_populated_fields() {
        let (repo, _db) = setup().await;
        let p = repo.create(sample_params()).await.unwrap();

        assert!(nomifun_common::ProviderId::parse(p.provider_id.clone()).is_ok());
        assert_eq!(p.platform, "anthropic");
        assert_eq!(p.name, "Anthropic");
        assert_eq!(p.base_url, "https://api.anthropic.com");
        assert_eq!(p.api_key_encrypted, "encrypted_key_data");
        assert!(p.enabled);
        assert!(p.model_context_limits.is_none());
        assert!(p.model_protocols.is_none());
        assert!(p.bedrock_config.is_none());
        assert!(p.created_at > 0);
        assert_eq!(p.created_at, p.updated_at);
    }

    #[tokio::test]
    async fn create_with_caller_supplied_id() {
        let (repo, _db) = setup().await;
        let p = repo
            .create(CreateProviderParams {
                provider_id: Some(CALLER_PROVIDER_ID),
                ..sample_params()
            })
            .await
            .unwrap();

        assert_eq!(p.provider_id, CALLER_PROVIDER_ID);
        assert_eq!(p.platform, "anthropic");

        let found = repo.find_by_id(CALLER_PROVIDER_ID).await.unwrap().unwrap();
        assert_eq!(found.provider_id, CALLER_PROVIDER_ID);
    }

    #[tokio::test]
    async fn create_rejects_invalid_caller_supplied_id() {
        let (repo, _db) = setup().await;
        let err = repo
            .create(CreateProviderParams {
                provider_id: Some("my-custom-id-1"),
                ..sample_params()
            })
            .await
            .unwrap_err();

        assert!(
            matches!(err, DbError::Conflict(ref message) if message.contains("invalid provider_id")),
            "expected invalid provider_id conflict, got: {err:?}"
        );
        assert!(repo.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn create_with_duplicate_caller_id_returns_conflict() {
        let (repo, _db) = setup().await;
        repo.create(CreateProviderParams {
            provider_id: Some(DUPLICATE_PROVIDER_ID),
            ..sample_params()
        })
        .await
        .unwrap();

        let err = repo
            .create(CreateProviderParams {
                provider_id: Some(DUPLICATE_PROVIDER_ID),
                ..sample_params()
            })
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::Conflict(_)));
    }

    #[tokio::test]
    async fn create_then_find_by_id() {
        let (repo, _db) = setup().await;
        let created = repo.create(sample_params()).await.unwrap();

        let found = repo.find_by_id(&created.provider_id).await.unwrap().unwrap();
        assert_eq!(found.provider_id, created.provider_id);
        assert_eq!(found.platform, "anthropic");
        assert_eq!(found.models, r#"["claude-sonnet-4-20250514"]"#);
    }

    #[tokio::test]
    async fn find_by_id_nonexistent() {
        let (repo, _db) = setup().await;
        assert!(repo.find_by_id("no_such_id").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_returns_all_ordered_by_created_at() {
        let (repo, _db) = setup().await;
        let p1 = repo.create(sample_params()).await.unwrap();
        let p2 = repo
            .create(CreateProviderParams {
                platform: "openai",
                name: "OpenAI",
                base_url: "https://api.openai.com",
                ..sample_params()
            })
            .await
            .unwrap();

        let all = repo.list().await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].provider_id, p1.provider_id);
        assert_eq!(all[1].provider_id, p2.provider_id);
    }

    #[tokio::test]
    async fn update_partial_fields() {
        let (repo, _db) = setup().await;
        let created = repo.create(sample_params()).await.unwrap();

        let updated = repo
            .update(
                &created.provider_id,
                UpdateProviderParams {
                    name: Some("Anthropic Updated"),
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.name, "Anthropic Updated");
        assert!(!updated.enabled);
        // Unchanged fields preserved
        assert_eq!(updated.platform, "anthropic");
        assert_eq!(updated.base_url, "https://api.anthropic.com");
        assert!(updated.updated_at >= created.updated_at);
    }

    #[tokio::test]
    async fn update_api_key() {
        let (repo, _db) = setup().await;
        let created = repo.create(sample_params()).await.unwrap();

        let updated = repo
            .update(
                &created.provider_id,
                UpdateProviderParams {
                    api_key_encrypted: Some("new_encrypted_key"),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.api_key_encrypted, "new_encrypted_key");
    }

    #[tokio::test]
    async fn update_nonexistent_returns_not_found() {
        let (repo, _db) = setup().await;
        let err = repo.update("no_id", UpdateProviderParams::default()).await.unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn update_optional_json_fields() {
        let (repo, _db) = setup().await;
        let created = repo.create(sample_params()).await.unwrap();
        assert!(created.model_protocols.is_none());

        // Set optional field
        let updated = repo
            .update(
                &created.provider_id,
                UpdateProviderParams {
                    model_protocols: Some(Some(r#"{"model1":"openai"}"#)),
                    bedrock_config: Some(Some(r#"{"region":"us-east-1"}"#)),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.model_protocols.as_deref(), Some(r#"{"model1":"openai"}"#));
        assert_eq!(updated.bedrock_config.as_deref(), Some(r#"{"region":"us-east-1"}"#));

        // Clear optional field
        let cleared = repo
            .update(
                &created.provider_id,
                UpdateProviderParams {
                    model_protocols: Some(None),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(cleared.model_protocols.is_none());
        // bedrock_config should still be set
        assert!(cleared.bedrock_config.is_some());
    }

    #[tokio::test]
    async fn delete_existing() {
        let (repo, _db) = setup().await;
        let created = repo.create(sample_params()).await.unwrap();

        repo.delete(&created.provider_id).await.unwrap();
        assert!(repo.find_by_id(&created.provider_id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_nonexistent_returns_not_found() {
        let (repo, _db) = setup().await;
        let err = repo.delete("no_id").await.unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn create_syncs_provider_model_rows() {
        let (repo, db) = setup().await;
        let p = repo
            .create(CreateProviderParams {
                provider_id: None,
                platform: "openai",
                name: "P",
                base_url: "https://x.test/v1",
                api_key_encrypted: "enc",
                models: r#"["a","b"]"#,
                enabled: true,
                capabilities: "[]",
                model_context_limits: Some(r#"{"a":100}"#),
                model_protocols: None,
                model_descriptions: None,
                model_enabled: Some(r#"{"b":false}"#),
                model_health: None,
                bedrock_config: None,
                is_full_url: false,
                sort_order: None,
            })
            .await
            .unwrap();
        let rows: Vec<(String, i64, Option<i64>)> = sqlx::query_as(
            "SELECT model, enabled, context_limit FROM provider_models WHERE provider_id = ? ORDER BY sort_order",
        )
        .bind(&p.provider_id)
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(rows, vec![("a".into(), 1, Some(100)), ("b".into(), 0, None)]);
    }

    #[tokio::test]
    async fn update_membership_adds_and_removes_rows_preserving_profiles() {
        let (repo, db) = setup().await;
        let p = repo
            .create(CreateProviderParams {
                provider_id: None,
                platform: "openai",
                name: "P",
                base_url: "https://x.test/v1",
                api_key_encrypted: "enc",
                models: r#"["a","b"]"#,
                enabled: true,
                capabilities: "[]",
                model_context_limits: None,
                model_protocols: None,
                model_descriptions: None,
                model_enabled: None,
                model_health: None,
                bedrock_config: None,
                is_full_url: false,
                sort_order: None,
            })
            .await
            .unwrap();
        // Manually mark `a` with a user profile.
        sqlx::query(
            "UPDATE provider_models SET tasks='[\"chat\"]', source='user' WHERE provider_id=? AND model='a'",
        )
        .bind(&p.provider_id)
        .execute(db.pool())
        .await
        .unwrap();
        repo.update(
            &p.provider_id,
            UpdateProviderParams {
                models: Some(r#"["a","c"]"#),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT model, tasks, source FROM provider_models WHERE provider_id = ? ORDER BY sort_order",
        )
        .bind(&p.provider_id)
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0],
            ("a".into(), r#"["chat"]"#.into(), "user".into()),
            "existing row's profile untouched"
        );
        assert_eq!(rows[1].0, "c");
    }

    #[tokio::test]
    async fn update_map_param_is_whole_map_replacement_over_all_rows() {
        let (repo, db) = setup().await;
        let p = repo
            .create(CreateProviderParams {
                provider_id: None,
                platform: "openai",
                name: "P",
                base_url: "https://x.test/v1",
                api_key_encrypted: "enc",
                models: r#"["a","b"]"#,
                enabled: true,
                capabilities: "[]",
                model_context_limits: Some(r#"{"a":100,"b":200}"#),
                model_protocols: Some(r#"{"a":"openai"}"#),
                model_descriptions: None,
                model_enabled: Some(r#"{"b":false}"#),
                model_health: None,
                bedrock_config: None,
                is_full_url: false,
                sort_order: None,
            })
            .await
            .unwrap();

        // Replace model_enabled with a map that only covers `a`; `b` must be
        // reset to the enabled default (1). Explicitly clear the context
        // limits map; both rows must be reset to NULL. Protocol map is not
        // supplied, so protocol values stay put.
        repo.update(
            &p.provider_id,
            UpdateProviderParams {
                model_enabled: Some(Some(r#"{"a":false}"#)),
                model_context_limits: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let rows: Vec<(String, i64, Option<i64>, Option<String>)> = sqlx::query_as(
            "SELECT model, enabled, context_limit, protocol FROM provider_models \
             WHERE provider_id = ? ORDER BY sort_order",
        )
        .bind(&p.provider_id)
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(
            rows,
            vec![
                ("a".into(), 0, None, Some("openai".into())),
                ("b".into(), 1, None, None),
            ]
        );
    }

    #[tokio::test]
    async fn delete_cascades_provider_models_and_connections() {
        let (repo, db) = setup().await;
        let p = repo
            .create(CreateProviderParams {
                models: r#"["a","b"]"#,
                ..sample_params()
            })
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO provider_connections \
                (connection_id, provider_id, role, base_url, credentials_encrypted, created_at, updated_at) \
             VALUES ('0190f5fe-7c00-7a00-8000-0000000000aa', ?, 'voice', 'https://voice.test', 'enc', 1, 1)",
        )
        .bind(&p.provider_id)
        .execute(db.pool())
        .await
        .unwrap();

        repo.delete(&p.provider_id).await.unwrap();

        let models: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_models")
            .fetch_one(db.pool())
            .await
            .unwrap();
        let connections: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM provider_connections")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!((models, connections), (0, 0));
    }

    #[tokio::test]
    async fn delete_then_list_excludes_deleted() {
        let (repo, _db) = setup().await;
        let p1 = repo.create(sample_params()).await.unwrap();
        let p2 = repo
            .create(CreateProviderParams {
                name: "Other",
                ..sample_params()
            })
            .await
            .unwrap();

        repo.delete(&p1.provider_id).await.unwrap();

        let all = repo.list().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].provider_id, p2.provider_id);
    }

}
