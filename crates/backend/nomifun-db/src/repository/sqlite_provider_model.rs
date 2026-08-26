use nomifun_common::{now_ms, ProviderId};
use serde_json::{Value, json};
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct ConversationModelRef {
    provider_id: String,
    model: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ConversationModelPool {
    Automatic,
    Single(ConversationModelRef),
    Range(Vec<ConversationModelRef>),
}

fn parse_conversation_model_ref(
    value: &Value,
    context: &str,
) -> Result<ConversationModelRef, DbError> {
    let object = value
        .as_object()
        .ok_or_else(|| DbError::Conflict(format!("{context} must be an object")))?;
    let provider_id = object
        .get("provider_id")
        .and_then(Value::as_str)
        .ok_or_else(|| DbError::Conflict(format!("{context}.provider_id is missing")))?;
    let model = object
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| DbError::Conflict(format!("{context}.model is missing")))?;
    ProviderId::parse(provider_id).map_err(|error| {
        DbError::Conflict(format!("{context}.provider_id is not canonical: {error}"))
    })?;
    if model.trim().is_empty() || model.trim() != model {
        return Err(DbError::Conflict(format!(
            "{context}.model must be a trimmed, non-empty string"
        )));
    }
    Ok(ConversationModelRef {
        provider_id: provider_id.to_owned(),
        model: model.to_owned(),
    })
}

fn parse_conversation_model_json(
    raw: &str,
) -> Result<(ConversationModelRef, ConversationModelRef), DbError> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|error| DbError::Conflict(format!("Conversation model is invalid JSON: {error}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| DbError::Conflict("Conversation model must be an object".to_owned()))?;
    let base = parse_conversation_model_ref(&value, "Conversation model")?;
    let effective = match object.get("use_model") {
        None | Some(Value::Null) => base.model.as_str(),
        Some(Value::String(value)) => value.as_str(),
        Some(_) => {
            return Err(DbError::Conflict(
                "Conversation model.use_model must be a string or null".to_owned(),
            ));
        }
    };
    if effective.trim().is_empty() || effective.trim() != effective {
        return Err(DbError::Conflict(
            "Conversation model.use_model must be trimmed and non-empty".to_owned(),
        ));
    }
    Ok((
        base.clone(),
        ConversationModelRef {
            provider_id: base.provider_id,
            model: effective.to_owned(),
        },
    ))
}

fn replace_model_reference(
    raw: &str,
    provider_id: &str,
    target: &str,
    replacement: Option<&ConversationModelRef>,
) -> Result<Option<String>, DbError> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|error| DbError::Conflict(format!("Conversation model is invalid JSON: {error}")))?;
    let Some(replacement) = replacement else {
        return Ok(None);
    };
    let mut object = value
        .as_object()
        .cloned()
        .ok_or_else(|| DbError::Conflict("Conversation model must be an object".to_owned()))?;
    let base_matches = object
        .get("provider_id")
        .and_then(Value::as_str)
        .is_some_and(|value| value == provider_id)
        && object
            .get("model")
            .and_then(Value::as_str)
            .is_some_and(|value| value == target);
    let use_model_matches = object
        .get("use_model")
        .and_then(Value::as_str)
        .is_some_and(|value| value == target);

    // ProviderWithModel has one provider_id for both the catalog model and the
    // optional runtime override. If the catalog model is still valid and only
    // use_model disappeared, preserve the catalog selection when the fallback
    // stays on that provider. A cross-provider fallback cannot be represented
    // without changing the catalog pair, so rewrite both fields then.
    let preserve_catalog = !base_matches
        && use_model_matches
        && object
            .get("provider_id")
            .and_then(Value::as_str)
            .is_some_and(|value| value == replacement.provider_id);
    if !preserve_catalog {
        object.insert(
            "provider_id".to_owned(),
            Value::String(replacement.provider_id.clone()),
        );
        object.insert("model".to_owned(), Value::String(replacement.model.clone()));
    }
    if object.contains_key("use_model") && (use_model_matches || base_matches) {
        object.insert(
            "use_model".to_owned(),
            Value::String(replacement.model.clone()),
        );
    }
    Ok(Some(Value::Object(object).to_string()))
}

fn parse_conversation_model_pool(raw: &str) -> Result<ConversationModelPool, DbError> {
    let value: Value = serde_json::from_str(raw).map_err(|error| {
        DbError::Conflict(format!(
            "Conversation execution model pool is invalid JSON: {error}"
        ))
    })?;
    let object = value.as_object().ok_or_else(|| {
        DbError::Conflict("Conversation execution model pool must be an object".to_owned())
    })?;
    match object.get("mode").and_then(Value::as_str) {
        Some("automatic") => Ok(ConversationModelPool::Automatic),
        Some("single") => Ok(ConversationModelPool::Single(
            parse_conversation_model_ref(
                object.get("model").ok_or_else(|| {
                    DbError::Conflict(
                        "Conversation single execution model pool requires model".to_owned(),
                    )
                })?,
                "Conversation execution model pool.model",
            )?,
        )),
        Some("range") => {
            let models = object
                .get("models")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    DbError::Conflict(
                        "Conversation execution model range requires models".to_owned(),
                    )
                })?
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    parse_conversation_model_ref(
                        value,
                        &format!("Conversation execution model pool.models[{index}]"),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            if models.is_empty() {
                return Err(DbError::Conflict(
                    "Conversation execution model range requires at least one model".to_owned(),
                ));
            }
            Ok(ConversationModelPool::Range(models))
        }
        Some(mode) => Err(DbError::Conflict(format!(
            "Conversation execution model pool has unsupported mode '{mode}'"
        ))),
        None => Err(DbError::Conflict(
            "Conversation execution model pool requires a mode".to_owned(),
        )),
    }
}

fn conversation_model_pool_json(pool: &ConversationModelPool) -> String {
    match pool {
        ConversationModelPool::Automatic => json!({"mode":"automatic"}).to_string(),
        ConversationModelPool::Single(model) => json!({
            "mode": "single",
            "model": {
                "provider_id": model.provider_id,
                "model": model.model,
            }
        })
        .to_string(),
        ConversationModelPool::Range(models) => json!({
            "mode": "range",
            "models": models.iter().map(|model| json!({
                "provider_id": model.provider_id,
                "model": model.model,
            })).collect::<Vec<_>>(),
        })
        .to_string(),
    }
}

fn model_ref_matches(model: &ConversationModelRef, provider_id: &str, target: &str) -> bool {
    model.provider_id == provider_id && model.model == target
}

fn model_ref_is_live(
    model: &ConversationModelRef,
    live_models: &[(String, String)],
) -> bool {
    live_models
        .iter()
        .any(|(provider_id, model_id)| {
            provider_id == &model.provider_id && model_id == &model.model
        })
}

fn first_live_pool_model(
    pool: &ConversationModelPool,
    provider_id: &str,
    target: &str,
    live_models: &[(String, String)],
) -> Option<ConversationModelRef> {
    let candidates = match pool {
        ConversationModelPool::Automatic => return None,
        ConversationModelPool::Single(model) => std::slice::from_ref(model),
        ConversationModelPool::Range(models) => models.as_slice(),
    };
    candidates
        .iter()
        .find(|model| {
            !model_ref_matches(model, provider_id, target)
                && model_ref_is_live(model, live_models)
        })
        .cloned()
}

fn first_live_model(
    live_models: &[(String, String)],
) -> Option<ConversationModelRef> {
    live_models
        .first()
        .map(|(provider_id, model)| ConversationModelRef {
            provider_id: provider_id.clone(),
            model: model.clone(),
        })
}

fn pool_contains_target(
    pool: &ConversationModelPool,
    provider_id: &str,
    target: &str,
) -> bool {
    match pool {
        ConversationModelPool::Automatic => false,
        ConversationModelPool::Single(model) => model_ref_matches(model, provider_id, target),
        ConversationModelPool::Range(models) => models
            .iter()
            .any(|model| model_ref_matches(model, provider_id, target)),
    }
}

fn rewrite_conversation_pool(
    pool: ConversationModelPool,
    provider_id: &str,
    target: &str,
    replacement: Option<&ConversationModelRef>,
    lead: Option<&ConversationModelRef>,
    live_models: &[(String, String)],
) -> Option<ConversationModelPool> {
    match pool {
        ConversationModelPool::Automatic => Some(ConversationModelPool::Automatic),
        ConversationModelPool::Single(model) => {
            if !model_ref_matches(&model, provider_id, target) {
                return Some(ConversationModelPool::Single(model));
            }
            replacement.cloned().map(ConversationModelPool::Single)
        }
        ConversationModelPool::Range(models) => {
            let mut retained = models
                .into_iter()
                .filter(|model| {
                    !model_ref_matches(model, provider_id, target)
                        && model_ref_is_live(model, live_models)
                })
                .collect::<Vec<_>>();
            if retained.is_empty() {
                if let Some(replacement) = replacement {
                    return Some(ConversationModelPool::Range(vec![replacement.clone()]));
                }
                return None;
            }
            if let Some(lead) = lead
                && !retained.iter().any(|model| model == lead)
                && model_ref_is_live(lead, live_models)
            {
                retained.insert(0, lead.clone());
            }
            Some(ConversationModelPool::Range(retained))
        }
    }
}

async fn reconcile_idle_conversation_model_references(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    provider_id: &str,
    target_model: &str,
    now: i64,
) -> Result<(), DbError> {
    // Only inspect rows whose JSON shape can actually mention the target.
    // Malformed unrelated legacy data must not make an otherwise safe model
    // deletion impossible, while malformed matching data fails closed below.
    let conversations: Vec<(
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT conversation_id, status, active_turn_operation_id, model, \
                execution_model_pool \
         FROM conversations \
         WHERE ((json_valid(model) \
                 AND json_extract(model, '$.provider_id') = ?1 \
                 AND (json_extract(model, '$.model') = ?2 \
                      OR json_extract(model, '$.use_model') = ?2)) \
             OR (json_valid(execution_model_pool) \
                 AND ((json_extract(execution_model_pool, '$.mode') = 'single' \
                       AND json_extract(execution_model_pool, '$.model.provider_id') = ?1 \
                       AND json_extract(execution_model_pool, '$.model.model') = ?2) \
                      OR (json_extract(execution_model_pool, '$.mode') = 'range' \
                          AND EXISTS (\
                              SELECT 1 FROM json_each(execution_model_pool, '$.models') item \
                              WHERE json_extract(item.value, '$.provider_id') = ?1 \
                                AND json_extract(item.value, '$.model') = ?2\
                          ))))) \
           AND NOT EXISTS (\
               SELECT 1 FROM conversation_execution_links retained_attempt \
               WHERE retained_attempt.conversation_id = conversations.conversation_id \
                 AND retained_attempt.relation = 'attempt'\
           ) \
           AND NOT EXISTS (\
               SELECT 1 FROM creative_studio_agent_sessions creative_session \
               WHERE creative_session.conversation_id = conversations.conversation_id\
           ) \
         ORDER BY conversation_id",
    )
    .bind(provider_id)
    .bind(target_model)
    .fetch_all(&mut **transaction)
    .await?;

    // Active execution participants are a real running use, unlike a
    // completed/paused execution snapshot. Keep the model until that work is
    // settled; historical execution rows remain readable after deletion.
    let active_execution: Option<String> = sqlx::query_scalar(
        "SELECT execution.execution_id \
         FROM agent_execution_participants participant \
         JOIN agent_executions execution \
           ON execution.execution_id = participant.execution_id \
         WHERE participant.provider_id = ?1 AND participant.model = ?2 \
           AND participant.retired_in_revision IS NULL \
           AND execution.deleted_at IS NULL \
           AND (execution.status IN ('running', 'waiting_input') \
                OR EXISTS (\
                    SELECT 1 FROM agent_execution_attempts attempt \
                    WHERE attempt.execution_id = execution.execution_id \
                      AND attempt.participant_id = participant.participant_id \
                      AND attempt.status IN ('queued', 'running', 'waiting_input')\
                )) \
         ORDER BY execution.updated_at ASC, execution.execution_id ASC LIMIT 1",
    )
    .bind(provider_id)
    .bind(target_model)
    .fetch_optional(&mut **transaction)
    .await?;
    if let Some(execution_id) = active_execution {
        return Err(DbError::Conflict(format!(
            "provider model '{provider_id}/{target_model}' is used by running Agent Execution '{execution_id}'"
        )));
    }
    if conversations.is_empty() {
        return Ok(());
    }

    let live_models: Vec<(String, String)> = sqlx::query_as(
        "SELECT provider.provider_id, model.model \
         FROM provider_models model \
         JOIN providers provider ON provider.provider_id = model.provider_id \
         JOIN provider_model_capabilities capability \
           ON capability.provider_id = model.provider_id \
          AND capability.model = model.model \
          AND capability.task = 'chat' \
         WHERE provider.enabled = 1 AND model.enabled = 1 \
           AND NOT (model.provider_id = ?1 AND model.model = ?2) \
         GROUP BY provider.provider_id, model.model \
         ORDER BY provider.sort_order ASC, provider.created_at ASC, provider.id ASC, \
                  model.sort_order ASC, model.id ASC",
    )
    .bind(provider_id)
    .bind(target_model)
    .fetch_all(&mut **transaction)
    .await?;

    let first_live_model = first_live_model(&live_models);
    let mut updates = Vec::with_capacity(conversations.len());
    for (
        conversation_id,
        status,
        active_turn_operation_id,
        raw_model,
        raw_pool,
    ) in conversations
    {
        if status == "running" || active_turn_operation_id.is_some() {
            return Err(DbError::Conflict(format!(
                "provider model '{provider_id}/{target_model}' is used by running Conversation '{conversation_id}'"
            )));
        }

        let parsed_model = raw_model
            .as_deref()
            .map(parse_conversation_model_json)
            .transpose()?;
        let parsed_pool = raw_pool
            .as_deref()
            .map(parse_conversation_model_pool)
            .transpose()?;
        let model_matches = parsed_model.as_ref().is_some_and(|(base, effective)| {
            model_ref_matches(base, provider_id, target_model)
                || model_ref_matches(effective, provider_id, target_model)
        });
        let pool_matches = parsed_pool
            .as_ref()
            .is_some_and(|pool| pool_contains_target(pool, provider_id, target_model));
        if !model_matches && !pool_matches {
            continue;
        }

        let current_lead = parsed_model.as_ref().and_then(|(base, effective)| {
            [effective, base]
                .into_iter()
                .find(|model| {
                    !model_ref_matches(model, provider_id, target_model)
                        && model_ref_is_live(model, &live_models)
                })
                .cloned()
        });
        let pool_replacement = parsed_pool.as_ref().and_then(|pool| {
            first_live_pool_model(pool, provider_id, target_model, &live_models)
        });
        let replacement = if model_matches {
            pool_replacement
                .or(current_lead.clone())
                .or_else(|| first_live_model.clone())
        } else {
            current_lead.clone()
                .or(pool_replacement)
                .or_else(|| first_live_model.clone())
        };
        let next_model = if model_matches {
            raw_model
                .as_deref()
                .map(|raw| {
                    replace_model_reference(
                        raw,
                        provider_id,
                        target_model,
                        replacement.as_ref(),
                    )
                })
                .transpose()?
                .flatten()
        } else {
            raw_model.clone()
        };
        let lead = if model_matches {
            replacement.as_ref()
        } else {
            current_lead.as_ref().or(replacement.as_ref())
        };
        let next_pool = parsed_pool.and_then(|pool| {
            if !pool_contains_target(&pool, provider_id, target_model) {
                return Some(pool);
            }
            rewrite_conversation_pool(
                pool,
                provider_id,
                target_model,
                replacement.as_ref(),
                lead,
                &live_models,
            )
        });
        let next_pool = next_pool.map(|pool| conversation_model_pool_json(&pool));
        if next_model == raw_model && next_pool == raw_pool {
            continue;
        }
        updates.push((conversation_id, next_model, next_pool));
    }

    for (conversation_id, model, pool) in updates {
        let updated = sqlx::query(
            "UPDATE conversations \
             SET model = ?, execution_model_pool = ?, \
                 updated_at = MAX(updated_at, ?) \
             WHERE conversation_id = ? \
               AND status <> 'running' \
               AND active_turn_operation_id IS NULL",
        )
        .bind(model)
        .bind(pool)
        .bind(now)
        .bind(&conversation_id)
        .execute(&mut **transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(DbError::Conflict(format!(
                "conversation '{conversation_id}' changed while repairing provider model references"
            )));
        }
    }
    Ok(())
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

        let live_template_run: Option<String> = sqlx::query_scalar(
            "SELECT template_run_id FROM creative_studio_template_runs AS run \
             WHERE run.status IN ('requested', 'awaiting-review', 'queued', 'running') \
               AND EXISTS (\
                   SELECT 1 FROM json_each(run.aggregate_json, '$.templateSnapshot.steps') AS step \
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
             ORDER BY run.updated_at ASC, run.template_run_id ASC LIMIT 1",
        )
        .bind(provider_id.as_str())
        .bind(&plan.model)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(template_run_id) = live_template_run {
            return Err(DbError::Conflict(format!(
                "provider model '{}/{}' is pinned by nonterminal template run '{template_run_id}'",
                provider_id, plan.model
            )));
        }

        reconcile_idle_conversation_model_references(
            &mut transaction,
            provider_id.as_str(),
            &plan.model,
            now_ms(),
        )
        .await?;

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

        for cleanup in &plan.cleanup.templates {
            let replacement = &cleanup.replacement;
            if replacement.template_id != cleanup.template_id
                || replacement.revision != cleanup.expected_revision + 1
            {
                return Err(DbError::Conflict(format!(
                    "creative studio template '{}' cleanup replacement must preserve its ID and increment revision once",
                    cleanup.template_id
                )));
            }
            let updated = sqlx::query(
                "UPDATE creative_studio_templates \
                 SET revision = ?, name = ?, description = ?, category = ?, visibility = ?, \
                     definition_json = ?, updated_at = ? \
                 WHERE template_id = ? AND revision = ?",
            )
            .bind(replacement.revision)
            .bind(&replacement.name)
            .bind(&replacement.description)
            .bind(&replacement.category)
            .bind(&replacement.visibility)
            .bind(&replacement.definition_json)
            .bind(replacement.updated_at)
            .bind(&cleanup.template_id)
            .bind(cleanup.expected_revision)
            .execute(&mut *transaction)
            .await?;
            if updated.rows_affected() != 1 {
                return Err(DbError::Conflict(format!(
                    "creative studio template '{}' changed during provider model cleanup; expected revision {}",
                    cleanup.template_id, cleanup.expected_revision
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
