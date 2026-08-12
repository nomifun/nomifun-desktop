use std::collections::HashSet;

use nomifun_common::now_ms;
use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::{NewProviderModelCapability, ProviderModelCapabilityRow};
use crate::repository::provider_model_capability::IProviderModelCapabilityRepository;

#[derive(Clone, Debug)]
pub struct SqliteProviderModelCapabilityRepository {
    pool: SqlitePool,
}

impl SqliteProviderModelCapabilityRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn validate_capabilities(capabilities: &[NewProviderModelCapability<'_>]) -> Result<(), DbError> {
    if capabilities.is_empty() {
        return Err(DbError::Conflict(
            "provider model must have at least one capability".into(),
        ));
    }

    let mut tasks = HashSet::with_capacity(capabilities.len());
    for capability in capabilities {
        let task = capability.task.trim();
        if task.is_empty() {
            return Err(DbError::Conflict(
                "capability task must not be blank".into(),
            ));
        }
        if !tasks.insert(task) {
            return Err(DbError::Conflict(format!(
                "capability task '{task}' is duplicated"
            )));
        }
        if capability.protocol.trim().is_empty() {
            return Err(DbError::Conflict(format!(
                "capability '{task}' protocol must not be blank"
            )));
        }
        if capability.connection_role.trim().is_empty() {
            return Err(DbError::Conflict(format!(
                "capability '{task}' connection_role must not be blank"
            )));
        }
        let traits: serde_json::Value = serde_json::from_str(capability.traits)
            .map_err(|error| DbError::Conflict(format!("invalid capability traits: {error}")))?;
        if !traits.is_array() {
            return Err(DbError::Conflict(
                "capability traits must be a JSON array".into(),
            ));
        }
        let provider_params: serde_json::Value = serde_json::from_str(capability.provider_params)
            .map_err(|error| {
            DbError::Conflict(format!("invalid capability provider_params: {error}"))
        })?;
        if !provider_params.is_object() {
            return Err(DbError::Conflict(
                "capability provider_params must be a JSON object".into(),
            ));
        }
    }
    Ok(())
}

pub(crate) async fn replace_for_model_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    provider_id: &str,
    model: &str,
    capabilities: &[NewProviderModelCapability<'_>],
    now: i64,
) -> Result<bool, DbError> {
    validate_capabilities(capabilities)?;
    let mut configuration_changed = false;

    for connection_role in capabilities
        .iter()
        .map(|capability| capability.connection_role.trim())
        .filter(|role| *role != "default")
        .collect::<HashSet<_>>()
    {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM provider_connections \
             WHERE provider_id = ? AND role = ?)",
        )
        .bind(provider_id)
        .bind(connection_role)
        .fetch_one(&mut **transaction)
        .await?;
        if !exists {
            return Err(DbError::Conflict(format!(
                "capability connection_role '{connection_role}' is not configured for provider '{provider_id}'"
            )));
        }
    }

    for capability in capabilities {
        let result = sqlx::query(
            "INSERT INTO provider_model_capabilities \
                (provider_id, model, task, traits, protocol, connection_role, \
                 base_url_override, endpoint, poll_endpoint, content_endpoint, \
                 realtime_endpoint, allow_cross_origin_credentials, provider_params, \
                 context_limit, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(provider_id, model, task) DO UPDATE SET \
                 health = NULL, \
                 health_checked_at = NULL, \
                 traits = excluded.traits, \
                 protocol = excluded.protocol, \
                 connection_role = excluded.connection_role, \
                 base_url_override = excluded.base_url_override, \
                 endpoint = excluded.endpoint, \
                 poll_endpoint = excluded.poll_endpoint, \
                 content_endpoint = excluded.content_endpoint, \
                 realtime_endpoint = excluded.realtime_endpoint, \
                 allow_cross_origin_credentials = excluded.allow_cross_origin_credentials, \
                 provider_params = excluded.provider_params, \
                 context_limit = excluded.context_limit, \
                 updated_at = excluded.updated_at \
             WHERE NOT ( \
                 provider_model_capabilities.traits IS excluded.traits AND \
                 provider_model_capabilities.protocol IS excluded.protocol AND \
                 provider_model_capabilities.connection_role IS excluded.connection_role AND \
                 provider_model_capabilities.base_url_override IS excluded.base_url_override AND \
                 provider_model_capabilities.endpoint IS excluded.endpoint AND \
                 provider_model_capabilities.poll_endpoint IS excluded.poll_endpoint AND \
                 provider_model_capabilities.content_endpoint IS excluded.content_endpoint AND \
                 provider_model_capabilities.realtime_endpoint IS excluded.realtime_endpoint AND \
                 provider_model_capabilities.allow_cross_origin_credentials \
                     IS excluded.allow_cross_origin_credentials AND \
                 provider_model_capabilities.provider_params IS excluded.provider_params AND \
                 provider_model_capabilities.context_limit IS excluded.context_limit \
             )",
        )
        .bind(provider_id)
        .bind(model)
        .bind(capability.task.trim())
        .bind(capability.traits)
        .bind(capability.protocol.trim())
        .bind(capability.connection_role.trim())
        .bind(capability.base_url_override)
        .bind(capability.endpoint)
        .bind(capability.poll_endpoint)
        .bind(capability.content_endpoint)
        .bind(capability.realtime_endpoint)
        .bind(capability.allow_cross_origin_credentials)
        .bind(capability.provider_params)
        .bind(capability.context_limit)
        .bind(now)
        .bind(now)
        .execute(&mut **transaction)
        .await?;
        configuration_changed |= result.rows_affected() > 0;
    }

    let placeholders = vec!["?"; capabilities.len()].join(", ");
    let sql = format!(
        "DELETE FROM provider_model_capabilities \
         WHERE provider_id = ? AND model = ? AND task NOT IN ({placeholders})"
    );
    let mut delete = sqlx::query(&sql).bind(provider_id).bind(model);
    for capability in capabilities {
        delete = delete.bind(capability.task.trim());
    }
    let deleted = delete.execute(&mut **transaction).await?;
    configuration_changed |= deleted.rows_affected() > 0;
    Ok(configuration_changed)
}

/// Invalidate observations that were made through one provider connection.
/// Callers invoke this inside the same transaction that changes the effective
/// base URL, authentication, credentials, or connection parameters.
pub(crate) async fn clear_health_for_connection_role_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    provider_id: &str,
    connection_role: &str,
) -> Result<u64, DbError> {
    let result = sqlx::query(
        "UPDATE provider_model_capabilities \
         SET health = NULL, health_checked_at = NULL \
         WHERE provider_id = ? AND connection_role = ? \
           AND (health IS NOT NULL OR health_checked_at IS NOT NULL)",
    )
    .bind(provider_id)
    .bind(connection_role)
    .execute(&mut **transaction)
    .await?;
    Ok(result.rows_affected())
}

/// Advance the provider-wide invocation graph revision exactly once for the
/// surrounding transaction.
pub(crate) async fn bump_provider_config_revision_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    provider_id: &str,
) -> Result<(), DbError> {
    let result = sqlx::query(
        "UPDATE providers SET config_revision = config_revision + 1 WHERE provider_id = ?",
    )
    .bind(provider_id)
    .execute(&mut **transaction)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DbError::NotFound(format!(
            "Provider '{provider_id}' not found while advancing config revision"
        )));
    }
    Ok(())
}

#[async_trait::async_trait]
impl IProviderModelCapabilityRepository for SqliteProviderModelCapabilityRepository {
    async fn list(&self) -> Result<Vec<ProviderModelCapabilityRow>, DbError> {
        Ok(sqlx::query_as::<_, ProviderModelCapabilityRow>(
            "SELECT * FROM provider_model_capabilities \
             ORDER BY provider_id ASC, model ASC, task ASC, id ASC",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    async fn list_for_provider(
        &self,
        provider_id: &str,
    ) -> Result<Vec<ProviderModelCapabilityRow>, DbError> {
        Ok(sqlx::query_as::<_, ProviderModelCapabilityRow>(
            "SELECT * FROM provider_model_capabilities WHERE provider_id = ? \
             ORDER BY model ASC, task ASC, id ASC",
        )
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn list_for_model(
        &self,
        provider_id: &str,
        model: &str,
    ) -> Result<Vec<ProviderModelCapabilityRow>, DbError> {
        Ok(sqlx::query_as::<_, ProviderModelCapabilityRow>(
            "SELECT * FROM provider_model_capabilities \
             WHERE provider_id = ? AND model = ? ORDER BY task ASC, id ASC",
        )
        .bind(provider_id)
        .bind(model)
        .fetch_all(&self.pool)
        .await?)
    }

    async fn get(
        &self,
        provider_id: &str,
        model: &str,
        task: &str,
    ) -> Result<Option<ProviderModelCapabilityRow>, DbError> {
        Ok(sqlx::query_as::<_, ProviderModelCapabilityRow>(
            "SELECT * FROM provider_model_capabilities \
             WHERE provider_id = ? AND model = ? AND task = ?",
        )
        .bind(provider_id)
        .bind(model)
        .bind(task)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn set_health(
        &self,
        provider_id: &str,
        expected_config_revision: i64,
        model: &str,
        task: &str,
        health_json: Option<&str>,
    ) -> Result<bool, DbError> {
        let now = now_ms();
        let checked_at = health_json.map(|_| now);
        let result = sqlx::query(
            "UPDATE provider_model_capabilities \
             SET health = ?, health_checked_at = ? \
             WHERE provider_id = ? AND model = ? AND task = ? \
               AND EXISTS (\
                   SELECT 1 FROM providers \
                   WHERE provider_id = ? AND config_revision = ?\
               )",
        )
        .bind(health_json)
        .bind(checked_at)
        .bind(provider_id)
        .bind(model)
        .bind(task)
        .bind(provider_id)
        .bind(expected_config_revision)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}
