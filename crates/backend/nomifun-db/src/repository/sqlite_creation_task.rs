use nomifun_common::{
    CreationTaskId, CreativeStudioNodeId, CreativeStudioProjectId,
    CreativeStudioTemplateId, CreativeStudioTemplateRunId, CreativeStudioTemplateStepId,
    ProviderId, WorkshopAssetId,
};
#[cfg(test)]
use nomifun_common::validate_uuidv7;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Sqlite, SqlitePool, Transaction};

use crate::error::DbError;
use crate::models::CreationTaskRow;
use crate::repository::ICreationTaskRepository;
use crate::repository::creation_task::{
    CreateCreativeTaskParams, CreativeTaskOwnerRef, IdempotentCreationTask,
    UpdateCreationTaskParams,
};

/// SQLite-backed implementation of [`ICreationTaskRepository`].
#[derive(Clone, Debug)]
pub struct SqliteCreationTaskRepository {
    pool: SqlitePool,
}

impl SqliteCreationTaskRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct CreationTaskDbRow {
    conversation_id: Option<String>,
    message_id: Option<String>,
    creation_task_id: String,
    project_id: Option<String>,

    template_id: Option<String>,
    template_run_id: Option<String>,
    template_step_id: Option<String>,
    node_id: Option<String>,
    provider_id: String,
    model: String,
    capability: String,
    params: String,
    input_bindings: Option<String>,
    status: String,
    error: Option<String>,
    result_asset_ids: String,
    remote_task_id: Option<String>,
    attempt: i64,
    submitted_at: i64,
    started_at: Option<i64>,
    finished_at: Option<i64>,
    deleted_at: Option<i64>,
    request_fingerprint: Option<String>,
}

impl TryFrom<CreationTaskDbRow> for CreationTaskRow {
    type Error = DbError;

    fn try_from(row: CreationTaskDbRow) -> Result<Self, Self::Error> {
        let CreationTaskDbRow {
            conversation_id,
            message_id,
            creation_task_id,
            project_id,
            template_id,
            template_run_id,
            template_step_id,
            node_id,
            provider_id,
            model,
            capability,
            params,
            input_bindings,
            status,
            error,
            result_asset_ids,
            remote_task_id,
            attempt,
            submitted_at,
            started_at,
            finished_at,
            deleted_at,
            request_fingerprint,
        } = row;
        validate_creation_task_id(&creation_task_id)?;
        if let Some(id) = project_id.as_deref() {
            CreativeStudioProjectId::parse(id).map_err(|error| {
                DbError::Conflict(format!(
                    "creation task {creation_task_id} has invalid project_id {id:?}: {error}"
                ))
            })?;
        }
        if let Some(id) = template_id.as_deref() {
            CreativeStudioTemplateId::parse(id).map_err(|error| {
                DbError::Conflict(format!(
                    "creation task {creation_task_id} has invalid template_id {id:?}: {error}"
                ))
            })?;
        }
        if let Some(id) = template_run_id.as_deref() {
            CreativeStudioTemplateRunId::parse(id).map_err(|error| {
                DbError::Conflict(format!(
                    "creation task {creation_task_id} has invalid template_run_id {id:?}: {error}"
                ))
            })?;
        }
        if let Some(id) = template_step_id.as_deref() {
            CreativeStudioTemplateStepId::parse(id).map_err(|error| {
                DbError::Conflict(format!(
                    "creation task {creation_task_id} has invalid template_step_id {id:?}: {error}"
                ))
            })?;
        }
        if let Some(node_id) = &node_id {
            CreativeStudioNodeId::parse(node_id).map_err(|error| {
                DbError::Conflict(format!(
                    "creation task {creation_task_id} has invalid node_id {node_id:?}: {error}"
                ))
            })?;
        }
        ProviderId::parse(&provider_id).map_err(|error| {
            DbError::Conflict(format!(
                "creation task {creation_task_id} has invalid provider_id {provider_id:?}: {error}"
            ))
        })?;
        let canvas_owner = project_id.is_some()
            && node_id.is_some()

            && template_id.is_none()
            && template_run_id.is_none()
            && template_step_id.is_none();
        let template_owner = project_id.is_none()

            && node_id.is_none()
            && template_id.is_some()
            && template_run_id.is_some()
            && template_step_id.is_some();
        let conversation_owner = conversation_id.is_some() && message_id.is_some()
            && project_id.is_none() && node_id.is_none()
            && template_id.is_none() && template_run_id.is_none() && template_step_id.is_none();
        if let Some(id) = conversation_id.as_deref() { nomifun_common::ConversationId::parse(id).map_err(|e| DbError::Conflict(e.to_string()))?; }
        if let Some(id) = message_id.as_deref() { nomifun_common::MessageId::parse(id).map_err(|e| DbError::Conflict(e.to_string()))?; }
        let owner_branches = usize::from(conversation_owner) + usize::from(canvas_owner)
            + usize::from(template_owner);
        let valid_owner = request_fingerprint.is_some() && owner_branches == 1
            && (conversation_owner || (conversation_id.is_none() && message_id.is_none()));
        if !valid_owner {
            return Err(DbError::Conflict(format!(
                "creation task {creation_task_id} has an invalid tagged owner"
            )));
        }
        let canonical_result_asset_ids = canonicalize_result_asset_ids(&result_asset_ids)?;
        if canonical_result_asset_ids != result_asset_ids {
            return Err(DbError::Conflict(format!(
                "creation task {creation_task_id} result_asset_ids is not canonically encoded"
            )));
        }
        let canonical_input_bindings = canonicalize_input_bindings(input_bindings.as_deref())?;
        if canonical_input_bindings != input_bindings {
            return Err(DbError::Conflict(format!(
                "creation task {creation_task_id} input_bindings is not canonically encoded"
            )));
        }
        if deleted_at.is_some_and(|deleted_at| {
            deleted_at < submitted_at
                || !conversation_owner
                || node_id.is_some()
                || template_id.is_some()
                || template_run_id.is_some()
                || template_step_id.is_some()
                || !matches!(status.as_str(), "failed" | "canceled" | "succeeded")
        }) {
            return Err(DbError::Conflict(format!(
                "creation task {creation_task_id} has an invalid retirement tombstone"
            )));
        }
        Ok(Self {
            conversation_id,
            message_id,
            creation_task_id,
            project_id,
            template_id,
            template_run_id,
            template_step_id,
            node_id,
            provider_id,
            model,
            capability,
            params,
            input_bindings,
            status,
            error,
            result_asset_ids,
            remote_task_id,
            attempt,
            submitted_at,
            started_at,
            finished_at,
            deleted_at,
        })
    }
}

fn validate_creation_task_id(creation_task_id: &str) -> Result<(), DbError> {
    CreationTaskId::parse(creation_task_id).map_err(|error| {
        DbError::Conflict(format!(
            "Creation task creation_task_id '{creation_task_id}' is not a canonical UUIDv7: {error}"
        ))
    })?;
    Ok(())
}

fn provider_task_for_creation_capability(capability: &str) -> Option<&'static str> {
    match capability {
        "t2i" => Some("image_generation"),
        "i2i" | "inpaint" => Some("image_edit"),
        "t2v" | "i2v" | "v2v" => Some("video_generation"),
        "tts" => Some("speech_synthesis"),
        "music" => Some("music_generation"),
        "text" => Some("chat"),
        _ => None,
    }
}

#[derive(Debug, Clone)]
enum CanonicalTaskOwner {
    ConversationTurn { conversation_id: String, message_id: String },
    CanvasNode {
        project_id: String,
        node_id: String,
    },
    TemplateStep {
        template_id: String,
        template_run_id: String,
        template_step_id: String,
    },
}

fn normalize_canonical_owner(owner: CreativeTaskOwnerRef<'_>) -> Result<CanonicalTaskOwner, DbError> {
    match owner {
        CreativeTaskOwnerRef::ConversationTurn { conversation_id, message_id } => Ok(CanonicalTaskOwner::ConversationTurn {
            conversation_id: nomifun_common::ConversationId::parse(conversation_id).map_err(|e| DbError::Conflict(e.to_string()))?.into_string(),
            message_id: nomifun_common::MessageId::parse(message_id).map_err(|e| DbError::Conflict(e.to_string()))?.into_string(),
        }),
        CreativeTaskOwnerRef::CanvasNode {
            project_id,
            node_id,
        } => Ok(CanonicalTaskOwner::CanvasNode {
            project_id: CreativeStudioProjectId::parse(project_id)
                .map_err(|error| {
                    DbError::Conflict(format!(
                        "Creative task project_id '{project_id}' is not a canonical UUIDv7: {error}"
                    ))
                })?
                .into_string(),
            node_id: CreativeStudioNodeId::parse(node_id)
                .map_err(|error| {
                    DbError::Conflict(format!(
                        "Creative task node_id '{node_id}' is not a canonical UUIDv7: {error}"
                    ))
                })?
                .into_string(),
        }),
        CreativeTaskOwnerRef::TemplateStep {
            template_id,
            template_run_id,
            template_step_id,
        } => Ok(CanonicalTaskOwner::TemplateStep {
            template_id: CreativeStudioTemplateId::parse(template_id)
                .map_err(|error| {
                    DbError::Conflict(format!(
                        "Creative task template_id '{template_id}' is not a canonical UUIDv7: {error}"
                    ))
                })?
                .into_string(),
            template_run_id: CreativeStudioTemplateRunId::parse(template_run_id)
                .map_err(|error| {
                    DbError::Conflict(format!(
                        "Creative task template_run_id '{template_run_id}' is not a canonical UUIDv7: {error}"
                    ))
                })?
                .into_string(),
            template_step_id: CreativeStudioTemplateStepId::parse(template_step_id)
                .map_err(|error| {
                    DbError::Conflict(format!(
                        "Creative task template_step_id '{template_step_id}' is not a canonical UUIDv7: {error}"
                    ))
                })?
                .into_string(),
        }),
    }
}

fn stored_owner_matches(stored: &CreationTaskDbRow, owner: &CanonicalTaskOwner) -> bool {
    match owner {
        CanonicalTaskOwner::ConversationTurn { conversation_id, message_id } => stored.conversation_id.as_ref() == Some(conversation_id) && stored.message_id.as_ref() == Some(message_id)
            && stored.project_id.is_none() && stored.node_id.is_none() && stored.template_id.is_none(),
        CanonicalTaskOwner::CanvasNode {
            project_id,
            node_id,
        } => {
            stored.project_id.as_deref() == Some(project_id)
                && stored.node_id.as_deref() == Some(node_id)

                && stored.template_id.is_none()
                && stored.template_run_id.is_none()
                && stored.template_step_id.is_none()
        }
        CanonicalTaskOwner::TemplateStep {
            template_id,
            template_run_id,
            template_step_id,
        } => {
            stored.project_id.is_none()

                && stored.node_id.is_none()
                && stored.template_id.as_deref() == Some(template_id)
                && stored.template_run_id.as_deref() == Some(template_run_id)
                && stored.template_step_id.as_deref() == Some(template_step_id)
        }
    }
}

fn validate_idempotent_creative_task(
    stored: &CreationTaskDbRow,
    params: &CreateCreativeTaskParams<'_>,
    owner: &CanonicalTaskOwner,
    provider_id: &str,
) -> Result<(), DbError> {
    if stored.request_fingerprint.as_deref() != Some(params.request_fingerprint) {
        return Err(DbError::Conflict(format!(
            "Idempotency-Key '{}' was already used for a different creation request",
            params.creation_task_id
        )));
    }
    if !stored_owner_matches(stored, owner)
        || stored.provider_id != provider_id
        || stored.model != params.model
        || stored.capability != params.capability
        || stored.params != params.params
        || stored.input_bindings.as_deref() != Some(params.input_bindings)
    {
        return Err(DbError::Conflict(format!(
            "Idempotency-Key '{}' resolved to an inconsistent creation task",
            params.creation_task_id
        )));
    }
    Ok(())
}

/// The concrete column values written by both the unconditional and conditional
/// update paths — `params` merged over the current row (`Some` replaces, `None`
/// keeps; inner `Option` distinguishes "set NULL" from "keep").
struct MergedTaskUpdate {
    status: String,
    error: Option<String>,
    result_asset_ids: String,
    remote_task_id: Option<String>,
    attempt: i64,
    started_at: Option<i64>,
    finished_at: Option<i64>,
}

fn merge_update_fields(existing: &CreationTaskRow, params: &UpdateCreationTaskParams<'_>) -> MergedTaskUpdate {
    MergedTaskUpdate {
        status: params.status.unwrap_or(&existing.status).to_string(),
        error: match params.error {
            Some(e) => e.map(str::to_string),
            None => existing.error.clone(),
        },
        result_asset_ids: params.result_asset_ids.unwrap_or(&existing.result_asset_ids).to_string(),
        remote_task_id: match params.remote_task_id {
            Some(r) => r.map(str::to_string),
            None => existing.remote_task_id.clone(),
        },
        attempt: params.attempt.unwrap_or(existing.attempt),
        started_at: match params.started_at {
            Some(s) => s,
            None => existing.started_at,
        },
        finished_at: match params.finished_at {
            Some(f) => f,
            None => existing.finished_at,
        },
    }
}

async fn lock_creative_project(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
) -> Result<String, DbError> {
    let project_id = CreativeStudioProjectId::parse(project_id).map_err(|error| {
        DbError::Conflict(format!(
            "Creative task project_id '{project_id}' is not a canonical UUIDv7: {error}"
        ))
    })?;
    let parent = sqlx::query(
        "UPDATE creative_studio_projects SET updated_at = updated_at WHERE project_id = ?",
    )
    .bind(project_id.as_str())
    .execute(&mut **tx)
    .await?;
    if parent.rows_affected() == 0 {
        return Err(DbError::Conflict(format!(
            "Creative task project '{}' does not exist",
            project_id
        )));
    }
    Ok(project_id.into_string())
}

async fn lock_creative_template_step(
    tx: &mut Transaction<'_, Sqlite>,
    template_id: &str,
    template_run_id: &str,
    template_step_id: &str,
) -> Result<(), DbError> {
    let locked = sqlx::query(
        "UPDATE creative_studio_template_runs \
         SET updated_at = updated_at \
         WHERE template_run_id = ?1 \
           AND template_id = ?2 \
           AND status IN ('queued', 'running') \
           AND EXISTS (\
               SELECT 1 FROM json_each(step_ids_json) \
               WHERE json_each.value = ?3\
           )",
    )
    .bind(template_run_id)
    .bind(template_id)
    .bind(template_step_id)
    .execute(&mut **tx)
    .await?;
    if locked.rows_affected() == 0 {
        return Err(DbError::Conflict(format!(
            "Creative template task owner run '{template_run_id}' is missing, not executable, belongs to another template, or does not contain step '{template_step_id}'"
        )));
    }
    Ok(())
}

async fn lock_canonical_owner(
    tx: &mut Transaction<'_, Sqlite>,
    owner: &CanonicalTaskOwner,
) -> Result<(), DbError> {
    match owner {
        CanonicalTaskOwner::ConversationTurn { conversation_id, .. } => {
            let found = sqlx::query("UPDATE conversations SET updated_at=updated_at WHERE conversation_id=?")
                .bind(conversation_id).execute(&mut **tx).await?.rows_affected();
            if found != 1 { return Err(DbError::NotFound(format!("Conversation {conversation_id} not found"))); }
        }
        CanonicalTaskOwner::CanvasNode { project_id, .. } => {
            lock_creative_project(tx, project_id).await?;
        }
        CanonicalTaskOwner::TemplateStep {
            template_id,
            template_run_id,
            template_step_id,
        } => {
            lock_creative_template_step(
                tx,
                template_id,
                template_run_id,
                template_step_id,
            )
            .await?;
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CanonicalInputBinding {
    asset_id: String,
    kind: String,
    role: String,
}

fn canonicalize_input_bindings(raw: Option<&str>) -> Result<Option<String>, DbError> {
    let Some(raw) = raw else {
        // Only migration 044 may produce NULL, to identify legacy rows whose
        // complete input order/kind could not be proven.
        return Ok(None);
    };
    let bindings: Vec<CanonicalInputBinding> = serde_json::from_str(raw).map_err(|error| {
        DbError::Conflict(format!(
            "creation task input_bindings must be a JSON array of exact bindings: {error}"
        ))
    })?;
    for (index, binding) in bindings.iter().enumerate() {
        WorkshopAssetId::parse(&binding.asset_id).map_err(|error| {
            DbError::Conflict(format!(
                "creation task input_bindings[{index}].asset_id {:?} is not a canonical UUIDv7: {error}",
                binding.asset_id
            ))
        })?;
        if !matches!(binding.kind.as_str(), "image" | "video" | "audio" | "text") {
            return Err(DbError::Conflict(format!(
                "creation task input_bindings[{index}].kind {:?} is invalid",
                binding.kind
            )));
        }
        if !matches!(
            binding.role.as_str(),
            "reference" | "mask" | "first_frame" | "last_frame" | "video" | "audio"
        ) {
            return Err(DbError::Conflict(format!(
                "creation task input_bindings[{index}].role {:?} is invalid",
                binding.role
            )));
        }
    }
    serde_json::to_string(&bindings)
        .map(Some)
        .map_err(|error| DbError::Init(format!("encode creation task input_bindings: {error}")))
}

/// Canonicalize the task's JSON result asset references.
///
/// These are logical references, not SQLite foreign keys. The asset sink owns
/// the atomic asset write, while the creation service/workshop bridge owns
/// existence, ownership, and locatability audits. Keeping this repository
/// check structural avoids coupling a task state update to a second repository
/// (and permits a provisional result batch to be committed by an alternate
/// asset sink in the same service operation).
fn canonicalize_result_asset_ids(raw: &str) -> Result<String, DbError> {
    let values: Value = serde_json::from_str(raw).map_err(|error| {
        DbError::Conflict(format!(
            "creation task result_asset_ids must be valid JSON: {error}"
        ))
    })?;
    let values = values.as_array().ok_or_else(|| {
        DbError::Conflict("creation task result_asset_ids must be a JSON array".into())
    })?;
    let mut canonical = Vec::with_capacity(values.len());
    let mut seen = std::collections::HashSet::with_capacity(values.len());
    for value in values {
        let raw_id = value.as_str().ok_or_else(|| {
            DbError::Conflict(
                "creation task result_asset_ids must contain only UUIDv7 strings".into(),
            )
        })?;
        let asset_id = WorkshopAssetId::parse(raw_id).map_err(|error| {
            DbError::Conflict(format!(
                "creation task result asset '{raw_id}' is not a canonical UUIDv7: {error}"
            ))
        })?;
        if !seen.insert(asset_id.as_str().to_owned()) {
            return Err(DbError::Conflict(format!(
                "creation task result_asset_ids contains duplicate asset '{}'",
                asset_id
            )));
        }
        canonical.push(asset_id.into_string());
    }
    serde_json::to_string(&canonical)
        .map_err(|error| DbError::Init(format!("encode creation task result_asset_ids: {error}")))
}

#[async_trait::async_trait]
impl ICreationTaskRepository for SqliteCreationTaskRepository {
    async fn get_or_create_creative_task(
        &self,
        params: CreateCreativeTaskParams<'_>,
    ) -> Result<IdempotentCreationTask, DbError> {
        validate_creation_task_id(params.creation_task_id)?;
        let owner = normalize_canonical_owner(params.owner)?;
        let provider_id = ProviderId::parse(params.provider_id).map_err(|error| {
            DbError::Conflict(format!(
                "Creation task provider_id '{}' is not a canonical UUIDv7: {error}",
                params.provider_id
            ))
        })?;

        let mut tx = self.pool.begin().await?;

        // Take SQLite's writer authority on the idempotency key before looking
        // at mutable parent state. Exact replays are historical reads and must
        // remain recoverable after their project/provider is retired. A key
        // that has never existed continues below and must validate live parents.
        let existing = sqlx::query(
            "UPDATE creation_tasks SET submitted_at = submitted_at WHERE creation_task_id = ?",
        )
        .bind(params.creation_task_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if existing {
            let stored = sqlx::query_as::<_, CreationTaskDbRow>(
                "SELECT * FROM creation_tasks WHERE creation_task_id = ?",
            )
            .bind(params.creation_task_id)
            .fetch_one(&mut *tx)
            .await?;
            validate_idempotent_creative_task(
                &stored,
                &params,
                &owner,
                provider_id.as_str(),
            )?;
            let row = stored.try_into()?;
            tx.commit().await?;
            return Ok(IdempotentCreationTask {
                row,
                inserted: false,
            });
        }

        lock_canonical_owner(&mut tx, &owner).await?;
        let provider = sqlx::query("UPDATE providers SET updated_at = updated_at WHERE provider_id = ?")
            .bind(provider_id.as_str())
            .execute(&mut *tx)
            .await?;
        if provider.rows_affected() == 0 {
            return Err(DbError::Conflict(format!(
                "Creation task provider '{}' does not exist",
                provider_id
            )));
        }
        let provider_task = provider_task_for_creation_capability(params.capability).ok_or_else(|| {
            DbError::Conflict(format!(
                "Creation task capability {:?} is unsupported",
                params.capability
            ))
        })?;
        let exact_model_supports_task: bool = sqlx::query_scalar(
            "SELECT EXISTS(\
                 SELECT 1 FROM providers AS provider \
                 JOIN provider_models AS model ON model.provider_id = provider.provider_id \
                 JOIN provider_model_capabilities AS capability \
                   ON capability.provider_id = model.provider_id \
                  AND capability.model = model.model \
                 WHERE provider.provider_id = ? AND provider.enabled = 1 \
                   AND model.model = ? AND model.enabled = 1 AND capability.task = ?\
             )",
        )
        .bind(provider_id.as_str())
        .bind(params.model)
        .bind(provider_task)
        .fetch_one(&mut *tx)
        .await?;
        if !exact_model_supports_task {
            return Err(DbError::Conflict(format!(
                "Creation task model '{}/{}' does not support capability '{}'",
                provider_id, params.model, params.capability
            )));
        }

        let (project_id, template_id, template_run_id, template_step_id, node_id) = match &owner {
            CanonicalTaskOwner::ConversationTurn { .. } => (None, None, None, None, None),
            CanonicalTaskOwner::CanvasNode { project_id, node_id } => (Some(project_id.as_str()), None, None, None, Some(node_id.as_str())),
            CanonicalTaskOwner::TemplateStep { template_id, template_run_id, template_step_id } => (None, Some(template_id.as_str()), Some(template_run_id.as_str()), Some(template_step_id.as_str()), None),
        };
        // The transaction already holds the SQLite write lock. Pair this
        // check with content deletion's live-task check so neither operation
        // can race past the other and schedule work with deleted content.
        let deleted_input: Option<String> = sqlx::query_scalar(
            "SELECT asset.asset_id FROM workshop_assets asset \
             JOIN json_each(?1) input ON json_extract(input.value, '$.asset_id') = asset.asset_id \
             WHERE asset.deleted_at IS NOT NULL LIMIT 1",
        )
        .bind(params.input_bindings)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(asset_id) = deleted_input {
            return Err(DbError::Conflict(format!(
                "Creation task input asset '{asset_id}' has been permanently deleted"
            )));
        }
        let (conversation_id, message_id) = match &owner {
            CanonicalTaskOwner::ConversationTurn { conversation_id, message_id } => (Some(conversation_id.as_str()), Some(message_id.as_str())),
            _ => (None, None),
        };
        if let (Some(conversation_id), Some(message_id)) = (conversation_id, message_id) {
            let request: Value = serde_json::from_str(params.params).map_err(|e| DbError::Conflict(e.to_string()))?;
            let content = serde_json::json!({"content": request.get("prompt").and_then(Value::as_str).unwrap_or_default(), "creation": {"agent": request.get("_nomifun_creation_agent"), "creation_task_id": params.creation_task_id}}).to_string();
            // A professional submission owns its new user message. Tool calls
            // and batch siblings attach to that existing turn without replacing
            // its original prompt or Agent snapshot.
            if message_id == params.creation_task_id {
                sqlx::query("INSERT INTO messages (message_id,conversation_id,msg_id,type,content,position,status,hidden,created_at) VALUES (?, ?, ?, 'text', ?, 'right', 'finish', 0, ?) ON CONFLICT(message_id) DO NOTHING")
                    .bind(message_id).bind(conversation_id).bind(message_id).bind(&content).bind(params.submitted_at).execute(&mut *tx).await?;
            }
            let actual: Option<(String,String,String)> = sqlx::query_as("SELECT conversation_id,content,position FROM messages WHERE message_id=?")
                .bind(message_id).fetch_optional(&mut *tx).await?;
            if actual.is_none_or(|actual| actual.0 != conversation_id || actual.2 != "right" || (message_id == params.creation_task_id && actual.1 != content)) {
                return Err(DbError::Conflict("Generation must belong to an existing user turn in this conversation".into()));
            }
            sqlx::query("UPDATE conversations SET updated_at=MAX(updated_at,?) WHERE conversation_id=?").bind(params.submitted_at).bind(conversation_id).execute(&mut *tx).await?;
        }
        let inserted = sqlx::query(
            "INSERT INTO creation_tasks \
                (creation_task_id, project_id, template_id, template_run_id, template_step_id, \
                 node_id, provider_id, model, capability, \
                 params, input_bindings, status, error, result_asset_ids, remote_task_id, attempt, submitted_at, \
                 started_at, finished_at, request_fingerprint, conversation_id, message_id) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, '[]', NULL, 0, ?, NULL, NULL, ?, ?, ?) \
             ON CONFLICT(creation_task_id) DO NOTHING",
        )
        .bind(params.creation_task_id)
        .bind(project_id)
        .bind(template_id)
        .bind(template_run_id)
        .bind(template_step_id)
        .bind(node_id)
        .bind(provider_id.as_str())
        .bind(params.model)
        .bind(params.capability)
        .bind(params.params)
        .bind(params.input_bindings)
        .bind(params.status)
        .bind(params.submitted_at)
        .bind(params.request_fingerprint)
        .bind(conversation_id)
        .bind(message_id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;

        let stored = sqlx::query_as::<_, CreationTaskDbRow>(
            "SELECT * FROM creation_tasks WHERE creation_task_id = ?",
        )
        .bind(params.creation_task_id)
        .fetch_one(&mut *tx)
        .await?;

        validate_idempotent_creative_task(
            &stored,
            &params,
            &owner,
            provider_id.as_str(),
        )?;

        let row = stored.try_into()?;
        tx.commit().await?;
        Ok(IdempotentCreationTask { row, inserted })
    }

    async fn get_task(
        &self,
        creation_task_id: &str,
    ) -> Result<Option<CreationTaskRow>, DbError> {
        validate_creation_task_id(creation_task_id)?;
        let row = sqlx::query_as::<_, CreationTaskDbRow>(
            "SELECT * FROM creation_tasks WHERE creation_task_id = ?",
        )
            .bind(creation_task_id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(TryInto::try_into).transpose()
    }



    async fn list_conversation_tasks(&self, conversation_id: &str) -> Result<Vec<CreationTaskRow>, DbError> {
        let rows = sqlx::query_as::<_, CreationTaskDbRow>("SELECT * FROM creation_tasks WHERE conversation_id=? AND deleted_at IS NULL ORDER BY submitted_at ASC, creation_task_id ASC")
            .bind(conversation_id).fetch_all(&self.pool).await?;
        rows.into_iter().map(CreationTaskRow::try_from).collect()
    }

    async fn conversation_message_creation_references(
        &self,
        conversation_id: &str,
        message_id: &str,
    ) -> Result<Option<Value>, DbError> {
        let content: Option<String> = sqlx::query_scalar(
            "SELECT content FROM messages WHERE conversation_id = ? AND message_id = ? AND position = 'right' AND type = 'text'",
        )
        .bind(conversation_id)
        .bind(message_id)
        .fetch_optional(&self.pool)
        .await?;
        content
            .map(|raw| serde_json::from_str::<Value>(&raw))
            .transpose()
            .map(|content| content.and_then(|content| content.get("creation_references").cloned()))
            .map_err(|error| DbError::Init(format!("Invalid user message creation metadata: {error}")))
    }

    async fn list_all_tasks(&self) -> Result<Vec<CreationTaskRow>, DbError> {
        sqlx::query_as::<_, CreationTaskDbRow>(
            "SELECT * FROM creation_tasks ORDER BY submitted_at ASC, creation_task_id ASC",
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn update_task(
        &self,
        creation_task_id: &str,
        params: UpdateCreationTaskParams<'_>,
    ) -> Result<CreationTaskRow, DbError> {
        validate_creation_task_id(creation_task_id)?;
        let mut tx = self.pool.begin().await?;
        let existing = sqlx::query_as::<_, CreationTaskDbRow>(
            "SELECT * FROM creation_tasks WHERE creation_task_id = ?",
        )
            .bind(creation_task_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| {
                DbError::NotFound(format!("creation task '{creation_task_id}' not found"))
            })?
            .try_into()?;

        let mut m = merge_update_fields(&existing, &params);
        m.result_asset_ids = canonicalize_result_asset_ids(&m.result_asset_ids)?;

        let result = sqlx::query(
            "UPDATE creation_tasks SET status = ?, error = ?, result_asset_ids = ?, remote_task_id = ?, \
             attempt = ?, started_at = ?, finished_at = ? WHERE creation_task_id = ?",
        )
        .bind(&m.status)
        .bind(&m.error)
        .bind(&m.result_asset_ids)
        .bind(&m.remote_task_id)
        .bind(m.attempt)
        .bind(m.started_at)
        .bind(m.finished_at)
        .bind(creation_task_id)
        .execute(&mut *tx)
        .await.map_err(DbError::from_asset_reference_guard)?;
        if result.rows_affected() != 1 {
            return Err(DbError::NotFound(format!(
                "creation task '{creation_task_id}' not found"
            )));
        }
        tx.commit().await?;

        Ok(CreationTaskRow {
            status: m.status,
            error: m.error,
            result_asset_ids: m.result_asset_ids,
            remote_task_id: m.remote_task_id,
            attempt: m.attempt,
            started_at: m.started_at,
            finished_at: m.finished_at,
            ..existing
        })
    }

    async fn update_task_if_live(
        &self,
        creation_task_id: &str,
        params: UpdateCreationTaskParams<'_>,
    ) -> Result<bool, DbError> {
        validate_creation_task_id(creation_task_id)?;
        let mut tx = self.pool.begin().await?;
        let Some(existing) = sqlx::query_as::<_, CreationTaskDbRow>(
            "SELECT * FROM creation_tasks WHERE creation_task_id = ?",
        )
        .bind(creation_task_id)
        .fetch_optional(&mut *tx)
        .await?
        else {
            return Ok(false); // unknown id → treat as "not live"
        };
        let existing: CreationTaskRow = existing.try_into()?;
        let mut m = merge_update_fields(&existing, &params);
        m.result_asset_ids = canonicalize_result_asset_ids(&m.result_asset_ids)?;

        // The `WHERE ... status IN ('queued','running')` predicate is the
        // compare-and-set: if a concurrent cancel wrote a terminal status
        // between our read and this write, zero rows match and we do not
        // overwrite it.
        let res = sqlx::query(
            "UPDATE creation_tasks SET status = ?, error = ?, result_asset_ids = ?, remote_task_id = ?, \
             attempt = ?, started_at = ?, finished_at = ? \
             WHERE creation_task_id = ? AND status IN ('queued', 'running')",
        )
        .bind(&m.status)
        .bind(&m.error)
        .bind(&m.result_asset_ids)
        .bind(&m.remote_task_id)
        .bind(m.attempt)
        .bind(m.started_at)
        .bind(m.finished_at)
        .bind(creation_task_id)
        .execute(&mut *tx)
        .await.map_err(DbError::from_asset_reference_guard)?;
        tx.commit().await?;
        Ok(res.rows_affected() > 0)
    }

    async fn set_remote_task_id_if_live(
        &self,
        creation_task_id: &str,
        remote_task_id: &str,
    ) -> Result<bool, DbError> {
        validate_creation_task_id(creation_task_id)?;
        let result = sqlx::query(
            "UPDATE creation_tasks SET remote_task_id = ? \
             WHERE creation_task_id = ? AND status IN ('queued', 'running')",
        )
        .bind(remote_task_id)
        .bind(creation_task_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_live_tasks(&self) -> Result<Vec<CreationTaskRow>, DbError> {
        let rows = sqlx::query_as::<_, CreationTaskDbRow>(
            "SELECT * FROM creation_tasks \
             WHERE status IN ('queued', 'running') AND deleted_at IS NULL \
             ORDER BY submitted_at ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(TryInto::try_into).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;
    use crate::repository::{
        CoordinatedProviderModelDelete, IProviderModelRepository, ProviderModelCleanupPlan,
        SqliteProviderModelRepository,
    };
    use nomifun_common::{WorkshopAssetId, generate_id};
    use std::sync::Arc;

    async fn repo() -> (SqliteCreationTaskRepository, crate::Database, String) {
        let db = init_database_memory().await.unwrap();
        let provider_id = ProviderId::new().into_string();
        sqlx::query(
            "INSERT INTO providers \
                (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, enabled, \
                 created_at, updated_at) \
             VALUES (?, 'openai', 'Creation Test Provider', \
                 'https://example.invalid', 'bearer', '', 1, 0, 0)",
        )
        .bind(&provider_id)
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO provider_models \
                (provider_id, model, enabled, sort_order, description, created_at, updated_at) \
             VALUES (?, 'image-model-v1', 1, 0, NULL, 0, 0)",
        )
        .bind(&provider_id)
        .execute(db.pool())
        .await
        .unwrap();
        for (task, protocol) in [
            ("image_generation", "openai.images"),
            ("video_generation", "openai.videos"),
        ] {
            sqlx::query(
                "INSERT INTO provider_model_capabilities \
                    (provider_id, model, task, traits, protocol, connection_role, \
                     provider_params, created_at, updated_at) \
                 VALUES (?, 'image-model-v1', ?, '[]', ?, 'default', '{}', 0, 0)",
            )
            .bind(&provider_id)
            .bind(task)
            .bind(protocol)
            .execute(db.pool())
            .await
            .unwrap();
        }
        let repo = SqliteCreationTaskRepository::new(db.pool().clone());
        (repo, db, provider_id)
    }

    async fn seed_creative_project(db: &crate::Database) -> String {
        let project_id = CreativeStudioProjectId::new().into_string();
        let document = serde_json::json!({
            "schema": "nomifun.creative-studio/v1",
            "projectId": project_id,
            "nodes": []
        });
        sqlx::query(
            "INSERT INTO creative_studio_projects \
                (project_id, title, revision, node_count, connection_count, document_json, created_at, updated_at) \
             VALUES (?, 'Idempotency Test', 1, 0, 0, ?, 0, 0)",
        )
        .bind(&project_id)
        .bind(document.to_string())
        .execute(db.pool())
        .await
        .unwrap();
        project_id
    }

    async fn seed_creative_template_run(
        db: &crate::Database,
    ) -> (String, String, String) {
        let template_id = CreativeStudioTemplateId::new().into_string();
        let template_run_id = CreativeStudioTemplateRunId::new().into_string();
        let template_step_id = CreativeStudioTemplateStepId::new().into_string();
        let definition = serde_json::json!({
            "id": template_id,
            "revision": 1
        });
        sqlx::query(
            "INSERT INTO creative_studio_templates \
                (template_id, revision, name, description, category, visibility, definition_json, \
                 created_at, updated_at) \
             VALUES (?, 1, 'Task Owner Test', '', '', 'private', ?, 0, 0)",
        )
        .bind(&template_id)
        .bind(definition.to_string())
        .execute(db.pool())
        .await
        .unwrap();
        let aggregate = serde_json::json!({
            "kind": "nomifun.creative-studio.template-run",
            "version": 1,
            "revision": 1,
            "templateSnapshot": { "id": template_id, "revision": 1 },
            "request": {
                "id": template_run_id,
                "templateId": template_id,
                "templateRevision": 1
            },
            "record": {
                "requestId": template_run_id,
                "templateId": template_id,
                "status": "queued"
            }
        });
        sqlx::query(
            "INSERT INTO creative_studio_template_runs \
                (template_run_id, template_id, template_revision, revision, status, step_ids_json, \
                 aggregate_json, created_at, updated_at) \
             VALUES (?, ?, 1, 1, 'queued', ?, ?, 0, 0)",
        )
        .bind(&template_run_id)
        .bind(&template_id)
        .bind(serde_json::to_string(&[&template_step_id]).unwrap())
        .bind(aggregate.to_string())
        .execute(db.pool())
        .await
        .unwrap();
        (template_id, template_run_id, template_step_id)
    }

    fn creative_params<'a>(
        creation_task_id: &'a str,
        project_id: &'a str,
        node_id: &'a str,
        provider_id: &'a str,
        fingerprint: &'a str,
    ) -> CreateCreativeTaskParams<'a> {
        CreateCreativeTaskParams {
            creation_task_id,
            owner: CreativeTaskOwnerRef::CanvasNode {
                project_id,
                node_id,
            },
            provider_id,
            model: "image-model-v1",
            capability: "t2i",
            params: r#"{"prompt":"Aurora"}"#,
            input_bindings: "[]",
            request_fingerprint: fingerprint,
            status: "queued",
            submitted_at: 100,
        }
    }

    async fn create_project_task(
        repo: &SqliteCreationTaskRepository,
        db: &crate::Database,
        creation_task_id: &str,
        provider_id: &str,
    ) -> CreationTaskRow {
        let project_id = seed_creative_project(db).await;
        let node_id = CreativeStudioNodeId::new().into_string();
        let fingerprint = serde_json::json!({"test_task_id": creation_task_id}).to_string();
        repo.get_or_create_creative_task(creative_params(
            creation_task_id,
            &project_id,
            &node_id,
            provider_id,
            &fingerprint,
        ))
        .await
        .unwrap()
        .row
    }

    fn template_creative_params<'a>(
        creation_task_id: &'a str,
        template_id: &'a str,
        template_run_id: &'a str,
        template_step_id: &'a str,
        provider_id: &'a str,
        fingerprint: &'a str,
    ) -> CreateCreativeTaskParams<'a> {
        CreateCreativeTaskParams {
            creation_task_id,
            owner: CreativeTaskOwnerRef::TemplateStep {
                template_id,
                template_run_id,
                template_step_id,
            },
            provider_id,
            model: "image-model-v1",
            capability: "t2i",
            params: r#"{"prompt":"Aurora"}"#,
            input_bindings: "[]",
            request_fingerprint: fingerprint,
            status: "queued",
            submitted_at: 100,
        }
    }

    fn canvas_reference_params<'a>(
        creation_task_id: &'a str,
        project_id: &'a str,
        node_id: &'a str,
        provider_id: &'a str,
        input_bindings: &'a str,
        fingerprint: &'a str,
    ) -> CreateCreativeTaskParams<'a> {
        CreateCreativeTaskParams {
            creation_task_id,
            owner: CreativeTaskOwnerRef::CanvasNode { project_id, node_id },
            provider_id,
            model: "image-model-v1",
            capability: "i2v",
            params: r#"{"prompt":"Aurora"}"#,
            input_bindings,
            request_fingerprint: fingerprint,
            status: "queued",
            submitted_at: 100,
        }
    }

    async fn raw_insert_task_ownership(
        db: &crate::Database,
        creation_task_id: &str,
        project_id: Option<&str>,
        template_id: Option<&str>,
        node_id: Option<&str>,
        provider_id: &str,
        request_fingerprint: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO creation_tasks \
                (creation_task_id, project_id, template_id, node_id, provider_id, model, capability, \
                 params, status, submitted_at, request_fingerprint) \
             VALUES (?, ?, ?, ?, ?, 'image-model-v1', 't2i', '{}', 'queued', 100, ?)",
        )
        .bind(creation_task_id)
        .bind(project_id)
        .bind(template_id)
        .bind(node_id)
        .bind(provider_id)
        .bind(request_fingerprint)
        .execute(db.pool())
        .await
        .map(|_| ())
    }

    #[tokio::test]
    async fn schema_rejects_mixed_or_incomplete_canonical_task_ownership() {
        let (_repo, db, provider_id) = repo().await;
        let project_id = seed_creative_project(&db).await;
        let node_id = CreativeStudioNodeId::new().into_string();
        let template_id = CreativeStudioTemplateId::new().into_string();

        let mixed = raw_insert_task_ownership(
            &db,
            &CreationTaskId::new().into_string(),
            Some(&project_id),
            Some(&template_id),
            Some(&node_id),
            &provider_id,
            Some(r#"{"project_id":"mixed"}"#),
        )
        .await;
        assert!(mixed.is_err(), "project and template owners must be exclusive");

        let missing_fingerprint = raw_insert_task_ownership(
            &db,
            &CreationTaskId::new().into_string(),
            Some(&project_id),
            None,
            Some(&node_id),
            &provider_id,
            None,
        )
        .await;
        assert!(
            missing_fingerprint.is_err(),
            "canonical project ownership requires a durable request fingerprint"
        );

        let orphan_fingerprint = raw_insert_task_ownership(
            &db,
            &CreationTaskId::new().into_string(),
            None,
            None,
            None,
            &provider_id,
            Some(r#"{"project_id":"missing"}"#),
        )
        .await;
        assert!(
            orphan_fingerprint.is_err(),
            "ownerless task rows are retired"
        );
    }

    #[tokio::test]
    async fn creative_project_idempotency_reuses_exact_request_without_reopening_terminal_state() {
        let (repo, db, provider_id) = repo().await;
        let project_id = seed_creative_project(&db).await;
        let node_id = CreativeStudioNodeId::new().into_string();
        let task_id = CreationTaskId::new().into_string();
        let fingerprint = r#"{"project_id":"p","inputs":[]}"#;

        let first = repo
            .get_or_create_creative_task(creative_params(
                &task_id,
                &project_id,
                &node_id,
                &provider_id,
                fingerprint,
            ))
            .await
            .unwrap();
        assert!(first.inserted);
        assert_eq!(first.row.project_id.as_deref(), Some(project_id.as_str()));

        repo.update_task(
            &task_id,
            UpdateCreationTaskParams {
                status: Some("canceled"),
                finished_at: Some(Some(200)),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let retry = repo
            .get_or_create_creative_task(creative_params(
                &task_id,
                &project_id,
                &node_id,
                &provider_id,
                fingerprint,
            ))
            .await
            .unwrap();
        assert!(!retry.inserted);
        assert_eq!(retry.row.status, "canceled");
        assert_eq!(retry.row.finished_at, Some(200));

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM creation_tasks WHERE creation_task_id = ?",
        )
        .bind(&task_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(count, 1);
    }




    #[tokio::test]
    async fn exact_retry_survives_parent_removal_but_a_new_key_still_requires_a_live_project() {
        let (repo, db, provider_id) = repo().await;
        let project_id = seed_creative_project(&db).await;
        let node_id = CreativeStudioNodeId::new().into_string();
        let task_id = CreationTaskId::new().into_string();
        let fingerprint = r#"{"project_id":"historical"}"#;

        let first = repo
            .get_or_create_creative_task(creative_params(
                &task_id,
                &project_id,
                &node_id,
                &provider_id,
                fingerprint,
            ))
            .await
            .unwrap();
        assert!(first.inserted);
        sqlx::query("DELETE FROM creative_studio_projects WHERE project_id = ?")
            .bind(&project_id)
            .execute(db.pool())
            .await
            .unwrap();

        let historical_retry = repo
            .get_or_create_creative_task(creative_params(
                &task_id,
                &project_id,
                &node_id,
                &provider_id,
                fingerprint,
            ))
            .await
            .unwrap();
        assert!(!historical_retry.inserted);
        assert_eq!(historical_retry.row.creation_task_id, task_id);

        let new_key = CreationTaskId::new().into_string();
        let new_submission = repo
            .get_or_create_creative_task(creative_params(
                &new_key,
                &project_id,
                &node_id,
                &provider_id,
                r#"{"project_id":"new"}"#,
            ))
            .await
            .unwrap_err();
        assert!(matches!(
            new_submission,
            DbError::Conflict(message) if message.contains("does not exist")
        ));
    }

    #[tokio::test]
    async fn exact_replay_survives_model_delete_but_a_new_key_requires_the_live_capability() {
        let (repo, db, provider_id) = repo().await;
        let project_id = seed_creative_project(&db).await;
        let node_id = CreativeStudioNodeId::new().into_string();
        let task_id = CreationTaskId::new().into_string();
        let fingerprint = r#"{"project_id":"model-history"}"#;
        let first = repo
            .get_or_create_creative_task(creative_params(
                &task_id,
                &project_id,
                &node_id,
                &provider_id,
                fingerprint,
            ))
            .await
            .unwrap();
        assert!(first.inserted);
        repo.update_task(
            &task_id,
            UpdateCreationTaskParams {
                status: Some("canceled"),
                finished_at: Some(Some(200)),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert!(
            SqliteProviderModelRepository::new(db.pool().clone())
                .delete_coordinated(&CoordinatedProviderModelDelete {
                    provider_id: provider_id.clone(),
                    model: "image-model-v1".to_owned(),
                    expected_config_revision: 0,
                    cleanup: ProviderModelCleanupPlan::default(),
                })
                .await
                .unwrap()
        );

        let replay = repo
            .get_or_create_creative_task(creative_params(
                &task_id,
                &project_id,
                &node_id,
                &provider_id,
                fingerprint,
            ))
            .await
            .unwrap();
        assert!(!replay.inserted);
        assert_eq!(replay.row.status, "canceled");

        let new_task_id = CreationTaskId::new().into_string();
        let error = repo
            .get_or_create_creative_task(creative_params(
                &new_task_id,
                &project_id,
                &node_id,
                &provider_id,
                r#"{"project_id":"new-after-delete"}"#,
            ))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            DbError::Conflict(message) if message.contains("does not support capability")
        ));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM creation_tasks")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn new_task_requires_enabled_provider_model_and_exact_capability() {
        let (repo, db, provider_id) = repo().await;
        let project_id = seed_creative_project(&db).await;
        let node_id = CreativeStudioNodeId::new().into_string();

        sqlx::query(
            "DELETE FROM provider_model_capabilities \
             WHERE provider_id = ? AND model = 'image-model-v1' AND task = 'image_generation'",
        )
        .bind(&provider_id)
        .execute(db.pool())
        .await
        .unwrap();
        let missing_capability_id = CreationTaskId::new().into_string();
        let error = repo
            .get_or_create_creative_task(creative_params(
                &missing_capability_id,
                &project_id,
                &node_id,
                &provider_id,
                r#"{"gate":"capability"}"#,
            ))
            .await
            .unwrap_err();
        assert!(matches!(error, DbError::Conflict(message) if message.contains("does not support")));

        sqlx::query(
            "INSERT INTO provider_model_capabilities \
                (provider_id, model, task, traits, protocol, connection_role, provider_params, \
                 created_at, updated_at) \
             VALUES (?, 'image-model-v1', 'image_generation', '[]', 'openai.images', \
                     'default', '{}', 0, 0)",
        )
        .bind(&provider_id)
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "UPDATE provider_models SET enabled = 0 \
             WHERE provider_id = ? AND model = 'image-model-v1'",
        )
        .bind(&provider_id)
        .execute(db.pool())
        .await
        .unwrap();
        let disabled_model_id = CreationTaskId::new().into_string();
        let error = repo
            .get_or_create_creative_task(creative_params(
                &disabled_model_id,
                &project_id,
                &node_id,
                &provider_id,
                r#"{"gate":"model"}"#,
            ))
            .await
            .unwrap_err();
        assert!(matches!(error, DbError::Conflict(message) if message.contains("does not support")));

        sqlx::query(
            "UPDATE provider_models SET enabled = 1 \
             WHERE provider_id = ? AND model = 'image-model-v1'",
        )
        .bind(&provider_id)
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query("UPDATE providers SET enabled = 0 WHERE provider_id = ?")
            .bind(&provider_id)
            .execute(db.pool())
            .await
            .unwrap();
        let disabled_provider_id = CreationTaskId::new().into_string();
        let error = repo
            .get_or_create_creative_task(creative_params(
                &disabled_provider_id,
                &project_id,
                &node_id,
                &provider_id,
                r#"{"gate":"provider"}"#,
            ))
            .await
            .unwrap_err();
        assert!(matches!(error, DbError::Conflict(message) if message.contains("does not support")));

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM creation_tasks")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn creative_project_idempotency_rejects_key_reuse_for_another_request() {
        let (repo, db, provider_id) = repo().await;
        let project_id = seed_creative_project(&db).await;
        let node_id = CreativeStudioNodeId::new().into_string();
        let task_id = CreationTaskId::new().into_string();
        repo.get_or_create_creative_task(creative_params(
            &task_id,
            &project_id,
            &node_id,
            &provider_id,
            r#"{"prompt":"first"}"#,
        ))
        .await
        .unwrap();

        let error = repo
            .get_or_create_creative_task(creative_params(
                &task_id,
                &project_id,
                &node_id,
                &provider_id,
                r#"{"prompt":"different"}"#,
            ))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            DbError::Conflict(message) if message.contains("different creation request")
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_creative_project_retries_have_one_insert_authority() {
        let (repo, db, provider_id) = repo().await;
        let project_id = seed_creative_project(&db).await;
        let node_id = CreativeStudioNodeId::new().into_string();
        let task_id = CreationTaskId::new().into_string();
        let repo = Arc::new(repo);
        let mut retries = Vec::new();
        for _ in 0..8 {
            let repo = repo.clone();
            let task_id = task_id.clone();
            let project_id = project_id.clone();
            let node_id = node_id.clone();
            let provider_id = provider_id.clone();
            retries.push(tokio::spawn(async move {
                repo.get_or_create_creative_task(creative_params(
                    &task_id,
                    &project_id,
                    &node_id,
                    &provider_id,
                    r#"{"same":true}"#,
                ))
                .await
                .unwrap()
                .inserted
            }));
        }
        let mut insert_authorities = 0;
        for retry in retries {
            insert_authorities += usize::from(retry.await.unwrap());
        }
        assert_eq!(insert_authorities, 1);

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM creation_tasks WHERE creation_task_id = ?",
        )
        .bind(&task_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn template_step_owner_requires_an_executable_run_and_exact_step() {
        let (repo, db, provider_id) = repo().await;
        let (template_id, template_run_id, template_step_id) =
            seed_creative_template_run(&db).await;
        sqlx::query(
            "UPDATE creative_studio_template_runs SET status = 'running', \
             aggregate_json = json_set(aggregate_json, '$.record.status', 'running') \
             WHERE template_run_id = ?",
        )
        .bind(&template_run_id)
        .execute(db.pool())
        .await
        .unwrap();
        let task_id = CreationTaskId::new().into_string();
        let fingerprint = r#"{"owner":{"kind":"template_step"}}"#;

        let first = repo
            .get_or_create_creative_task(template_creative_params(
                &task_id,
                &template_id,
                &template_run_id,
                &template_step_id,
                &provider_id,
                fingerprint,
            ))
            .await
            .unwrap();
        assert!(first.inserted);
        assert_eq!(first.row.template_id.as_deref(), Some(template_id.as_str()));
        assert_eq!(
            first.row.template_run_id.as_deref(),
            Some(template_run_id.as_str())
        );
        assert_eq!(
            first.row.template_step_id.as_deref(),
            Some(template_step_id.as_str())
        );
        assert!(first.row.project_id.is_none());
        assert!(first.row.node_id.is_none());

        let replay = repo
            .get_or_create_creative_task(template_creative_params(
                &task_id,
                &template_id,
                &template_run_id,
                &template_step_id,
                &provider_id,
                fingerprint,
            ))
            .await
            .unwrap();
        assert!(!replay.inserted);

        let missing_step = CreativeStudioTemplateStepId::new().into_string();
        let error = repo
            .get_or_create_creative_task(template_creative_params(
                &CreationTaskId::new().into_string(),
                &template_id,
                &template_run_id,
                &missing_step,
                &provider_id,
                r#"{"owner":{"kind":"template_step","attempt":2}}"#,
            ))
            .await
            .unwrap_err();
        assert!(matches!(error, DbError::Conflict(message) if message.contains("does not contain step")));

        sqlx::query(
            "UPDATE creative_studio_template_runs SET status = 'succeeded', \
             aggregate_json = json_set(aggregate_json, '$.record.status', 'succeeded') \
             WHERE template_run_id = ?",
        )
        .bind(&template_run_id)
        .execute(db.pool())
        .await
        .unwrap();
        let terminal_error = repo
            .get_or_create_creative_task(template_creative_params(
                &CreationTaskId::new().into_string(),
                &template_id,
                &template_run_id,
                &template_step_id,
                &provider_id,
                r#"{"owner":{"kind":"template_step","attempt":3}}"#,
            ))
            .await
            .unwrap_err();
        assert!(matches!(terminal_error, DbError::Conflict(message) if message.contains("not executable")));
    }

    #[tokio::test]
    async fn create_get_and_update_flow() {
        let (repo, db, provider_id) = repo().await;
        let creation_task_id = generate_id();
        let t = create_project_task(&repo, &db, &creation_task_id, &provider_id).await;
        assert_eq!(t.creation_task_id, creation_task_id);
        assert_eq!(t.status, "queued");
        assert_eq!(t.result_asset_ids, "[]");
        assert_eq!(t.attempt, 0);

        // M0 shape: immediately fail with adapter_unavailable.
        let failed = repo
            .update_task(
                &creation_task_id,
                UpdateCreationTaskParams {
                    status: Some("failed"),
                    error: Some(Some(r#"{"kind":"adapter_unavailable","message":"no adapter"}"#)),
                    finished_at: Some(Some(200)),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.finished_at, Some(200));
        assert!(failed.error.as_deref().unwrap().contains("adapter_unavailable"));
        // unchanged fields preserved
        assert_eq!(failed.model, "image-model-v1");
        assert_eq!(failed.capability, "t2i");

        let missing_id = generate_id();
        assert!(matches!(
            repo.update_task(&missing_id, UpdateCreationTaskParams::default()).await.unwrap_err(),
            DbError::NotFound(_)
        ));
    }

    #[test]
    fn creation_task_business_id_rejects_non_uuidv7_boundaries() {
        for invalid in [
            "1",
            "task_0190f5fe-7c00-7a00-8000-000000000001",
            "0190f5fe-7c00-4a00-8000-000000000001",
            "0190F5FE-7C00-7A00-8000-000000000001",
            "0190f5fe7c007a008000000000000001",
            "0190f5fe-7c00-7a00-8000-000000000001 ",
        ] {
            assert!(validate_uuidv7(invalid).is_err());
            assert!(matches!(
                validate_creation_task_id(invalid),
                Err(DbError::Conflict(message)) if message.contains("canonical UUIDv7")
            ));
        }
        validate_creation_task_id("0190f5fe-7c00-7a00-8000-000000000001").unwrap();
    }

    #[tokio::test]
    async fn deleted_asset_guards_keep_terminal_history_and_reject_new_work() {
        use crate::repository::{IWorkshopRepository, SqliteWorkshopRepository};
        let (repo, db, provider_id) = repo().await;
        let project_id = seed_creative_project(&db).await;
        let node_id = CreativeStudioNodeId::new().into_string();
        let workshop = SqliteWorkshopRepository::new(db.pool().clone());
        let asset_id = WorkshopAssetId::new().into_string();
        sqlx::query("INSERT INTO workshop_assets (asset_id, kind, title, tags, in_library, created_at, updated_at) VALUES (?, 'image', 'input', '[]', 1, 1, 1)")
            .bind(&asset_id).execute(db.pool()).await.unwrap();
        let bindings = serde_json::json!([{"asset_id": asset_id, "kind": "image", "role": "reference"}]).to_string();
        let task_id = CreationTaskId::new().into_string();
        repo.get_or_create_creative_task(canvas_reference_params(
            &task_id, &project_id, &node_id, &provider_id, &bindings, r#"{"deletion-test":1}"#,
        )).await.unwrap();
        let result_asset_id = WorkshopAssetId::new().into_string();
        sqlx::query("INSERT INTO workshop_assets (asset_id, kind, title, tags, in_library, created_at, updated_at) VALUES (?, 'video', 'result', '[]', 1, 1, 1)")
            .bind(&result_asset_id).execute(db.pool()).await.unwrap();
        let results = serde_json::json!([result_asset_id]).to_string();
        repo.update_task(&task_id, UpdateCreationTaskParams {
            result_asset_ids: Some(&results), ..Default::default()
        }).await.unwrap();
        assert!(matches!(workshop.mark_asset_content_deleted(&asset_id, 200).await, Err(DbError::Conflict(_))));
        assert!(matches!(workshop.mark_asset_content_deleted(&result_asset_id, 200).await, Err(DbError::Conflict(_))));
        repo.update_task(&task_id, UpdateCreationTaskParams {
            status: Some("succeeded"), finished_at: Some(Some(200)), ..Default::default()
        }).await.unwrap();
        workshop.mark_asset_content_deleted(&asset_id, 300).await.unwrap();
        workshop.finish_asset_content_deletion(&asset_id, 300).await.unwrap();
        workshop.mark_asset_content_deleted(&result_asset_id, 300).await.unwrap();
        workshop.finish_asset_content_deletion(&result_asset_id, 300).await.unwrap();
        let historical = repo.update_task(&task_id, UpdateCreationTaskParams {
            remote_task_id: Some(Some("remote-history")), ..Default::default()
        }).await.unwrap();
        assert_eq!(historical.input_bindings.as_deref(), Some(bindings.as_str()));
        assert_eq!(historical.result_asset_ids, results);
        let different_results = serde_json::json!([asset_id]).to_string();
        assert!(matches!(repo.update_task(&task_id, UpdateCreationTaskParams {
            result_asset_ids: Some(&different_results), ..Default::default()
        }).await, Err(DbError::Conflict(_))));
        assert!(repo.update_task(&task_id, UpdateCreationTaskParams {
            status: Some("queued"), ..Default::default()
        }).await.is_err());
        let new_id = CreationTaskId::new().into_string();
        let new_task = repo.get_or_create_creative_task(canvas_reference_params(
            &new_id, &project_id, &node_id, &provider_id, &bindings, r#"{"deletion-test":2}"#,
        )).await;
        assert!(matches!(new_task, Err(DbError::Conflict(message)) if message.contains("permanently deleted")));
        assert!(repo.get_task(&new_id).await.unwrap().is_none());
        assert!(sqlx::query("INSERT INTO creation_tasks (creation_task_id, project_id, node_id, provider_id, model, capability, params, input_bindings, status, submitted_at, request_fingerprint) VALUES (?, ?, ?, ?, 'image-model-v1', 'i2v', '{}', ?, 'queued', 400, '{}')")
            .bind(&new_id).bind(&project_id).bind(&node_id).bind(&provider_id).bind(&bindings).execute(db.pool()).await.is_err());
    }

    #[tokio::test]
    async fn deleted_asset_and_creation_task_submission_serialize() {
        use crate::repository::{IWorkshopRepository, SqliteWorkshopRepository};
        let (repo, db, provider_id) = repo().await;
        let project_id = seed_creative_project(&db).await;
        let node_id = CreativeStudioNodeId::new().into_string();
        let workshop = SqliteWorkshopRepository::new(db.pool().clone());
        let asset_id = WorkshopAssetId::new().into_string();
        sqlx::query("INSERT INTO workshop_assets (asset_id, kind, title, tags, in_library, created_at, updated_at) VALUES (?, 'image', 'input', '[]', 1, 1, 1)")
            .bind(&asset_id).execute(db.pool()).await.unwrap();
        let bindings = serde_json::json!([{"asset_id": asset_id, "kind": "image", "role": "reference"}]).to_string();
        let task_id = CreationTaskId::new().into_string();
        let (deleted, submitted) = tokio::join!(
            workshop.mark_asset_content_deleted(&asset_id, 200),
            repo.get_or_create_creative_task(canvas_reference_params(
                &task_id, &project_id, &node_id, &provider_id, &bindings, r#"{"deletion-race":1}"#,
            ))
        );
        assert_ne!(deleted.is_ok(), submitted.is_ok());
        if deleted.is_ok() {
            assert!(repo.get_task(&task_id).await.unwrap().is_none());
        } else {
            assert!(workshop.get_asset(&asset_id).await.unwrap().unwrap().deleted_at.is_none());
        }
    }

    #[tokio::test]
    async fn result_asset_ids_are_structural_logical_references() {
        let (repo, db, provider_id) = repo().await;
        let creation_task_id = generate_id();
        create_project_task(&repo, &db, &creation_task_id, &provider_id).await;
        let asset_id = WorkshopAssetId::new().into_string();
        let ids_json = serde_json::to_string(&[asset_id.as_str()]).unwrap();

        // The task repository canonicalizes the JSON/UUIDv7 shape but does not
        // emulate a physical FK into workshop_assets. Existence, task ownership,
        // and file locatability are audited by CreationService + AssetSink.
        let updated = repo
            .update_task(
                &creation_task_id,
                UpdateCreationTaskParams {
                    result_asset_ids: Some(&ids_json),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&updated.result_asset_ids).unwrap(),
            vec![asset_id.clone()]
        );

        let duplicate_json = serde_json::to_string(&[asset_id.as_str(), asset_id.as_str()]).unwrap();
        assert!(matches!(
            repo.update_task(
                &creation_task_id,
                UpdateCreationTaskParams {
                    result_asset_ids: Some(&duplicate_json),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err(),
            DbError::Conflict(message) if message.contains("duplicate asset")
        ));
    }

    #[tokio::test]
    async fn live_and_complete_inventory_include_canonical_tasks() {
        let (repo, db, provider_id) = repo().await;
        let task_ids = [generate_id(), generate_id()];
        create_project_task(&repo, &db, &task_ids[0], &provider_id).await;
        create_project_task(&repo, &db, &task_ids[1], &provider_id).await;
        repo.update_task(&task_ids[1], UpdateCreationTaskParams { status: Some("running"), ..Default::default() })
            .await
            .unwrap();

        // both queued+running are "live"
        let live = repo.list_live_tasks().await.unwrap();
        assert_eq!(live.len(), 2);
        assert_eq!(repo.list_all_tasks().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn update_task_if_live_refuses_terminal_overwrite() {
        let (repo, db, provider_id) = repo().await;
        let canceled_id = generate_id();
        create_project_task(&repo, &db, &canceled_id, &provider_id).await;
        // queued → running (still live)
        repo.update_task(&canceled_id, UpdateCreationTaskParams { status: Some("running"), ..Default::default() })
            .await
            .unwrap();
        // A cancel writes the terminal status (cancel path is unconditional).
        repo.update_task(
            &canceled_id,
            UpdateCreationTaskParams { status: Some("canceled"), finished_at: Some(Some(1)), ..Default::default() },
        )
        .await
        .unwrap();
        // finalize's terminal write must NOT overwrite the canceled row.
        let applied = repo
            .update_task_if_live(
                &canceled_id,
                UpdateCreationTaskParams { status: Some("succeeded"), finished_at: Some(Some(2)), ..Default::default() },
            )
            .await
            .unwrap();
        assert!(!applied, "terminal (canceled) row must not be overwritten");
        assert_eq!(repo.get_task(&canceled_id).await.unwrap().unwrap().status, "canceled");

        // A still-live task IS updated by the conditional write.
        let succeeded_id = generate_id();
        create_project_task(&repo, &db, &succeeded_id, &provider_id).await;
        let applied2 = repo
            .update_task_if_live(&succeeded_id, UpdateCreationTaskParams { status: Some("succeeded"), ..Default::default() })
            .await
            .unwrap();
        assert!(applied2);
        assert_eq!(repo.get_task(&succeeded_id).await.unwrap().unwrap().status, "succeeded");

        // Unknown id → Ok(false), no error.
        let missing_id = generate_id();
        let applied3 = repo
            .update_task_if_live(&missing_id, UpdateCreationTaskParams { status: Some("failed"), ..Default::default() })
            .await
            .unwrap();
        assert!(!applied3);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_id_patch_racing_cancel_never_resurrects_task() {
        let (repo, db, provider_id) = repo().await;
        let repo = Arc::new(repo);
        for _ in 0..64 {
            let creation_task_id = generate_id();
            create_project_task(repo.as_ref(), &db, &creation_task_id, &provider_id).await;
            repo.update_task(
                &creation_task_id,
                UpdateCreationTaskParams {
                    status: Some("running"),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

            let cancel_repo = repo.clone();
            let cancel_id = creation_task_id.clone();
            let cancel = tokio::spawn(async move {
                cancel_repo
                    .update_task(
                        &cancel_id,
                        UpdateCreationTaskParams {
                            status: Some("canceled"),
                            finished_at: Some(Some(1)),
                            ..Default::default()
                        },
                    )
                    .await
                    .unwrap();
            });
            let remote_repo = repo.clone();
            let remote_id = creation_task_id.clone();
            let remote = tokio::spawn(async move {
                remote_repo
                    .set_remote_task_id_if_live(&remote_id, "remote-race")
                    .await
                    .unwrap()
            });
            let (_, remote_applied) = tokio::join!(cancel, remote);
            let _ = remote_applied.unwrap();

            let row = repo.get_task(&creation_task_id).await.unwrap().unwrap();
            assert_eq!(row.status, "canceled");
            assert!(
                !repo
                    .set_remote_task_id_if_live(&creation_task_id, "remote-after-cancel")
                    .await
                    .unwrap(),
                "terminal cancel must make subsequent remote patches no-op"
            );
            assert_eq!(
                repo.get_task(&creation_task_id).await.unwrap().unwrap().status,
                "canceled"
            );
        }
    }

    #[tokio::test]
    async fn canonical_create_rejects_missing_provider_atomically() {
        let (repo, db, _provider_id) = repo().await;
        let missing_provider = ProviderId::new().into_string();
        let creation_task_id = generate_id();
        let project_id = seed_creative_project(&db).await;
        let node_id = CreativeStudioNodeId::new().into_string();

        let error = repo
            .get_or_create_creative_task(creative_params(
                &creation_task_id,
                &project_id,
                &node_id,
                &missing_provider,
                r#"{"missing_provider":true}"#,
            ))
            .await
            .unwrap_err();
        assert!(matches!(error, DbError::Conflict(_)));

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM creation_tasks")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}
