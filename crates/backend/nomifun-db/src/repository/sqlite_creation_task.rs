use nomifun_common::{
    CreationTaskId, CreativeStudioNodeId, CreativeStudioProjectId,
    CreativeStudioWorkflowId, CreativeStudioWorkflowRunId, CreativeStudioWorkflowStepId,
    ProviderId, WorkshopAssetId,
};
#[cfg(test)]
use nomifun_common::validate_uuidv7;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{QueryBuilder, Sqlite, SqlitePool, Transaction};

use crate::error::DbError;
use crate::models::CreationTaskRow;
use crate::repository::ICreationTaskRepository;
use crate::repository::creation_task::{
    CreateCreativeTaskParams, CreativeTaskOwnerRef, IdempotentCreationTask,
    ListStandaloneWorkbenchTasksParams, RetireStandaloneWorkbenchTasksParams,
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
    creation_task_id: String,
    project_id: Option<String>,
    workbench_kind: Option<String>,
    workflow_id: Option<String>,
    workflow_run_id: Option<String>,
    workflow_step_id: Option<String>,
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
            creation_task_id,
            project_id,
            workbench_kind,
            workflow_id,
            workflow_run_id,
            workflow_step_id,
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
        if let Some(kind) = workbench_kind.as_deref()
            && !matches!(kind, "image" | "video" | "audio")
        {
            return Err(DbError::Conflict(format!(
                "creation task {creation_task_id} has invalid workbench_kind {kind:?}"
            )));
        }
        if let Some(id) = workflow_id.as_deref() {
            CreativeStudioWorkflowId::parse(id).map_err(|error| {
                DbError::Conflict(format!(
                    "creation task {creation_task_id} has invalid workflow_id {id:?}: {error}"
                ))
            })?;
        }
        if let Some(id) = workflow_run_id.as_deref() {
            CreativeStudioWorkflowRunId::parse(id).map_err(|error| {
                DbError::Conflict(format!(
                    "creation task {creation_task_id} has invalid workflow_run_id {id:?}: {error}"
                ))
            })?;
        }
        if let Some(id) = workflow_step_id.as_deref() {
            CreativeStudioWorkflowStepId::parse(id).map_err(|error| {
                DbError::Conflict(format!(
                    "creation task {creation_task_id} has invalid workflow_step_id {id:?}: {error}"
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
            && workbench_kind.is_none()
            && workflow_id.is_none()
            && workflow_run_id.is_none()
            && workflow_step_id.is_none();
        let standalone_owner = project_id.is_some()
            && workbench_kind.is_some()
            && node_id.is_none()
            && workflow_id.is_none()
            && workflow_run_id.is_none()
            && workflow_step_id.is_none();
        let workflow_owner = project_id.is_none()
            && workbench_kind.is_none()
            && node_id.is_none()
            && workflow_id.is_some()
            && workflow_run_id.is_some()
            && workflow_step_id.is_some();
        let owner_branches = usize::from(canvas_owner)
            + usize::from(standalone_owner)
            + usize::from(workflow_owner);
        let valid_owner = request_fingerprint.is_some() && owner_branches == 1;
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
                || workbench_kind.is_none()
                || node_id.is_some()
                || workflow_id.is_some()
                || workflow_run_id.is_some()
                || workflow_step_id.is_some()
                || !matches!(status.as_str(), "failed" | "canceled" | "succeeded")
        }) {
            return Err(DbError::Conflict(format!(
                "creation task {creation_task_id} has an invalid retirement tombstone"
            )));
        }
        Ok(Self {
            creation_task_id,
            project_id,
            workbench_kind,
            workflow_id,
            workflow_run_id,
            workflow_step_id,
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
        "text" => Some("chat"),
        _ => None,
    }
}

#[derive(Debug, Clone)]
enum CanonicalTaskOwner {
    CanvasNode {
        project_id: String,
        node_id: String,
    },
    StandaloneWorkbench {
        project_id: String,
        workbench_kind: String,
    },
    WorkflowStep {
        workflow_id: String,
        workflow_run_id: String,
        workflow_step_id: String,
    },
}

fn normalize_canonical_owner(owner: CreativeTaskOwnerRef<'_>) -> Result<CanonicalTaskOwner, DbError> {
    match owner {
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
        CreativeTaskOwnerRef::StandaloneWorkbench {
            project_id,
            workbench_kind,
        } => {
            let workbench_kind = match workbench_kind {
                "image" | "video" | "audio" => workbench_kind.to_owned(),
                other => {
                    return Err(DbError::Conflict(format!(
                        "Creative task workbench_kind {other:?} is invalid"
                    )));
                }
            };
            Ok(CanonicalTaskOwner::StandaloneWorkbench {
                project_id: CreativeStudioProjectId::parse(project_id)
                    .map_err(|error| {
                        DbError::Conflict(format!(
                            "Creative task project_id '{project_id}' is not a canonical UUIDv7: {error}"
                        ))
                    })?
                    .into_string(),
                workbench_kind,
            })
        }
        CreativeTaskOwnerRef::WorkflowStep {
            workflow_id,
            workflow_run_id,
            workflow_step_id,
        } => Ok(CanonicalTaskOwner::WorkflowStep {
            workflow_id: CreativeStudioWorkflowId::parse(workflow_id)
                .map_err(|error| {
                    DbError::Conflict(format!(
                        "Creative task workflow_id '{workflow_id}' is not a canonical UUIDv7: {error}"
                    ))
                })?
                .into_string(),
            workflow_run_id: CreativeStudioWorkflowRunId::parse(workflow_run_id)
                .map_err(|error| {
                    DbError::Conflict(format!(
                        "Creative task workflow_run_id '{workflow_run_id}' is not a canonical UUIDv7: {error}"
                    ))
                })?
                .into_string(),
            workflow_step_id: CreativeStudioWorkflowStepId::parse(workflow_step_id)
                .map_err(|error| {
                    DbError::Conflict(format!(
                        "Creative task workflow_step_id '{workflow_step_id}' is not a canonical UUIDv7: {error}"
                    ))
                })?
                .into_string(),
        }),
    }
}

fn stored_owner_matches(stored: &CreationTaskDbRow, owner: &CanonicalTaskOwner) -> bool {
    match owner {
        CanonicalTaskOwner::CanvasNode {
            project_id,
            node_id,
        } => {
            stored.project_id.as_deref() == Some(project_id)
                && stored.node_id.as_deref() == Some(node_id)
                && stored.workbench_kind.is_none()
                && stored.workflow_id.is_none()
                && stored.workflow_run_id.is_none()
                && stored.workflow_step_id.is_none()
        }
        CanonicalTaskOwner::StandaloneWorkbench {
            project_id,
            workbench_kind,
        } => {
            stored.project_id.as_deref() == Some(project_id)
                && stored.workbench_kind.as_deref() == Some(workbench_kind)
                && stored.node_id.is_none()
                && stored.workflow_id.is_none()
                && stored.workflow_run_id.is_none()
                && stored.workflow_step_id.is_none()
        }
        CanonicalTaskOwner::WorkflowStep {
            workflow_id,
            workflow_run_id,
            workflow_step_id,
        } => {
            stored.project_id.is_none()
                && stored.workbench_kind.is_none()
                && stored.node_id.is_none()
                && stored.workflow_id.as_deref() == Some(workflow_id)
                && stored.workflow_run_id.as_deref() == Some(workflow_run_id)
                && stored.workflow_step_id.as_deref() == Some(workflow_step_id)
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

async fn lock_creative_workflow_step(
    tx: &mut Transaction<'_, Sqlite>,
    workflow_id: &str,
    workflow_run_id: &str,
    workflow_step_id: &str,
) -> Result<(), DbError> {
    let locked = sqlx::query(
        "UPDATE creative_studio_workflow_runs \
         SET updated_at = updated_at \
         WHERE workflow_run_id = ?1 \
           AND workflow_id = ?2 \
           AND status IN ('queued', 'running') \
           AND EXISTS (\
               SELECT 1 FROM json_each(step_ids_json) \
               WHERE json_each.value = ?3\
           )",
    )
    .bind(workflow_run_id)
    .bind(workflow_id)
    .bind(workflow_step_id)
    .execute(&mut **tx)
    .await?;
    if locked.rows_affected() == 0 {
        return Err(DbError::Conflict(format!(
            "Creative workflow task owner run '{workflow_run_id}' is missing, not executable, belongs to another workflow, or does not contain step '{workflow_step_id}'"
        )));
    }
    Ok(())
}

async fn lock_canonical_owner(
    tx: &mut Transaction<'_, Sqlite>,
    owner: &CanonicalTaskOwner,
) -> Result<(), DbError> {
    match owner {
        CanonicalTaskOwner::CanvasNode { project_id, .. }
        | CanonicalTaskOwner::StandaloneWorkbench { project_id, .. } => {
            lock_creative_project(tx, project_id).await?;
        }
        CanonicalTaskOwner::WorkflowStep {
            workflow_id,
            workflow_run_id,
            workflow_step_id,
        } => {
            lock_creative_workflow_step(
                tx,
                workflow_id,
                workflow_run_id,
                workflow_step_id,
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
        // Only migration 043 may produce NULL, to identify legacy rows whose
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

        let (
            project_id,
            workbench_kind,
            workflow_id,
            workflow_run_id,
            workflow_step_id,
            node_id,
        ) = match &owner {
            CanonicalTaskOwner::CanvasNode {
                project_id,
                node_id,
            } => (
                Some(project_id.as_str()),
                None,
                None,
                None,
                None,
                Some(node_id.as_str()),
            ),
            CanonicalTaskOwner::StandaloneWorkbench {
                project_id,
                workbench_kind,
            } => (
                Some(project_id.as_str()),
                Some(workbench_kind.as_str()),
                None,
                None,
                None,
                None,
            ),
            CanonicalTaskOwner::WorkflowStep {
                workflow_id,
                workflow_run_id,
                workflow_step_id,
            } => (
                None,
                None,
                Some(workflow_id.as_str()),
                Some(workflow_run_id.as_str()),
                Some(workflow_step_id.as_str()),
                None,
            ),
        };
        let inserted = sqlx::query(
            "INSERT INTO creation_tasks \
                (creation_task_id, project_id, workbench_kind, workflow_id, workflow_run_id, workflow_step_id, \
                 node_id, provider_id, model, capability, \
                 params, input_bindings, status, error, result_asset_ids, remote_task_id, attempt, submitted_at, \
                 started_at, finished_at, request_fingerprint) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, '[]', NULL, 0, ?, NULL, NULL, ?) \
             ON CONFLICT(creation_task_id) DO NOTHING",
        )
        .bind(params.creation_task_id)
        .bind(project_id)
        .bind(workbench_kind)
        .bind(workflow_id)
        .bind(workflow_run_id)
        .bind(workflow_step_id)
        .bind(node_id)
        .bind(provider_id.as_str())
        .bind(params.model)
        .bind(params.capability)
        .bind(params.params)
        .bind(params.input_bindings)
        .bind(params.status)
        .bind(params.submitted_at)
        .bind(params.request_fingerprint)
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

    async fn list_standalone_workbench_tasks_page(
        &self,
        params: ListStandaloneWorkbenchTasksParams<'_>,
    ) -> Result<Vec<CreationTaskRow>, DbError> {
        let project_id = CreativeStudioProjectId::parse(params.project_id).map_err(|error| {
            DbError::Conflict(format!(
                "Creative task project_id {:?} is not a canonical UUIDv7: {error}",
                params.project_id
            ))
        })?;
        if !matches!(params.workbench_kind, "image" | "video" | "audio") {
            return Err(DbError::Conflict(format!(
                "Creative task workbench_kind {:?} is invalid",
                params.workbench_kind
            )));
        }
        if !(1..=100).contains(&params.limit) {
            return Err(DbError::Conflict(
                "standalone workbench task page limit must be between 1 and 100".into(),
            ));
        }
        if let Some(before) = params.before {
            if before.submitted_at < 0 {
                return Err(DbError::Conflict(
                    "standalone workbench task cursor timestamp must be non-negative".into(),
                ));
            }
            validate_creation_task_id(before.creation_task_id)?;
        }
        let fetch_limit = i64::try_from(params.limit + 1)
            .map_err(|_| DbError::Conflict("standalone workbench task page limit overflow".into()))?;
        let rows = if let Some(before) = params.before {
            sqlx::query_as::<_, CreationTaskDbRow>(
                "SELECT * FROM creation_tasks \
                 WHERE project_id = ?1 AND workbench_kind = ?2 \
                   AND deleted_at IS NULL \
                   AND node_id IS NULL AND workflow_id IS NULL \
                   AND workflow_run_id IS NULL AND workflow_step_id IS NULL \
                   AND (?3 = 0 OR status IN ('queued', 'running')) \
                   AND (submitted_at < ?4 OR (submitted_at = ?4 AND creation_task_id < ?5)) \
                 ORDER BY submitted_at DESC, creation_task_id DESC LIMIT ?6",
            )
            .bind(project_id.as_str())
            .bind(params.workbench_kind)
            .bind(i64::from(params.active_only))
            .bind(before.submitted_at)
            .bind(before.creation_task_id)
            .bind(fetch_limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, CreationTaskDbRow>(
                "SELECT * FROM creation_tasks \
                 WHERE project_id = ?1 AND workbench_kind = ?2 \
                   AND deleted_at IS NULL \
                   AND node_id IS NULL AND workflow_id IS NULL \
                   AND workflow_run_id IS NULL AND workflow_step_id IS NULL \
                   AND (?3 = 0 OR status IN ('queued', 'running')) \
                 ORDER BY submitted_at DESC, creation_task_id DESC LIMIT ?4",
            )
            .bind(project_id.as_str())
            .bind(params.workbench_kind)
            .bind(i64::from(params.active_only))
            .bind(fetch_limit)
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(TryInto::try_into).collect()
    }

    async fn retire_standalone_workbench_tasks(
        &self,
        params: RetireStandaloneWorkbenchTasksParams<'_>,
    ) -> Result<Vec<CreationTaskRow>, DbError> {
        let project_id = CreativeStudioProjectId::parse(params.project_id).map_err(|error| {
            DbError::Conflict(format!(
                "Creative task project_id {:?} is not a canonical UUIDv7: {error}",
                params.project_id
            ))
        })?;
        if !matches!(params.workbench_kind, "image" | "video" | "audio") {
            return Err(DbError::Conflict(format!(
                "Creative task workbench_kind {:?} is invalid",
                params.workbench_kind
            )));
        }
        if params.task_ids.is_empty() || params.task_ids.len() > 100 {
            return Err(DbError::Conflict(
                "standalone workbench retirement requires 1 to 100 task ids".into(),
            ));
        }
        if params.deleted_at < 0 {
            return Err(DbError::Conflict(
                "standalone workbench retirement deleted_at must be non-negative".into(),
            ));
        }
        let mut seen = std::collections::HashSet::with_capacity(params.task_ids.len());
        for task_id in params.task_ids {
            validate_creation_task_id(task_id)?;
            if !seen.insert(task_id.as_str()) {
                return Err(DbError::Conflict(format!(
                    "standalone workbench retirement contains duplicate task id {task_id}"
                )));
            }
        }

        let mut tx = self.pool.begin().await?;
        // Acquire SQLite writer authority before validating the batch, so a
        // worker cannot cross the live/terminal boundary between validation
        // and the tombstone write.
        let mut lock = QueryBuilder::<Sqlite>::new(
            "UPDATE creation_tasks SET status = status WHERE creation_task_id IN (",
        );
        {
            let mut ids = lock.separated(", ");
            for task_id in params.task_ids {
                ids.push_bind(task_id);
            }
        }
        lock.push(")");
        lock.build().execute(&mut *tx).await?;

        let mut select =
            QueryBuilder::<Sqlite>::new("SELECT * FROM creation_tasks WHERE creation_task_id IN (");
        {
            let mut ids = select.separated(", ");
            for task_id in params.task_ids {
                ids.push_bind(task_id);
            }
        }
        select.push(")");
        let rows = select
            .build_query_as::<CreationTaskDbRow>()
            .fetch_all(&mut *tx)
            .await?;
        if rows.len() != params.task_ids.len() {
            return Err(DbError::NotFound(
                "one or more standalone workbench tasks do not exist".into(),
            ));
        }
        for row in &rows {
            let exact_owner = row.project_id.as_deref() == Some(project_id.as_str())
                && row.workbench_kind.as_deref() == Some(params.workbench_kind)
                && row.node_id.is_none()
                && row.workflow_id.is_none()
                && row.workflow_run_id.is_none()
                && row.workflow_step_id.is_none();
            if !exact_owner {
                return Err(DbError::Conflict(format!(
                    "creation task {} does not belong to the requested standalone workbench owner",
                    row.creation_task_id
                )));
            }
            if matches!(row.status.as_str(), "queued" | "running") {
                return Err(DbError::Conflict(format!(
                    "live creation task {} cannot be retired",
                    row.creation_task_id
                )));
            }
            if !matches!(row.status.as_str(), "failed" | "canceled" | "succeeded") {
                return Err(DbError::Conflict(format!(
                    "creation task {} has unsupported terminal status {:?}",
                    row.creation_task_id, row.status
                )));
            }
            if params.deleted_at < row.submitted_at {
                return Err(DbError::Conflict(format!(
                    "retirement timestamp predates creation task {}",
                    row.creation_task_id
                )));
            }
        }
        if rows.iter().any(|row| row.deleted_at.is_none()) {
            let mut update = QueryBuilder::<Sqlite>::new(
                "UPDATE creation_tasks SET deleted_at = COALESCE(deleted_at, ",
            );
            update.push_bind(params.deleted_at);
            update.push(") WHERE creation_task_id IN (");
            {
                let mut ids = update.separated(", ");
                for task_id in params.task_ids {
                    ids.push_bind(task_id);
                }
            }
            update.push(")");
            update.build().execute(&mut *tx).await?;
        }

        let mut refreshed = QueryBuilder::<Sqlite>::new(
            "SELECT * FROM creation_tasks WHERE creation_task_id IN (",
        );
        {
            let mut ids = refreshed.separated(", ");
            for task_id in params.task_ids {
                ids.push_bind(task_id);
            }
        }
        refreshed.push(")");
        let rows = refreshed
            .build_query_as::<CreationTaskDbRow>()
            .fetch_all(&mut *tx)
            .await?;
        let mut by_id = rows
            .into_iter()
            .map(|row| Ok((row.creation_task_id.clone(), CreationTaskRow::try_from(row)?)))
            .collect::<Result<std::collections::HashMap<_, _>, DbError>>()?;
        let ordered = params
            .task_ids
            .iter()
            .map(|task_id| {
                by_id.remove(task_id).ok_or_else(|| {
                    DbError::Init(format!(
                        "retired creation task {task_id} vanished before commit"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        tx.commit().await?;
        Ok(ordered)
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
        .await?;
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
        .await?;
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

    async fn seed_creative_workflow_run(
        db: &crate::Database,
    ) -> (String, String, String) {
        let workflow_id = CreativeStudioWorkflowId::new().into_string();
        let workflow_run_id = CreativeStudioWorkflowRunId::new().into_string();
        let workflow_step_id = CreativeStudioWorkflowStepId::new().into_string();
        let definition = serde_json::json!({
            "id": workflow_id,
            "revision": 1
        });
        sqlx::query(
            "INSERT INTO creative_studio_workflows \
                (workflow_id, revision, name, description, category, visibility, definition_json, \
                 created_at, updated_at) \
             VALUES (?, 1, 'Task Owner Test', '', '', 'private', ?, 0, 0)",
        )
        .bind(&workflow_id)
        .bind(definition.to_string())
        .execute(db.pool())
        .await
        .unwrap();
        let aggregate = serde_json::json!({
            "kind": "nomifun.creative-studio.workflow-run",
            "version": 1,
            "revision": 1,
            "workflowSnapshot": { "id": workflow_id, "revision": 1 },
            "request": {
                "id": workflow_run_id,
                "workflowId": workflow_id,
                "workflowRevision": 1
            },
            "record": {
                "requestId": workflow_run_id,
                "workflowId": workflow_id,
                "status": "queued"
            }
        });
        sqlx::query(
            "INSERT INTO creative_studio_workflow_runs \
                (workflow_run_id, workflow_id, workflow_revision, revision, status, step_ids_json, \
                 aggregate_json, created_at, updated_at) \
             VALUES (?, ?, 1, 1, 'queued', ?, ?, 0, 0)",
        )
        .bind(&workflow_run_id)
        .bind(&workflow_id)
        .bind(serde_json::to_string(&[&workflow_step_id]).unwrap())
        .bind(aggregate.to_string())
        .execute(db.pool())
        .await
        .unwrap();
        (workflow_id, workflow_run_id, workflow_step_id)
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

    fn workflow_creative_params<'a>(
        creation_task_id: &'a str,
        workflow_id: &'a str,
        workflow_run_id: &'a str,
        workflow_step_id: &'a str,
        provider_id: &'a str,
        fingerprint: &'a str,
    ) -> CreateCreativeTaskParams<'a> {
        CreateCreativeTaskParams {
            creation_task_id,
            owner: CreativeTaskOwnerRef::WorkflowStep {
                workflow_id,
                workflow_run_id,
                workflow_step_id,
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

    fn standalone_creative_params<'a>(
        creation_task_id: &'a str,
        project_id: &'a str,
        workbench_kind: &'a str,
        provider_id: &'a str,
        input_bindings: &'a str,
        fingerprint: &'a str,
    ) -> CreateCreativeTaskParams<'a> {
        CreateCreativeTaskParams {
            creation_task_id,
            owner: CreativeTaskOwnerRef::StandaloneWorkbench {
                project_id,
                workbench_kind,
            },
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
        workflow_id: Option<&str>,
        node_id: Option<&str>,
        provider_id: &str,
        request_fingerprint: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO creation_tasks \
                (creation_task_id, project_id, workflow_id, node_id, provider_id, model, capability, \
                 params, status, submitted_at, request_fingerprint) \
             VALUES (?, ?, ?, ?, ?, 'image-model-v1', 't2i', '{}', 'queued', 100, ?)",
        )
        .bind(creation_task_id)
        .bind(project_id)
        .bind(workflow_id)
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
        let workflow_id = CreativeStudioWorkflowId::new().into_string();

        let mixed = raw_insert_task_ownership(
            &db,
            &CreationTaskId::new().into_string(),
            Some(&project_id),
            Some(&workflow_id),
            Some(&node_id),
            &provider_id,
            Some(r#"{"project_id":"mixed"}"#),
        )
        .await;
        assert!(mixed.is_err(), "project and workflow owners must be exclusive");

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
    async fn standalone_owner_and_ordered_inputs_are_frozen_by_idempotency() {
        let (repo, db, provider_id) = repo().await;
        let project_id = seed_creative_project(&db).await;
        let task_id = CreationTaskId::new().into_string();
        let first_asset = WorkshopAssetId::new().into_string();
        let second_asset = WorkshopAssetId::new().into_string();
        let bindings = serde_json::json!([
            {"asset_id": first_asset, "kind": "image", "role": "first_frame"},
            {"asset_id": second_asset, "kind": "image", "role": "last_frame"}
        ])
        .to_string();
        let fingerprint = serde_json::json!({
            "owner": {
                "kind": "standalone_workbench",
                "project_id": project_id,
                "workbench_kind": "video"
            },
            "inputs": serde_json::from_str::<Value>(&bindings).unwrap()
        })
        .to_string();

        let created = repo
            .get_or_create_creative_task(standalone_creative_params(
                &task_id,
                &project_id,
                "video",
                &provider_id,
                &bindings,
                &fingerprint,
            ))
            .await
            .unwrap();
        assert!(created.inserted);
        assert_eq!(created.row.project_id.as_deref(), Some(project_id.as_str()));
        assert_eq!(created.row.workbench_kind.as_deref(), Some("video"));
        assert!(created.row.node_id.is_none());
        assert_eq!(created.row.input_bindings.as_deref(), Some(bindings.as_str()));

        let replay = repo
            .get_or_create_creative_task(standalone_creative_params(
                &task_id,
                &project_id,
                "video",
                &provider_id,
                &bindings,
                &fingerprint,
            ))
            .await
            .unwrap();
        assert!(!replay.inserted);

        let wrong_owner = repo
            .get_or_create_creative_task(standalone_creative_params(
                &task_id,
                &project_id,
                "image",
                &provider_id,
                &bindings,
                &fingerprint,
            ))
            .await
            .unwrap_err();
        assert!(matches!(wrong_owner, DbError::Conflict(message) if message.contains("inconsistent")));

        let reversed = serde_json::json!([
            {"asset_id": second_asset, "kind": "image", "role": "last_frame"},
            {"asset_id": first_asset, "kind": "image", "role": "first_frame"}
        ])
        .to_string();
        let wrong_inputs = repo
            .get_or_create_creative_task(standalone_creative_params(
                &task_id,
                &project_id,
                "video",
                &provider_id,
                &reversed,
                &fingerprint,
            ))
            .await
            .unwrap_err();
        assert!(matches!(wrong_inputs, DbError::Conflict(message) if message.contains("inconsistent")));
    }

    #[tokio::test]
    async fn standalone_page_uses_descending_two_column_cursor_and_exact_owner_scope() {
        let (repo, db, provider_id) = repo().await;
        let project_id = seed_creative_project(&db).await;
        let other_project_id = seed_creative_project(&db).await;
        let mut matching = Vec::new();
        for submitted_at in [300, 200, 200, 100] {
            let task_id = CreationTaskId::new().into_string();
            let fingerprint = serde_json::json!({"task": task_id}).to_string();
            let mut params = standalone_creative_params(
                &task_id,
                &project_id,
                "video",
                &provider_id,
                "[]",
                &fingerprint,
            );
            params.submitted_at = submitted_at;
            repo.get_or_create_creative_task(params).await.unwrap();
            matching.push((submitted_at, task_id));
        }
        for (other_project, kind) in [(&other_project_id, "video"), (&project_id, "image")] {
            let task_id = CreationTaskId::new().into_string();
            let fingerprint = serde_json::json!({"task": task_id}).to_string();
            let mut params = standalone_creative_params(
                &task_id,
                other_project,
                kind,
                &provider_id,
                "[]",
                &fingerprint,
            );
            params.submitted_at = 400;
            repo.get_or_create_creative_task(params).await.unwrap();
        }
        matching.sort_by(|left, right| right.cmp(left));

        let first = repo
            .list_standalone_workbench_tasks_page(ListStandaloneWorkbenchTasksParams {
                project_id: &project_id,
                workbench_kind: "video",
                active_only: false,
                before: None,
                limit: 2,
            })
            .await
            .unwrap();
        assert_eq!(first.len(), 3, "repository fetches limit + 1");
        assert_eq!(
            first
                .iter()
                .map(|row| (row.submitted_at, row.creation_task_id.clone()))
                .collect::<Vec<_>>(),
            matching[..3]
        );

        let visible_last = &first[1];
        let second = repo
            .list_standalone_workbench_tasks_page(ListStandaloneWorkbenchTasksParams {
                project_id: &project_id,
                workbench_kind: "video",
                active_only: false,
                before: Some(crate::repository::CreationTaskPageCursorRef {
                    submitted_at: visible_last.submitted_at,
                    creation_task_id: &visible_last.creation_task_id,
                }),
                limit: 2,
            })
            .await
            .unwrap();
        assert_eq!(
            second
                .iter()
                .map(|row| (row.submitted_at, row.creation_task_id.clone()))
                .collect::<Vec<_>>(),
            matching[2..]
        );

        repo.update_task(
            &matching[0].1,
            UpdateCreationTaskParams {
                status: Some("failed"),
                error: Some(Some(r#"{"kind":"provider","message":"fixture"}"#)),
                finished_at: Some(Some(301)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let active = repo
            .list_standalone_workbench_tasks_page(ListStandaloneWorkbenchTasksParams {
                project_id: &project_id,
                workbench_kind: "video",
                active_only: true,
                before: None,
                limit: 100,
            })
            .await
            .unwrap();
        assert_eq!(
            active
                .iter()
                .map(|row| (row.submitted_at, row.creation_task_id.clone()))
                .collect::<Vec<_>>(),
            matching[1..]
        );
    }

    #[tokio::test]
    async fn standalone_retirement_is_terminal_owner_scoped_atomic_and_idempotent() {
        let (repo, db, provider_id) = repo().await;
        let project_id = seed_creative_project(&db).await;
        let mut ids = Vec::new();
        for (index, status) in ["failed", "canceled", "queued"].into_iter().enumerate() {
            let task_id = CreationTaskId::new().into_string();
            let fingerprint = serde_json::json!({"task": task_id}).to_string();
            let mut create = standalone_creative_params(
                &task_id,
                &project_id,
                "video",
                &provider_id,
                "[]",
                &fingerprint,
            );
            create.submitted_at = 100 + index as i64;
            repo.get_or_create_creative_task(create).await.unwrap();
            if status != "queued" {
                repo.update_task(
                    &task_id,
                    UpdateCreationTaskParams {
                        status: Some(status),
                        finished_at: Some(Some(200 + index as i64)),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            }
            ids.push(task_id);
        }
        let retired = repo
            .retire_standalone_workbench_tasks(RetireStandaloneWorkbenchTasksParams {
                project_id: &project_id,
                workbench_kind: "video",
                task_ids: &ids[..2],
                deleted_at: 500,
            })
            .await
            .unwrap();
        assert_eq!(
            retired
                .iter()
                .map(|row| row.creation_task_id.as_str())
                .collect::<Vec<_>>(),
            ids[..2].iter().map(String::as_str).collect::<Vec<_>>()
        );
        assert!(retired.iter().all(|row| row.deleted_at == Some(500)));
        let replay_fingerprint = serde_json::json!({"task": &ids[0]}).to_string();
        let replay = repo
            .get_or_create_creative_task(standalone_creative_params(
                &ids[0],
                &project_id,
                "video",
                &provider_id,
                "[]",
                &replay_fingerprint,
            ))
            .await
            .unwrap();
        assert!(!replay.inserted);
        assert_eq!(replay.row.deleted_at, Some(500));
        let visible = repo
            .list_standalone_workbench_tasks_page(ListStandaloneWorkbenchTasksParams {
                project_id: &project_id,
                workbench_kind: "video",
                active_only: false,
                before: None,
                limit: 10,
            })
            .await
            .unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].creation_task_id, ids[2]);

        let repeated = repo
            .retire_standalone_workbench_tasks(RetireStandaloneWorkbenchTasksParams {
                project_id: &project_id,
                workbench_kind: "video",
                task_ids: &ids[..2],
                deleted_at: 600,
            })
            .await
            .unwrap();
        assert!(repeated.iter().all(|row| row.deleted_at == Some(500)));

        let new_terminal_id = CreationTaskId::new().into_string();
        let fingerprint = serde_json::json!({"task": new_terminal_id}).to_string();
        let mut create = standalone_creative_params(
            &new_terminal_id,
            &project_id,
            "video",
            &provider_id,
            "[]",
            &fingerprint,
        );
        create.submitted_at = 150;
        repo.get_or_create_creative_task(create).await.unwrap();
        repo.update_task(
            &new_terminal_id,
            UpdateCreationTaskParams {
                status: Some("failed"),
                finished_at: Some(Some(151)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mixed_terminal_ids = vec![ids[0].clone(), new_terminal_id.clone()];
        let mixed_terminal = repo
            .retire_standalone_workbench_tasks(RetireStandaloneWorkbenchTasksParams {
                project_id: &project_id,
                workbench_kind: "video",
                task_ids: &mixed_terminal_ids,
                deleted_at: 700,
            })
            .await
            .unwrap();
        assert_eq!(mixed_terminal[0].deleted_at, Some(500));
        assert_eq!(mixed_terminal[1].deleted_at, Some(700));

        let mixed_live = repo
            .retire_standalone_workbench_tasks(RetireStandaloneWorkbenchTasksParams {
                project_id: &project_id,
                workbench_kind: "video",
                task_ids: &[ids[0].clone(), ids[2].clone()],
                deleted_at: 700,
            })
            .await;
        assert!(matches!(mixed_live, Err(DbError::Conflict(message)) if message.contains("live")));
        assert_eq!(
            repo.get_task(&ids[0]).await.unwrap().unwrap().deleted_at,
            Some(500),
            "failed mixed batch must not rewrite an existing tombstone"
        );
        assert!(repo.get_task(&ids[2]).await.unwrap().unwrap().deleted_at.is_none());

        let wrong_owner = repo
            .retire_standalone_workbench_tasks(RetireStandaloneWorkbenchTasksParams {
                project_id: &project_id,
                workbench_kind: "image",
                task_ids: &[ids[0].clone()],
                deleted_at: 800,
            })
            .await;
        assert!(matches!(wrong_owner, Err(DbError::Conflict(message)) if message.contains("does not belong")));
        let missing_id = CreationTaskId::new().into_string();
        let missing = repo
            .retire_standalone_workbench_tasks(RetireStandaloneWorkbenchTasksParams {
                project_id: &project_id,
                workbench_kind: "video",
                task_ids: &[missing_id],
                deleted_at: 800,
            })
            .await;
        assert!(matches!(missing, Err(DbError::NotFound(_))));
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
    async fn workflow_step_owner_requires_an_executable_run_and_exact_step() {
        let (repo, db, provider_id) = repo().await;
        let (workflow_id, workflow_run_id, workflow_step_id) =
            seed_creative_workflow_run(&db).await;
        sqlx::query(
            "UPDATE creative_studio_workflow_runs SET status = 'running', \
             aggregate_json = json_set(aggregate_json, '$.record.status', 'running') \
             WHERE workflow_run_id = ?",
        )
        .bind(&workflow_run_id)
        .execute(db.pool())
        .await
        .unwrap();
        let task_id = CreationTaskId::new().into_string();
        let fingerprint = r#"{"owner":{"kind":"workflow_step"}}"#;

        let first = repo
            .get_or_create_creative_task(workflow_creative_params(
                &task_id,
                &workflow_id,
                &workflow_run_id,
                &workflow_step_id,
                &provider_id,
                fingerprint,
            ))
            .await
            .unwrap();
        assert!(first.inserted);
        assert_eq!(first.row.workflow_id.as_deref(), Some(workflow_id.as_str()));
        assert_eq!(
            first.row.workflow_run_id.as_deref(),
            Some(workflow_run_id.as_str())
        );
        assert_eq!(
            first.row.workflow_step_id.as_deref(),
            Some(workflow_step_id.as_str())
        );
        assert!(first.row.project_id.is_none());
        assert!(first.row.node_id.is_none());

        let replay = repo
            .get_or_create_creative_task(workflow_creative_params(
                &task_id,
                &workflow_id,
                &workflow_run_id,
                &workflow_step_id,
                &provider_id,
                fingerprint,
            ))
            .await
            .unwrap();
        assert!(!replay.inserted);

        let missing_step = CreativeStudioWorkflowStepId::new().into_string();
        let error = repo
            .get_or_create_creative_task(workflow_creative_params(
                &CreationTaskId::new().into_string(),
                &workflow_id,
                &workflow_run_id,
                &missing_step,
                &provider_id,
                r#"{"owner":{"kind":"workflow_step","attempt":2}}"#,
            ))
            .await
            .unwrap_err();
        assert!(matches!(error, DbError::Conflict(message) if message.contains("does not contain step")));

        sqlx::query(
            "UPDATE creative_studio_workflow_runs SET status = 'succeeded', \
             aggregate_json = json_set(aggregate_json, '$.record.status', 'succeeded') \
             WHERE workflow_run_id = ?",
        )
        .bind(&workflow_run_id)
        .execute(db.pool())
        .await
        .unwrap();
        let terminal_error = repo
            .get_or_create_creative_task(workflow_creative_params(
                &CreationTaskId::new().into_string(),
                &workflow_id,
                &workflow_run_id,
                &workflow_step_id,
                &provider_id,
                r#"{"owner":{"kind":"workflow_step","attempt":3}}"#,
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
