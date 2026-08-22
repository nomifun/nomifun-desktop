use sqlx::{QueryBuilder, Sqlite, SqlitePool};
use serde_json::Value;

use crate::error::DbError;
use crate::models::{
    CreativeStudioAgentProposalReceiptRow, CreativeStudioProjectRow, CreativeStudioWorkflowRow,
    CreativeStudioWorkflowRunRow, WorkshopAssetRow,
};
use crate::repository::IWorkshopRepository;
use crate::repository::workshop::{
    ApplyCreativeAgentProposalParams, AssetSort, CreativeAgentProposalCommit, ListAssetsParams,
    UpdateAssetParams,
};

/// SQLite-backed implementation of [`IWorkshopRepository`].
#[derive(Clone, Debug)]
pub struct SqliteWorkshopRepository {
    pool: SqlitePool,
}

/// Map a [`AssetSort`] to its ORDER BY clause. The strings are fixed literals
/// (never user input), each with an `id` tiebreaker for a stable total order.
fn order_by_sql(sort: AssetSort) -> &'static str {
    match sort {
        AssetSort::CreatedDesc => "created_at DESC, id DESC",
        AssetSort::CreatedAsc => "created_at ASC, id ASC",
        AssetSort::UpdatedDesc => "updated_at DESC, id DESC",
        AssetSort::TitleAsc => "title COLLATE NOCASE ASC, id DESC",
        AssetSort::SizeDesc => "bytes DESC, id DESC",
    }
}

impl SqliteWorkshopRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

struct OriginReferences {
    provider_id: Option<String>,
    project_id: Option<String>,
    node_id: Option<String>,
    workbench_kind: Option<String>,
    workflow_id: Option<String>,
    workflow_run_id: Option<String>,
    workflow_step_id: Option<String>,
    creation_task_id: Option<String>,
}

fn optional_origin_id(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<String>, DbError> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => {
            nomifun_common::validate_uuidv7(value).map_err(|error| {
                DbError::Conflict(format!(
                    "workshop asset origin.{key} is not a canonical UUIDv7: {error}"
                ))
            })?;
            Ok(Some(value.clone()))
        }
        Some(Value::Null) => Err(DbError::Conflict(format!(
            "workshop asset origin.{key} must be omitted when absent; JSON null is not valid"
        ))),
        Some(_) => Err(DbError::Conflict(format!(
            "workshop asset origin.{key} must be a canonical UUIDv7 string"
        ))),
    }
}

fn origin_references(origin: Option<&str>) -> Result<OriginReferences, DbError> {
    let Some(origin) = origin else {
        return Ok(OriginReferences {
            provider_id: None,
            project_id: None,
            node_id: None,
            workbench_kind: None,
            workflow_id: None,
            workflow_run_id: None,
            workflow_step_id: None,
            creation_task_id: None,
        });
    };
    let value: Value = serde_json::from_str(origin)
        .map_err(|error| DbError::Conflict(format!("invalid workshop asset origin JSON: {error}")))?;
    let object = value.as_object().ok_or_else(|| {
        DbError::Conflict("workshop asset origin must be a JSON object".into())
    })?;
    for retired_key in [
        "task_id",
        "providerId",
        "canvas_id",
        "canvasId",
        "nodeId",
        "creationTaskId",
        "projectId",
        "workbenchKind",
        "workflowId",
        "workflowRunId",
        "workflowStepId",
    ] {
        if object.contains_key(retired_key) {
            return Err(DbError::Conflict(format!(
                "workshop asset origin contains unsupported ID field {retired_key:?}"
            )));
        }
    }
    let provider_id = optional_origin_id(object, "provider_id")?;
    let project_id = optional_origin_id(object, "project_id")?;
    let node_id = optional_origin_id(object, "node_id")?;
    let workbench_kind = match object.get("workbench_kind") {
        None => None,
        Some(Value::String(value)) if matches!(value.as_str(), "image" | "video" | "audio") => {
            Some(value.clone())
        }
        Some(Value::String(value)) => {
            return Err(DbError::Conflict(format!(
                "workshop asset origin.workbench_kind {value:?} is invalid"
            )));
        }
        Some(Value::Null) => {
            return Err(DbError::Conflict(
                "workshop asset origin.workbench_kind must be omitted when absent".into(),
            ));
        }
        Some(_) => {
            return Err(DbError::Conflict(
                "workshop asset origin.workbench_kind must be image, video, or audio".into(),
            ));
        }
    };
    let workflow_id = optional_origin_id(object, "workflow_id")?;
    let workflow_run_id = optional_origin_id(object, "workflow_run_id")?;
    let workflow_step_id = optional_origin_id(object, "workflow_step_id")?;
    let creation_task_id = optional_origin_id(object, "creation_task_id")?;
    let canvas_owner = project_id.is_some() && node_id.is_some() && workbench_kind.is_none();
    let standalone_owner =
        project_id.is_some() && node_id.is_none() && workbench_kind.is_some();
    let workflow_owner_count = [
        workflow_id.is_some(),
        workflow_run_id.is_some(),
        workflow_step_id.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    if workflow_owner_count != 0 && workflow_owner_count != 3 {
        return Err(DbError::Conflict(
            "workshop asset workflow origin requires workflow_id, workflow_run_id, and workflow_step_id"
                .into(),
        ));
    }
    let any_project_owner = project_id.is_some() || node_id.is_some() || workbench_kind.is_some();
    if any_project_owner && !canvas_owner && !standalone_owner {
        return Err(DbError::Conflict(
            "workshop asset project origin requires exactly one node_id or workbench_kind branch"
                .into(),
        ));
    }
    if any_project_owner && workflow_owner_count != 0 {
        return Err(DbError::Conflict(
            "workshop asset origin cannot combine project and workflow ownership".into(),
        ));
    }
    Ok(OriginReferences {
        provider_id,
        project_id,
        node_id,
        workbench_kind,
        workflow_id,
        workflow_run_id,
        workflow_step_id,
        creation_task_id,
    })
}

fn validate_asset_row(row: &WorkshopAssetRow) -> Result<(), DbError> {
    nomifun_common::WorkshopAssetId::parse(&row.asset_id).map_err(|error| {
        DbError::Conflict(format!(
            "workshop asset asset_id {:?} is not a canonical UUIDv7: {error}",
            row.asset_id
        ))
    })?;
    origin_references(row.origin.as_deref()).map_err(|error| {
        DbError::Conflict(format!(
            "workshop asset {} has invalid durable origin: {error}",
            row.asset_id
        ))
    })?;
    Ok(())
}

fn validate_asset_rows(rows: &[WorkshopAssetRow]) -> Result<(), DbError> {
    for row in rows {
        validate_asset_row(row)?;
    }
    Ok(())
}

fn validate_creative_workflow_run_row_ids(
    row: &CreativeStudioWorkflowRunRow,
) -> Result<(), DbError> {
    nomifun_common::CreativeStudioWorkflowRunId::parse(&row.workflow_run_id).map_err(|error| {
        DbError::Conflict(format!(
            "creative studio workflow_run_id {:?} is not a canonical UUIDv7: {error}",
            row.workflow_run_id
        ))
    })?;
    nomifun_common::CreativeStudioWorkflowId::parse(&row.workflow_id).map_err(|error| {
        DbError::Conflict(format!(
            "creative studio workflow_id {:?} is not a canonical UUIDv7: {error}",
            row.workflow_id
        ))
    })?;
    Ok(())
}

fn workflow_run_json_references_asset(
    aggregate_json: &str,
    asset_id: &str,
) -> Result<bool, DbError> {
    fn contains(value: &Value, asset_id: &str) -> bool {
        match value {
            Value::Object(object) => object.iter().any(|(key, value)| {
                match key.as_str() {
                    "assetId" | "defaultAssetId" => value.as_str() == Some(asset_id),
                    "assetIds" | "defaultAssetIds" | "referenceAssetIds"
                    | "resultAssetIds" => value.as_array().is_some_and(|values| {
                        values.iter().any(|value| value.as_str() == Some(asset_id))
                    }),
                    _ => contains(value, asset_id),
                }
            }),
            Value::Array(values) => values.iter().any(|value| contains(value, asset_id)),
            _ => false,
        }
    }

    let aggregate: Value = serde_json::from_str(aggregate_json).map_err(|error| {
        DbError::Conflict(format!(
            "stored creative studio workflow run has invalid aggregate JSON: {error}"
        ))
    })?;
    Ok(contains(&aggregate, asset_id))
}

#[async_trait::async_trait]
impl IWorkshopRepository for SqliteWorkshopRepository {
    async fn provider_exists(&self, provider_id: &str) -> Result<bool, DbError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM providers WHERE provider_id = ?)",
        )
        .bind(provider_id)
        .fetch_one(&self.pool)
        .await?)
    }

    async fn provider_model_exists(
        &self,
        provider_id: &str,
        model: &str,
    ) -> Result<bool, DbError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(\
                SELECT 1 FROM provider_models \
                WHERE provider_id = ? AND model = ?\
            )",
        )
        .bind(provider_id)
        .bind(model)
        .fetch_one(&self.pool)
        .await?)
    }

    async fn provider_model_supports_task(
        &self,
        provider_id: &str,
        model: &str,
        task: &str,
    ) -> Result<bool, DbError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(\
                SELECT 1 \
                FROM provider_models model_row \
                JOIN providers provider ON provider.provider_id = model_row.provider_id \
                JOIN provider_model_capabilities capability \
                  ON capability.provider_id = model_row.provider_id \
                 AND capability.model = model_row.model \
                WHERE model_row.provider_id = ? AND model_row.model = ? \
                  AND capability.task = ? AND model_row.enabled = 1 AND provider.enabled = 1\
            )",
        )
        .bind(provider_id)
        .bind(model)
        .bind(task)
        .fetch_one(&self.pool)
        .await?)
    }

    // ---- canonical Creative Studio projects ----

    async fn list_creative_projects(&self) -> Result<Vec<CreativeStudioProjectRow>, DbError> {
        let rows = sqlx::query_as::<_, CreativeStudioProjectRow>(
            "SELECT * FROM creative_studio_projects ORDER BY updated_at DESC, id DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        for row in &rows {
            nomifun_common::validate_uuidv7(&row.project_id).map_err(|error| {
                DbError::Conflict(format!(
                    "creative studio project_id {:?} is not a canonical UUIDv7: {error}",
                    row.project_id
                ))
            })?;
        }
        Ok(rows)
    }

    async fn get_creative_project(
        &self,
        project_id: &str,
    ) -> Result<Option<CreativeStudioProjectRow>, DbError> {
        let row = sqlx::query_as::<_, CreativeStudioProjectRow>(
            "SELECT * FROM creative_studio_projects WHERE project_id = ?",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(row) = &row {
            nomifun_common::validate_uuidv7(&row.project_id).map_err(|error| {
                DbError::Conflict(format!(
                    "creative studio project_id {:?} is not a canonical UUIDv7: {error}",
                    row.project_id
                ))
            })?;
        }
        Ok(row)
    }

    async fn create_creative_project(
        &self,
        project_id: &str,
        title: &str,
        document_json: &str,
        now: i64,
    ) -> Result<CreativeStudioProjectRow, DbError> {
        nomifun_common::validate_uuidv7(project_id).map_err(|error| {
            DbError::Conflict(format!(
                "creative studio project_id {project_id:?} is not a canonical UUIDv7: {error}"
            ))
        })?;
        let row = sqlx::query_as::<_, CreativeStudioProjectRow>(
            "INSERT INTO creative_studio_projects \
                (project_id, title, revision, node_count, connection_count, document_json, created_at, updated_at) \
             VALUES (?, ?, 1, 0, 0, ?, ?, ?) RETURNING *",
        )
        .bind(project_id)
        .bind(title)
        .bind(document_json)
        .bind(now)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    async fn rename_creative_project(
        &self,
        project_id: &str,
        title: &str,
        now: i64,
    ) -> Result<CreativeStudioProjectRow, DbError> {
        let row = sqlx::query_as::<_, CreativeStudioProjectRow>(
            "UPDATE creative_studio_projects SET title = ?, updated_at = ? \
             WHERE project_id = ? RETURNING *",
        )
        .bind(title)
        .bind(now)
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            DbError::NotFound(format!("creative studio project '{project_id}' not found"))
        })?;
        Ok(row)
    }

    async fn save_creative_project(
        &self,
        project_id: &str,
        expected_revision: i64,
        document_json: &str,
        node_count: i64,
        connection_count: i64,
        now: i64,
    ) -> Result<CreativeStudioProjectRow, DbError> {
        let row = sqlx::query_as::<_, CreativeStudioProjectRow>(
            "UPDATE creative_studio_projects \
             SET document_json = ?, node_count = ?, connection_count = ?, \
                 revision = revision + 1, updated_at = ? \
             WHERE project_id = ? AND revision = ? RETURNING *",
        )
        .bind(document_json)
        .bind(node_count)
        .bind(connection_count)
        .bind(now)
        .bind(project_id)
        .bind(expected_revision)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(row) = row {
            return Ok(row);
        }
        if self.get_creative_project(project_id).await?.is_none() {
            return Err(DbError::NotFound(format!(
                "creative studio project '{project_id}' not found"
            )));
        }
        Err(DbError::Conflict(format!(
            "creative studio project '{project_id}' revision conflict"
        )))
    }

    async fn get_creative_agent_proposal_receipt(
        &self,
        owner_id: &str,
        project_id: &str,
        assistant_message_id: &str,
    ) -> Result<Option<CreativeStudioAgentProposalReceiptRow>, DbError> {
        nomifun_common::UserId::parse(owner_id).map_err(|error| {
            DbError::Conflict(format!(
                "creative studio proposal owner_id is not a canonical UUIDv7: {error}"
            ))
        })?;
        nomifun_common::CreativeStudioProjectId::parse(project_id).map_err(|error| {
            DbError::Conflict(format!(
                "creative studio proposal project_id is not a canonical UUIDv7: {error}"
            ))
        })?;
        nomifun_common::MessageId::parse(assistant_message_id).map_err(|error| {
            DbError::Conflict(format!(
                "creative studio proposal assistant_message_id is not a canonical UUIDv7: {error}"
            ))
        })?;
        Ok(sqlx::query_as::<_, CreativeStudioAgentProposalReceiptRow>(
            "SELECT receipt.* \
             FROM creative_studio_agent_proposal_receipts receipt \
             WHERE receipt.project_id = ? AND receipt.assistant_message_id = ? \
               AND EXISTS ( \
                   SELECT 1 \
                   FROM creative_studio_projects project \
                   CROSS JOIN json_each(project.document_json, '$.chatSessions') session \
                   CROSS JOIN json_each(session.value, '$.messageIds') message \
                   JOIN creative_studio_agent_sessions binding \
                     ON binding.project_id = project.project_id \
                    AND binding.session_id = json_extract(session.value, '$.id') \
                    AND binding.owner_id = ? \
                   JOIN installation_identity identity \
                     ON identity.singleton_key = 'installation' \
                    AND identity.owner_user_id = binding.owner_id \
                   JOIN messages persisted \
                     ON persisted.conversation_id = binding.conversation_id \
                    AND persisted.message_id = receipt.assistant_message_id \
                   WHERE project.project_id = receipt.project_id \
                     AND CAST(message.key AS INTEGER) % 2 = 1 \
                     AND CAST(message.value AS TEXT) = receipt.assistant_message_id \
                     AND persisted.position = 'left' \
                     AND persisted.status = 'finish' \
                     AND persisted.hidden = 0 \
                     AND persisted.type = 'text' \
               )",
        )
        .bind(project_id)
        .bind(assistant_message_id)
        .bind(owner_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn is_creative_studio_owner(&self, owner_id: &str) -> Result<bool, DbError> {
        nomifun_common::UserId::parse(owner_id).map_err(|error| {
            DbError::Conflict(format!(
                "creative studio owner_id is not a canonical UUIDv7: {error}"
            ))
        })?;
        Ok(sqlx::query_scalar(
            "SELECT EXISTS( \
                 SELECT 1 FROM installation_identity \
                 WHERE singleton_key = 'installation' AND owner_user_id = ? \
             )",
        )
        .bind(owner_id)
        .fetch_one(&self.pool)
        .await?)
    }

    async fn get_creative_agent_proposal_message_content(
        &self,
        owner_id: &str,
        project_id: &str,
        assistant_message_id: &str,
    ) -> Result<Option<String>, DbError> {
        nomifun_common::UserId::parse(owner_id).map_err(|error| {
            DbError::Conflict(format!(
                "creative studio proposal owner_id is not a canonical UUIDv7: {error}"
            ))
        })?;
        nomifun_common::CreativeStudioProjectId::parse(project_id).map_err(|error| {
            DbError::Conflict(format!(
                "creative studio proposal project_id is not a canonical UUIDv7: {error}"
            ))
        })?;
        nomifun_common::MessageId::parse(assistant_message_id).map_err(|error| {
            DbError::Conflict(format!(
                "creative studio proposal assistant_message_id is not a canonical UUIDv7: {error}"
            ))
        })?;
        let contents = sqlx::query_scalar::<_, String>(
            "SELECT persisted.content \
             FROM creative_studio_projects project \
             CROSS JOIN json_each(project.document_json, '$.chatSessions') session \
             CROSS JOIN json_each(session.value, '$.messageIds') message \
             JOIN creative_studio_agent_sessions binding \
               ON binding.project_id = project.project_id \
              AND binding.session_id = json_extract(session.value, '$.id') \
              AND binding.owner_id = ? \
             JOIN installation_identity identity \
               ON identity.singleton_key = 'installation' \
              AND identity.owner_user_id = binding.owner_id \
             JOIN messages persisted \
               ON persisted.conversation_id = binding.conversation_id \
              AND persisted.message_id = CAST(message.value AS TEXT) \
             WHERE project.project_id = ? \
               AND CAST(message.key AS INTEGER) % 2 = 1 \
               AND CAST(message.value AS TEXT) = ? \
               AND persisted.position = 'left' \
               AND persisted.status = 'finish' \
               AND persisted.hidden = 0 \
               AND persisted.type = 'text'",
        )
        .bind(owner_id)
        .bind(project_id)
        .bind(assistant_message_id)
        .fetch_all(&self.pool)
        .await?;
        match contents.as_slice() {
            [] => Ok(None),
            [content] => Ok(Some(content.clone())),
            _ => Err(DbError::Conflict(format!(
                "assistantMessageId '{assistant_message_id}' is ambiguous across project chat sessions"
            ))),
        }
    }

    async fn apply_creative_agent_proposal(
        &self,
        params: ApplyCreativeAgentProposalParams<'_>,
    ) -> Result<CreativeAgentProposalCommit, DbError> {
        nomifun_common::UserId::parse(params.owner_id).map_err(|error| {
            DbError::Conflict(format!(
                "creative studio proposal owner_id is not a canonical UUIDv7: {error}"
            ))
        })?;
        nomifun_common::CreativeStudioProjectId::parse(params.project_id).map_err(|error| {
            DbError::Conflict(format!(
                "creative studio proposal project_id is not a canonical UUIDv7: {error}"
            ))
        })?;
        nomifun_common::MessageId::parse(params.assistant_message_id).map_err(|error| {
            DbError::Conflict(format!(
                "creative studio proposal assistant_message_id is not a canonical UUIDv7: {error}"
            ))
        })?;
        if params.expected_revision < 1 {
            return Err(DbError::Conflict(
                "creative studio proposal expected revision must be positive".to_owned(),
            ));
        }
        let applied_revision = params.expected_revision.checked_add(1).ok_or_else(|| {
            DbError::Conflict("creative studio proposal revision overflow".to_owned())
        })?;
        if params.node_count < 0 || params.connection_count < 0 {
            return Err(DbError::Conflict(
                "creative studio proposal graph counts must be non-negative".to_owned(),
            ));
        }
        if params.ops_fingerprint.len() != 64
            || params
                .ops_fingerprint
                .bytes()
                .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
        {
            return Err(DbError::Conflict(
                "creative studio proposal fingerprint must be lowercase SHA-256 hex".to_owned(),
            ));
        }
        for (label, json) in [
            ("ops_json", params.ops_json),
            ("results_json", params.results_json),
        ] {
            let value: Value = serde_json::from_str(json).map_err(|error| {
                DbError::Conflict(format!(
                    "creative studio proposal {label} is invalid JSON: {error}"
                ))
            })?;
            if !value.is_array() {
                return Err(DbError::Conflict(format!(
                    "creative studio proposal {label} must be a JSON array"
                )));
            }
        }

        let mut tx = self.pool.begin().await?;
        // This reversible proof-carrying sentinel write is deliberately the
        // first statement. It takes SQLite's global writer position before any
        // read snapshot, so concurrent proposals (including cross-project
        // reuse of the same assistant UUID) serialize before inspecting the
        // receipt table. First application overwrites the sentinel with `now`;
        // replay restores the original timestamp before commit; every error
        // rolls it back with the transaction.
        let locked_updated_at = sqlx::query_scalar::<_, i64>(
            "UPDATE creative_studio_projects \
             SET updated_at = updated_at + 1 \
             WHERE project_id = ? \
               AND EXISTS ( \
                 SELECT 1 \
                 FROM creative_studio_projects project \
                 CROSS JOIN json_each(project.document_json, '$.chatSessions') session \
                 CROSS JOIN json_each(session.value, '$.messageIds') message \
                 JOIN creative_studio_agent_sessions binding \
                   ON binding.project_id = project.project_id \
                  AND binding.session_id = json_extract(session.value, '$.id') \
                 JOIN installation_identity identity \
                   ON identity.singleton_key = 'installation' \
                  AND identity.owner_user_id = binding.owner_id \
                 JOIN messages persisted \
                   ON persisted.conversation_id = binding.conversation_id \
                  AND persisted.message_id = CAST(message.value AS TEXT) \
                 WHERE project.project_id = ? \
                   AND binding.owner_id = ? \
                   AND CAST(message.key AS INTEGER) % 2 = 1 \
                   AND CAST(message.value AS TEXT) = ? \
                   AND persisted.content = ? \
                   AND persisted.position = 'left' \
                   AND persisted.status = 'finish' \
                   AND persisted.hidden = 0 \
                   AND persisted.type = 'text' \
             ) \
             RETURNING updated_at",
        )
        .bind(params.project_id)
        .bind(params.project_id)
        .bind(params.owner_id)
        .bind(params.assistant_message_id)
        .bind(params.assistant_message_content_json)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(locked_updated_at) = locked_updated_at else {
            let project_exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM creative_studio_projects WHERE project_id = ?)",
            )
            .bind(params.project_id)
            .fetch_one(&mut *tx)
            .await?;
            if !project_exists {
                return Err(DbError::NotFound(format!(
                    "creative studio project '{}' not found",
                    params.project_id
                )));
            }
            let owner_matches: bool = sqlx::query_scalar(
                "SELECT EXISTS( \
                     SELECT 1 FROM installation_identity \
                     WHERE singleton_key = 'installation' AND owner_user_id = ? \
                 )",
            )
            .bind(params.owner_id)
            .fetch_one(&mut *tx)
            .await?;
            if !owner_matches {
                return Err(DbError::Conflict(
                    "Creative Studio proposal requires the installation owner".to_owned(),
                ));
            }
            let current_content = sqlx::query_scalar::<_, String>(
                "SELECT persisted.content \
                 FROM creative_studio_projects project \
                 CROSS JOIN json_each(project.document_json, '$.chatSessions') session \
                 CROSS JOIN json_each(session.value, '$.messageIds') message \
                 JOIN creative_studio_agent_sessions binding \
                   ON binding.project_id = project.project_id \
                  AND binding.session_id = json_extract(session.value, '$.id') \
                  AND binding.owner_id = ? \
                 JOIN messages persisted \
                   ON persisted.conversation_id = binding.conversation_id \
                  AND persisted.message_id = CAST(message.value AS TEXT) \
                 WHERE project.project_id = ? \
                   AND CAST(message.key AS INTEGER) % 2 = 1 \
                   AND CAST(message.value AS TEXT) = ? \
                   AND persisted.position = 'left' \
                   AND persisted.status = 'finish' \
                   AND persisted.hidden = 0 \
                   AND persisted.type = 'text' \
                 LIMIT 1",
            )
            .bind(params.owner_id)
            .bind(params.project_id)
            .bind(params.assistant_message_id)
            .fetch_optional(&mut *tx)
            .await?;
            if current_content.as_deref().is_some_and(|content| {
                content != params.assistant_message_content_json
            }) {
                return Err(DbError::Conflict(format!(
                    "creative studio assistant proposal '{}' source content changed during application",
                    params.assistant_message_id
                )));
            }
            return Err(DbError::Conflict(format!(
                "assistantMessageId '{}' is not a completed visible assistant message in a bound Creative Studio session",
                params.assistant_message_id
            )));
        };

        let receipt = sqlx::query_as::<_, CreativeStudioAgentProposalReceiptRow>(
            "SELECT * FROM creative_studio_agent_proposal_receipts \
             WHERE assistant_message_id = ?",
        )
        .bind(params.assistant_message_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(receipt) = receipt {
            if receipt.project_id != params.project_id {
                return Err(DbError::Conflict(format!(
                    "creative studio assistant proposal '{}' is already owned by another project",
                    params.assistant_message_id
                )));
            }
            if receipt.ops_fingerprint != params.ops_fingerprint
                || receipt.ops_json != params.ops_json
            {
                return Err(DbError::Conflict(format!(
                    "creative studio assistant proposal '{}' payload mismatch",
                    params.assistant_message_id
                )));
            }
            sqlx::query(
                "UPDATE creative_studio_projects SET updated_at = ? WHERE project_id = ?",
            )
            .bind(locked_updated_at - 1)
            .bind(params.project_id)
            .execute(&mut *tx)
            .await?;
            let project = sqlx::query_as::<_, CreativeStudioProjectRow>(
                "SELECT * FROM creative_studio_projects WHERE project_id = ?",
            )
            .bind(params.project_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| {
                DbError::NotFound(format!(
                    "creative studio project '{}' not found",
                    params.project_id
                ))
            })?;
            tx.commit().await?;
            return Ok(CreativeAgentProposalCommit {
                project,
                receipt,
                replayed: true,
            });
        }

        let project = sqlx::query_as::<_, CreativeStudioProjectRow>(
            "UPDATE creative_studio_projects \
             SET document_json = ?, node_count = ?, connection_count = ?, \
                 revision = revision + 1, updated_at = ? \
             WHERE project_id = ? AND revision = ? RETURNING *",
        )
        .bind(params.document_json)
        .bind(params.node_count)
        .bind(params.connection_count)
        .bind(params.now)
        .bind(params.project_id)
        .bind(params.expected_revision)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(project) = project else {
            // Dropping the transaction rolls the proof lock back without a
            // receipt, so a corrected revision can claim this assistant ID.
            return Err(DbError::Conflict(format!(
                "creative studio project '{}' revision conflict",
                params.project_id
            )));
        };
        sqlx::query(
            "INSERT INTO creative_studio_agent_proposal_receipts \
                (project_id, assistant_message_id, ops_fingerprint, ops_json, results_json, \
                 applied_revision, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(params.project_id)
        .bind(params.assistant_message_id)
        .bind(params.ops_fingerprint)
        .bind(params.ops_json)
        .bind(params.results_json)
        .bind(applied_revision)
        .bind(params.now)
        .execute(&mut *tx)
        .await?;
        let receipt = sqlx::query_as::<_, CreativeStudioAgentProposalReceiptRow>(
            "SELECT * FROM creative_studio_agent_proposal_receipts \
             WHERE project_id = ? AND assistant_message_id = ?",
        )
        .bind(params.project_id)
        .bind(params.assistant_message_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(CreativeAgentProposalCommit {
            project,
            receipt,
            replayed: false,
        })
    }

    async fn import_creative_project_with_assets(
        &self,
        project: &CreativeStudioProjectRow,
        assets: &[WorkshopAssetRow],
    ) -> Result<CreativeStudioProjectRow, DbError> {
        nomifun_common::validate_uuidv7(&project.project_id).map_err(|error| {
            DbError::Conflict(format!(
                "imported creative studio project_id {:?} is not a canonical UUIDv7: {error}",
                project.project_id
            ))
        })?;
        if project.revision != 1 || project.node_count < 0 || project.connection_count < 0 {
            return Err(DbError::Conflict(
                "imported creative studio project must start at revision 1 with non-negative counts"
                    .into(),
            ));
        }
        validate_asset_rows(assets)?;
        for asset in assets {
            let references = origin_references(asset.origin.as_deref())?;
            if references.provider_id.is_some()
                || references.project_id.is_some()
                || references.workflow_id.is_some()
                || references.creation_task_id.is_some()
            {
                return Err(DbError::Conflict(format!(
                    "imported creative studio asset {} contains a nonportable durable origin reference",
                    asset.asset_id
                )));
            }
        }

        let mut tx = self.pool.begin().await?;
        for asset in assets {
            sqlx::query(
                "INSERT INTO workshop_assets \
                    (asset_id, kind, title, collection, tags, rel_path, thumb_rel_path, mime, width, height, bytes, \
                     text_content, in_library, origin, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&asset.asset_id)
            .bind(&asset.kind)
            .bind(&asset.title)
            .bind(&asset.collection)
            .bind(&asset.tags)
            .bind(&asset.rel_path)
            .bind(&asset.thumb_rel_path)
            .bind(&asset.mime)
            .bind(asset.width)
            .bind(asset.height)
            .bind(asset.bytes)
            .bind(&asset.text_content)
            .bind(asset.in_library)
            .bind(&asset.origin)
            .bind(asset.created_at)
            .bind(asset.updated_at)
            .execute(&mut *tx)
            .await?;
        }
        let imported = sqlx::query_as::<_, CreativeStudioProjectRow>(
            "INSERT INTO creative_studio_projects \
                (project_id, title, revision, node_count, connection_count, document_json, created_at, updated_at) \
             VALUES (?, ?, 1, ?, ?, ?, ?, ?) RETURNING *",
        )
        .bind(&project.project_id)
        .bind(&project.title)
        .bind(project.node_count)
        .bind(project.connection_count)
        .bind(&project.document_json)
        .bind(project.created_at)
        .bind(project.updated_at)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(imported)
    }

    async fn delete_creative_project(&self, project_id: &str) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        let locked = sqlx::query(
            "UPDATE creative_studio_projects SET updated_at = updated_at WHERE project_id = ?",
        )
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
        if locked.rows_affected() == 0 {
            return Err(DbError::NotFound(format!(
                "creative studio project '{project_id}' not found"
            )));
        }
        let live_task: Option<String> = sqlx::query_scalar(
            "SELECT creation_task_id FROM creation_tasks \
             WHERE project_id = ? AND status IN ('queued', 'running') \
             ORDER BY submitted_at ASC, creation_task_id ASC LIMIT 1",
        )
        .bind(project_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(task_id) = live_task {
            return Err(DbError::Conflict(format!(
                "creative studio project '{project_id}' has live creation task '{task_id}'"
            )));
        }
        sqlx::query(
            "DELETE FROM creative_studio_agent_proposal_receipts WHERE project_id = ?",
        )
        .bind(project_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM creative_studio_projects WHERE project_id = ?")
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    // ---- canonical Creative Studio workflows ----

    async fn list_creative_workflows(&self) -> Result<Vec<CreativeStudioWorkflowRow>, DbError> {
        let rows = sqlx::query_as::<_, CreativeStudioWorkflowRow>(
            "SELECT * FROM creative_studio_workflows ORDER BY updated_at DESC, id DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        for row in &rows {
            nomifun_common::CreativeStudioWorkflowId::parse(&row.workflow_id).map_err(
                |error| {
                    DbError::Conflict(format!(
                        "creative studio workflow_id {:?} is not a canonical UUIDv7: {error}",
                        row.workflow_id
                    ))
                },
            )?;
        }
        Ok(rows)
    }

    async fn get_creative_workflow(
        &self,
        workflow_id: &str,
    ) -> Result<Option<CreativeStudioWorkflowRow>, DbError> {
        let row = sqlx::query_as::<_, CreativeStudioWorkflowRow>(
            "SELECT * FROM creative_studio_workflows WHERE workflow_id = ?",
        )
        .bind(workflow_id)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(row) = &row {
            nomifun_common::CreativeStudioWorkflowId::parse(&row.workflow_id).map_err(
                |error| {
                    DbError::Conflict(format!(
                        "creative studio workflow_id {:?} is not a canonical UUIDv7: {error}",
                        row.workflow_id
                    ))
                },
            )?;
        }
        Ok(row)
    }

    async fn create_creative_workflow(
        &self,
        row: &CreativeStudioWorkflowRow,
    ) -> Result<CreativeStudioWorkflowRow, DbError> {
        nomifun_common::CreativeStudioWorkflowId::parse(&row.workflow_id).map_err(|error| {
            DbError::Conflict(format!(
                "creative studio workflow_id {:?} is not a canonical UUIDv7: {error}",
                row.workflow_id
            ))
        })?;
        if row.revision != 1 {
            return Err(DbError::Conflict(
                "a creative studio workflow must start at revision 1".into(),
            ));
        }
        Ok(sqlx::query_as::<_, CreativeStudioWorkflowRow>(
            "INSERT INTO creative_studio_workflows \
                (workflow_id, revision, name, description, category, visibility, definition_json, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING *",
        )
        .bind(&row.workflow_id)
        .bind(row.revision)
        .bind(&row.name)
        .bind(&row.description)
        .bind(&row.category)
        .bind(&row.visibility)
        .bind(&row.definition_json)
        .bind(row.created_at)
        .bind(row.updated_at)
        .fetch_one(&self.pool)
        .await?)
    }

    async fn save_creative_workflow(
        &self,
        workflow_id: &str,
        expected_revision: i64,
        row: &CreativeStudioWorkflowRow,
    ) -> Result<CreativeStudioWorkflowRow, DbError> {
        if row.workflow_id != workflow_id || row.revision != expected_revision + 1 {
            return Err(DbError::Conflict(
                "creative studio workflow replacement must preserve its ID and increment revision once"
                    .into(),
            ));
        }
        let saved = sqlx::query_as::<_, CreativeStudioWorkflowRow>(
            "UPDATE creative_studio_workflows \
             SET revision = ?, name = ?, description = ?, category = ?, visibility = ?, \
                 definition_json = ?, updated_at = ? \
             WHERE workflow_id = ? AND revision = ? RETURNING *",
        )
        .bind(row.revision)
        .bind(&row.name)
        .bind(&row.description)
        .bind(&row.category)
        .bind(&row.visibility)
        .bind(&row.definition_json)
        .bind(row.updated_at)
        .bind(workflow_id)
        .bind(expected_revision)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(saved) = saved {
            return Ok(saved);
        }
        if self.get_creative_workflow(workflow_id).await?.is_none() {
            return Err(DbError::NotFound(format!(
                "creative studio workflow '{workflow_id}' not found"
            )));
        }
        Err(DbError::Conflict(format!(
            "creative studio workflow '{workflow_id}' revision conflict"
        )))
    }

    async fn delete_creative_workflow(&self, workflow_id: &str) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM creative_studio_workflows WHERE workflow_id = ?")
            .bind(workflow_id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!(
                "creative studio workflow '{workflow_id}' not found"
            )));
        }
        Ok(())
    }

    // ---- canonical Creative Studio workflow runs ----

    async fn list_creative_workflow_runs(
        &self,
        workflow_id: Option<&str>,
    ) -> Result<Vec<CreativeStudioWorkflowRunRow>, DbError> {
        let rows = if let Some(workflow_id) = workflow_id {
            nomifun_common::CreativeStudioWorkflowId::parse(workflow_id).map_err(|error| {
                DbError::Conflict(format!(
                    "creative studio workflow_id {workflow_id:?} is not a canonical UUIDv7: {error}"
                ))
            })?;
            sqlx::query_as::<_, CreativeStudioWorkflowRunRow>(
                "SELECT * FROM creative_studio_workflow_runs \
                 WHERE workflow_id = ? ORDER BY updated_at DESC, id DESC",
            )
            .bind(workflow_id)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, CreativeStudioWorkflowRunRow>(
                "SELECT * FROM creative_studio_workflow_runs ORDER BY updated_at DESC, id DESC",
            )
            .fetch_all(&self.pool)
            .await?
        };
        for row in &rows {
            validate_creative_workflow_run_row_ids(row)?;
        }
        Ok(rows)
    }

    async fn get_creative_workflow_run(
        &self,
        workflow_run_id: &str,
    ) -> Result<Option<CreativeStudioWorkflowRunRow>, DbError> {
        nomifun_common::CreativeStudioWorkflowRunId::parse(workflow_run_id).map_err(|error| {
            DbError::Conflict(format!(
                "creative studio workflow_run_id {workflow_run_id:?} is not a canonical UUIDv7: {error}"
            ))
        })?;
        let row = sqlx::query_as::<_, CreativeStudioWorkflowRunRow>(
            "SELECT * FROM creative_studio_workflow_runs WHERE workflow_run_id = ?",
        )
        .bind(workflow_run_id)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(row) = row.as_ref() {
            validate_creative_workflow_run_row_ids(row)?;
        }
        Ok(row)
    }

    async fn create_creative_workflow_run(
        &self,
        row: &CreativeStudioWorkflowRunRow,
        referenced_asset_ids: &[String],
    ) -> Result<CreativeStudioWorkflowRunRow, DbError> {
        validate_creative_workflow_run_row_ids(row)?;
        if row.revision != 1 {
            return Err(DbError::Conflict(
                "a creative studio workflow run must start at revision 1".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        for asset_id in referenced_asset_ids {
            nomifun_common::WorkshopAssetId::parse(asset_id).map_err(|error| {
                DbError::Conflict(format!(
                    "creative studio workflow run asset_id {asset_id:?} is not a canonical UUIDv7: {error}"
                ))
            })?;
            let locked = sqlx::query(
                "UPDATE workshop_assets SET updated_at = updated_at \
                 WHERE asset_id = ? AND kind = 'image'",
            )
            .bind(asset_id)
            .execute(&mut *tx)
            .await?;
            if locked.rows_affected() == 0 {
                return Err(DbError::Conflict(format!(
                    "creative studio workflow run reference '{asset_id}' is missing or is not an image"
                )));
            }
        }
        sqlx::query(
            "INSERT INTO creative_studio_workflow_runs \
                (workflow_run_id, workflow_id, workflow_revision, revision, status, \
                 step_ids_json, aggregate_json, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(workflow_run_id) DO NOTHING",
        )
        .bind(&row.workflow_run_id)
        .bind(&row.workflow_id)
        .bind(row.workflow_revision)
        .bind(row.revision)
        .bind(&row.status)
        .bind(&row.step_ids_json)
        .bind(&row.aggregate_json)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&mut *tx)
        .await?;
        let persisted = sqlx::query_as::<_, CreativeStudioWorkflowRunRow>(
            "SELECT * FROM creative_studio_workflow_runs WHERE workflow_run_id = ?",
        )
        .bind(&row.workflow_run_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            DbError::Init(format!(
                "creative studio workflow run {} vanished after idempotent insert",
                row.workflow_run_id
            ))
        })?;
        validate_creative_workflow_run_row_ids(&persisted)?;
        tx.commit().await?;
        Ok(persisted)
    }

    async fn save_creative_workflow_run(
        &self,
        workflow_run_id: &str,
        expected_revision: i64,
        row: &CreativeStudioWorkflowRunRow,
    ) -> Result<CreativeStudioWorkflowRunRow, DbError> {
        validate_creative_workflow_run_row_ids(row)?;
        if row.workflow_run_id != workflow_run_id || row.revision != expected_revision + 1 {
            return Err(DbError::Conflict(
                "creative studio workflow run replacement must preserve its ID and increment revision once"
                    .into(),
            ));
        }
        let saved = sqlx::query_as::<_, CreativeStudioWorkflowRunRow>(
            "UPDATE creative_studio_workflow_runs \
             SET revision = ?, status = ?, step_ids_json = ?, aggregate_json = ?, updated_at = ? \
             WHERE workflow_run_id = ? AND revision = ? RETURNING *",
        )
        .bind(row.revision)
        .bind(&row.status)
        .bind(&row.step_ids_json)
        .bind(&row.aggregate_json)
        .bind(row.updated_at)
        .bind(workflow_run_id)
        .bind(expected_revision)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(saved) = saved {
            return Ok(saved);
        }
        if self
            .get_creative_workflow_run(workflow_run_id)
            .await?
            .is_none()
        {
            return Err(DbError::NotFound(format!(
                "creative studio workflow run '{workflow_run_id}' not found"
            )));
        }
        Err(DbError::Conflict(format!(
            "creative studio workflow run '{workflow_run_id}' revision conflict"
        )))
    }

    // ---- assets ----

    async fn create_asset(&self, row: &WorkshopAssetRow) -> Result<WorkshopAssetRow, DbError> {
        validate_asset_row(row)?;
        let references = origin_references(row.origin.as_deref())?;
        let mut tx = self.pool.begin().await?;
        if let Some(provider_id) = references.provider_id {
            let locked = sqlx::query(
                "UPDATE providers SET updated_at = updated_at WHERE provider_id = ?",
            )
            .bind(&provider_id)
            .execute(&mut *tx)
            .await?;
            if locked.rows_affected() == 0 {
                return Err(DbError::Conflict(format!(
                    "workshop asset origin references missing provider '{provider_id}'"
                )));
            }
        }
        if let Some(project_id) = &references.project_id {
            let locked = sqlx::query(
                "UPDATE creative_studio_projects SET updated_at = updated_at WHERE project_id = ?",
            )
            .bind(project_id)
            .execute(&mut *tx)
            .await?;
            if locked.rows_affected() == 0 {
                return Err(DbError::Conflict(format!(
                    "workshop asset origin references missing creative studio project '{project_id}'"
                )));
            }
        }
        if let (Some(workflow_id), Some(workflow_run_id)) =
            (&references.workflow_id, &references.workflow_run_id)
        {
            let workflow = sqlx::query(
                "UPDATE creative_studio_workflows SET updated_at = updated_at WHERE workflow_id = ?",
            )
            .bind(workflow_id)
            .execute(&mut *tx)
            .await?;
            if workflow.rows_affected() == 0 {
                return Err(DbError::Conflict(format!(
                    "workshop asset origin references missing creative studio workflow '{workflow_id}'"
                )));
            }
            let run = sqlx::query(
                "UPDATE creative_studio_workflow_runs SET updated_at = updated_at \
                 WHERE workflow_run_id = ? AND workflow_id = ?",
            )
            .bind(workflow_run_id)
            .bind(workflow_id)
            .execute(&mut *tx)
            .await?;
            if run.rows_affected() == 0 {
                return Err(DbError::Conflict(format!(
                    "workshop asset origin references missing workflow run '{workflow_run_id}' for workflow '{workflow_id}'"
                )));
            }
        }
        if let Some(creation_task_id) = references.creation_task_id {
            let task = sqlx::query_as::<
                _,
                (
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                ),
            >(
                "UPDATE creation_tasks SET status = status WHERE creation_task_id = ? \
                 RETURNING project_id, node_id, workbench_kind, workflow_id, workflow_run_id, workflow_step_id",
            )
            .bind(&creation_task_id)
            .fetch_optional(&mut *tx)
            .await?;
            let task = task.ok_or_else(|| {
                DbError::Conflict(format!(
                    "workshop asset origin references missing creation task '{creation_task_id}'"
                ))
            })?;
            let expected = (
                references.project_id.as_deref(),
                references.node_id.as_deref(),
                references.workbench_kind.as_deref(),
                references.workflow_id.as_deref(),
                references.workflow_run_id.as_deref(),
                references.workflow_step_id.as_deref(),
            );
            let actual = (
                task.0.as_deref(),
                task.1.as_deref(),
                task.2.as_deref(),
                task.3.as_deref(),
                task.4.as_deref(),
                task.5.as_deref(),
            );
            if expected != actual {
                return Err(DbError::Conflict(format!(
                    "workshop asset origin owner does not match creation task '{creation_task_id}'"
                )));
            }
        }
        sqlx::query(
            "INSERT INTO workshop_assets \
                (asset_id, kind, title, collection, tags, rel_path, thumb_rel_path, mime, width, height, bytes, \
                 text_content, in_library, origin, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.asset_id)
        .bind(&row.kind)
        .bind(&row.title)
        .bind(&row.collection)
        .bind(&row.tags)
        .bind(&row.rel_path)
        .bind(&row.thumb_rel_path)
        .bind(&row.mime)
        .bind(row.width)
        .bind(row.height)
        .bind(row.bytes)
        .bind(&row.text_content)
        .bind(row.in_library)
        .bind(&row.origin)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row.clone())
    }

    async fn get_asset(&self, id: &str) -> Result<Option<WorkshopAssetRow>, DbError> {
        let row = sqlx::query_as::<_, WorkshopAssetRow>(
            "SELECT * FROM workshop_assets WHERE asset_id = ?",
        )
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        if let Some(row) = &row {
            validate_asset_row(row)?;
        }
        Ok(row)
    }

    async fn list_all_assets(&self) -> Result<Vec<WorkshopAssetRow>, DbError> {
        let rows = sqlx::query_as::<_, WorkshopAssetRow>("SELECT * FROM workshop_assets")
            .fetch_all(&self.pool)
            .await?;
        validate_asset_rows(&rows)?;
        Ok(rows)
    }

    async fn list_assets(&self, params: ListAssetsParams<'_>) -> Result<(Vec<WorkshopAssetRow>, i64), DbError> {
        // Shared WHERE assembly for both the COUNT and the page query.
        fn push_filters<'a>(qb: &mut QueryBuilder<'a, Sqlite>, p: &ListAssetsParams<'a>) {
            let mut first = true;
            let mut clause = |qb: &mut QueryBuilder<'a, Sqlite>| {
                qb.push(if first { " WHERE " } else { " AND " });
                first = false;
            };
            if let Some(kind) = p.kind {
                clause(qb);
                qb.push("kind = ").push_bind(kind);
            }
            if let Some(collection) = p.collection {
                clause(qb);
                qb.push("collection = ").push_bind(collection);
            }
            if p.ungrouped {
                clause(qb);
                qb.push("(collection IS NULL OR collection = '')");
            }
            if let Some(q) = p.q {
                clause(qb);
                qb.push("LOWER(title) LIKE ").push_bind(format!("%{}%", q.to_lowercase()));
            }
            if let Some(tag) = p.tag {
                clause(qb);
                // Match one entry of the JSON `tags` array (stored as e.g.
                // `["人物","场景"]`) via a case-sensitive substring search for the
                // quoted needle `"tag"`. `instr` (unlike LIKE) is case-sensitive
                // and treats `%`/`_` literally, so no metachar escaping is needed.
                qb.push("instr(tags, ").push_bind(format!("\"{tag}\"")).push(") > 0");
            }
            if let Some(in_library) = p.in_library {
                clause(qb);
                qb.push("in_library = ").push_bind(in_library);
            }
        }

        let mut count_qb: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT COUNT(*) FROM workshop_assets");
        push_filters(&mut count_qb, &params);
        let total: i64 = count_qb.build_query_scalar().fetch_one(&self.pool).await?;

        let page = params.page.max(1);
        let page_size = params.page_size.clamp(1, 200);
        let offset = (page - 1) * page_size;

        let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT * FROM workshop_assets");
        push_filters(&mut qb, &params);
        // ORDER BY is a fixed static clause chosen from a closed enum (never
        // user text), so pushing it verbatim is injection-safe. Every variant
        // carries an `id` tiebreaker for a stable total order.
        qb.push(" ORDER BY ")
            .push(order_by_sql(params.sort))
            .push(" LIMIT ")
            .push_bind(page_size)
            .push(" OFFSET ")
            .push_bind(offset);
        let items = qb.build_query_as::<WorkshopAssetRow>().fetch_all(&self.pool).await?;
        validate_asset_rows(&items)?;

        Ok((items, total))
    }

    async fn update_asset(
        &self,
        id: &str,
        params: UpdateAssetParams<'_>,
        now: i64,
    ) -> Result<WorkshopAssetRow, DbError> {
        let existing = self
            .get_asset(id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("workshop asset '{id}' not found")))?;

        let title = params.title.unwrap_or(&existing.title).to_string();
        let collection = match params.collection {
            Some(c) => c.map(str::to_string),
            None => existing.collection.clone(),
        };
        let tags = params.tags.unwrap_or(&existing.tags).to_string();
        let in_library = params.in_library.unwrap_or(existing.in_library);

        sqlx::query(
            "UPDATE workshop_assets SET title = ?, collection = ?, tags = ?, in_library = ?, updated_at = ? \
             WHERE asset_id = ?",
        )
        .bind(&title)
        .bind(&collection)
        .bind(&tags)
        .bind(in_library)
        .bind(now)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(WorkshopAssetRow {
            title,
            collection,
            tags,
            in_library,
            updated_at: now,
            ..existing
        })
    }

    async fn set_asset_thumb(&self, id: &str, thumb_rel_path: &str, now: i64) -> Result<(), DbError> {
        let result = sqlx::query(
            "UPDATE workshop_assets SET thumb_rel_path = ?, updated_at = ? WHERE asset_id = ?",
        )
            .bind(thumb_rel_path)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("workshop asset '{id}' not found")));
        }
        Ok(())
    }

    async fn delete_asset(&self, id: &str) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        let locked = sqlx::query(
            "UPDATE workshop_assets SET updated_at = updated_at WHERE asset_id = ?",
        )
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if locked.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("workshop asset '{id}' not found")));
        }
        let workflow_runs: Vec<(String, String)> = sqlx::query_as(
            "SELECT workflow_run_id, aggregate_json FROM creative_studio_workflow_runs",
        )
        .fetch_all(&mut *tx)
        .await?;
        for (workflow_run_id, aggregate_json) in workflow_runs {
            if workflow_run_json_references_asset(&aggregate_json, id)? {
                return Err(DbError::Conflict(format!(
                    "workshop asset '{id}' is referenced by creative studio workflow run '{workflow_run_id}'"
                )));
            }
        }
        let referencing_task: Option<(String, String)> = sqlx::query_as(
            "SELECT creation_task_id, \
                    CASE WHEN EXISTS ( \
                        SELECT 1 FROM json_each(input_bindings) input \
                        WHERE json_extract(input.value, '$.asset_id') = ?1 \
                    ) THEN 'input' ELSE 'result' END AS reference_kind \
             FROM creation_tasks \
             WHERE EXISTS ( \
                 SELECT 1 FROM json_each(input_bindings) input \
                 WHERE json_extract(input.value, '$.asset_id') = ?1 \
             ) OR EXISTS ( \
                 SELECT 1 FROM json_each(result_asset_ids) result \
                 WHERE result.value = ?1 \
             ) \
             ORDER BY creation_task_id ASC LIMIT 1",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((task_id, reference_kind)) = referencing_task {
            return Err(DbError::Conflict(format!(
                "workshop asset '{id}' is referenced as a creation task {reference_kind} by '{task_id}'"
            )));
        }
        sqlx::query("DELETE FROM workshop_assets WHERE asset_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn rename_collection(&self, from: &str, to: Option<&str>, now: i64) -> Result<u64, DbError> {
        let result = sqlx::query("UPDATE workshop_assets SET collection = ?, updated_at = ? WHERE collection = ?")
            .bind(to)
            .bind(now)
            .bind(from)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;

    const ASSET_1: &str = "0190f5fe-7c00-7a00-8abc-000000000101";
    const ASSET_2: &str = "0190f5fe-7c00-7a00-8abc-000000000102";
    const ASSET_3: &str = "0190f5fe-7c00-7a00-8abc-000000000103";
    const ASSET_NULL: &str = "0190f5fe-7c00-7a00-8abc-000000000111";
    const ASSET_EMPTY: &str = "0190f5fe-7c00-7a00-8abc-000000000112";
    const ASSET_GRP: &str = "0190f5fe-7c00-7a00-8abc-000000000113";
    const ASSET_X: &str = "0190f5fe-7c00-7a00-8abc-000000000121";
    const ASSET_T: &str = "0190f5fe-7c00-7a00-8abc-000000000131";
    const ASSET_TA: &str = "0190f5fe-7c00-7a00-8abc-000000000141";
    const ASSET_TB: &str = "0190f5fe-7c00-7a00-8abc-000000000142";
    const ASSET_S1: &str = "0190f5fe-7c00-7a00-8abc-000000000151";
    const ASSET_S2: &str = "0190f5fe-7c00-7a00-8abc-000000000152";
    const ASSET_S3: &str = "0190f5fe-7c00-7a00-8abc-000000000153";
    const ASSET_C1: &str = "0190f5fe-7c00-7a00-8abc-000000000161";
    const ASSET_C2: &str = "0190f5fe-7c00-7a00-8abc-000000000162";
    const ASSET_C3: &str = "0190f5fe-7c00-7a00-8abc-000000000163";
    const CREATIVE_PROJECT_A: &str = "0190f5fe-7c00-7a00-8abc-000000000171";
    const CREATIVE_WORKFLOW_A: &str = "0190f5fe-7c00-7a00-8abc-000000000172";
    const CREATIVE_WORKFLOW_RUN_A: &str = "0190f5fe-7c00-7a00-8abc-000000000173";
    const CREATIVE_WORKFLOW_STEP_A: &str = "0190f5fe-7c00-7a00-8abc-000000000174";

    async fn repo() -> (SqliteWorkshopRepository, crate::Database) {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteWorkshopRepository::new(db.pool().clone());
        (repo, db)
    }

    async fn seed_agent_proposal_project(
        repo: &SqliteWorkshopRepository,
        db: &crate::Database,
        project_id: &str,
        assistant_message_ids: &[&str],
    ) -> (String, Vec<String>) {
        let owner_id = crate::installation_owner_id(db.pool()).await.unwrap();
        let session_id = nomifun_common::generate_id();
        let conversation_id = nomifun_common::ConversationId::new().into_string();
        let mut message_ids = Vec::with_capacity(assistant_message_ids.len() * 2);
        for assistant_message_id in assistant_message_ids {
            message_ids.push(nomifun_common::MessageId::new().into_string());
            message_ids.push((*assistant_message_id).to_owned());
        }
        let initial_doc = serde_json::json!({
            "schema": "nomifun.creative-studio/v1",
            "projectId": project_id,
            "nodes": [],
            "chatSessions": [{
                "id": session_id,
                "messageIds": message_ids
            }]
        })
        .to_string();
        repo.create_creative_project(project_id, "Agent proposal", &initial_doc, 100)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO conversations \
                (conversation_id, user_id, name, type, extra, status, source, created_at, updated_at) \
             VALUES (?, ?, 'Creative Studio Agent', 'nomi', '{}', 'finished', 'nomifun', 1, 1)",
        )
        .bind(&conversation_id)
        .bind(&owner_id)
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO creative_studio_agent_sessions \
                (owner_id, project_id, session_id, conversation_id, created_at, updated_at) \
             VALUES (?, ?, ?, ?, 1, 1)",
        )
        .bind(&owner_id)
        .bind(project_id)
        .bind(&session_id)
        .bind(&conversation_id)
        .execute(db.pool())
        .await
        .unwrap();
        let artifact_text = "```json\n{\"kind\":\"nomifun.creative-studio.canvas-ops/v1\",\"summary\":\"Add durable text\",\"ops\":[{\"type\":\"add_node\",\"node_type\":\"text\",\"x\":0,\"y\":0,\"data\":{\"text\":\"durable\",\"format\":\"plain\",\"fontSize\":16,\"textAlign\":\"left\"}}]}\n```";
        let mut assistant_contents = Vec::with_capacity(assistant_message_ids.len());
        for assistant_message_id in assistant_message_ids {
            let content_json = serde_json::json!({ "content": artifact_text }).to_string();
            sqlx::query(
                "INSERT INTO messages \
                    (message_id, conversation_id, msg_id, type, content, position, status, hidden, created_at) \
                 VALUES (?, ?, ?, 'text', ?, 'left', 'finish', 0, 1)",
            )
            .bind(assistant_message_id)
            .bind(&conversation_id)
            .bind(assistant_message_id)
            .bind(&content_json)
            .execute(db.pool())
            .await
            .unwrap();
            assistant_contents.push(content_json);
        }
        (initial_doc, assistant_contents)
    }

    fn sample_asset(id: i64, asset_id: &str, kind: &str, title: &str) -> WorkshopAssetRow {
        WorkshopAssetRow {
            id,
            asset_id: asset_id.to_string(),
            kind: kind.to_string(),
            title: title.to_string(),
            collection: None,
            tags: "[]".to_string(),
            rel_path: Some(format!("workshop/assets/{asset_id}.png")),
            thumb_rel_path: None,
            mime: Some("image/png".to_string()),
            width: Some(10),
            height: Some(20),
            bytes: Some(123),
            text_content: None,
            in_library: true,
            origin: None,
            created_at: 1000,
            updated_at: 1000,
        }
    }

    #[tokio::test]
    async fn creative_project_crud_and_revision_compare_and_swap() {
        let (repo, _db) = repo().await;
        let initial_doc = format!(
            r#"{{"schema":"nomifun.creative-studio/v1","projectId":"{CREATIVE_PROJECT_A}","nodes":[]}}"#
        );
        let created = repo
            .create_creative_project(CREATIVE_PROJECT_A, "新项目", &initial_doc, 100)
            .await
            .unwrap();
        assert_eq!(created.revision, 1);
        assert_eq!(created.node_count, 0);
        assert_eq!(created.connection_count, 0);

        let renamed = repo
            .rename_creative_project(CREATIVE_PROJECT_A, "重命名", 110)
            .await
            .unwrap();
        assert_eq!(renamed.title, "重命名");
        assert_eq!(renamed.revision, 1, "metadata rename must not invalidate autosave");

        let changed_doc = format!(
            r#"{{"schema":"nomifun.creative-studio/v1","projectId":"{CREATIVE_PROJECT_A}","nodes":[{{}}]}}"#
        );
        let saved = repo
            .save_creative_project(CREATIVE_PROJECT_A, 1, &changed_doc, 1, 2, 120)
            .await
            .unwrap();
        assert_eq!(saved.revision, 2);
        assert_eq!(saved.node_count, 1);
        assert_eq!(saved.connection_count, 2);

        let stale = repo
            .save_creative_project(CREATIVE_PROJECT_A, 1, &initial_doc, 0, 0, 130)
            .await
            .unwrap_err();
        assert!(matches!(stale, DbError::Conflict(_)));
        assert_eq!(
            repo.get_creative_project(CREATIVE_PROJECT_A)
                .await
                .unwrap()
                .unwrap()
                .document_json,
            changed_doc,
            "a stale writer must not replace the canonical document"
        );

        repo.delete_creative_project(CREATIVE_PROJECT_A)
            .await
            .unwrap();
        assert!(repo
            .get_creative_project(CREATIVE_PROJECT_A)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn creative_agent_proposal_receipt_replays_and_rolls_back_failed_cas() {
        const ASSISTANT_A: &str = "0190f5fe-7c00-7a00-8abc-000000000181";
        const ASSISTANT_B: &str = "0190f5fe-7c00-7a00-8abc-000000000182";
        const ASSISTANT_C: &str = "0190f5fe-7c00-7a00-8abc-000000000184";
        let (repo, db) = repo().await;
        let owner_id = crate::installation_owner_id(db.pool()).await.unwrap();
        let (initial, assistant_contents) = seed_agent_proposal_project(
            &repo,
            &db,
            CREATIVE_PROJECT_A,
            &[ASSISTANT_A, ASSISTANT_B, ASSISTANT_C],
        )
        .await;
        let mut changed: Value = serde_json::from_str(&initial).unwrap();
        changed["nodes"] = serde_json::json!([{}]);
        let changed_doc = changed.to_string();
        let first = repo
            .apply_creative_agent_proposal(ApplyCreativeAgentProposalParams {
                owner_id: &owner_id,
                project_id: CREATIVE_PROJECT_A,
                assistant_message_id: ASSISTANT_A,
                assistant_message_content_json: &assistant_contents[0],
                ops_fingerprint: &"a".repeat(64),
                ops_json: r#"[{"type":"add_node"}]"#,
                results_json: r#"[{"type":"node_added","node_id":"winner"}]"#,
                expected_revision: 1,
                document_json: &changed_doc,
                node_count: 1,
                connection_count: 0,
                now: 200,
            })
            .await
            .unwrap();
        assert!(!first.replayed);
        assert_eq!(first.project.revision, 2);
        assert_eq!(first.receipt.applied_revision, 2);

        let replay = repo
            .apply_creative_agent_proposal(ApplyCreativeAgentProposalParams {
                owner_id: &owner_id,
                project_id: CREATIVE_PROJECT_A,
                assistant_message_id: ASSISTANT_A,
                assistant_message_content_json: &assistant_contents[0],
                ops_fingerprint: &"a".repeat(64),
                ops_json: r#"[{"type":"add_node"}]"#,
                results_json: r#"[{"type":"node_added","node_id":"loser"}]"#,
                expected_revision: 999,
                document_json: &changed_doc,
                node_count: 99,
                connection_count: 0,
                now: 300,
            })
            .await
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.project.revision, 2);
        assert_eq!(replay.project.updated_at, first.project.updated_at);
        assert_eq!(replay.receipt.results_json, first.receipt.results_json);

        let mismatch = repo
            .apply_creative_agent_proposal(ApplyCreativeAgentProposalParams {
                owner_id: &owner_id,
                project_id: CREATIVE_PROJECT_A,
                assistant_message_id: ASSISTANT_A,
                assistant_message_content_json: &assistant_contents[0],
                ops_fingerprint: &"b".repeat(64),
                ops_json: r#"[{"type":"move_node"}]"#,
                results_json: "[]",
                expected_revision: 2,
                document_json: &changed_doc,
                node_count: 1,
                connection_count: 0,
                now: 400,
            })
            .await
            .unwrap_err();
        assert!(matches!(mismatch, DbError::Conflict(message) if message.contains("payload mismatch")));

        sqlx::query("UPDATE messages SET content = ? WHERE message_id = ?")
            .bind(r#"{"content":"changed after provenance read"}"#)
            .bind(ASSISTANT_B)
            .execute(db.pool())
            .await
            .unwrap();
        let source_race = repo
            .apply_creative_agent_proposal(ApplyCreativeAgentProposalParams {
                owner_id: &owner_id,
                project_id: CREATIVE_PROJECT_A,
                assistant_message_id: ASSISTANT_B,
                assistant_message_content_json: &assistant_contents[1],
                ops_fingerprint: &"c".repeat(64),
                ops_json: r#"[{"type":"add_node"}]"#,
                results_json: r#"[{"type":"node_added","node_id":"never"}]"#,
                expected_revision: 2,
                document_json: &changed_doc,
                node_count: 2,
                connection_count: 0,
                now: 450,
            })
            .await
            .unwrap_err();
        assert!(matches!(source_race, DbError::Conflict(message) if message.contains("source content changed")));
        assert!(repo
            .get_creative_agent_proposal_receipt(&owner_id, CREATIVE_PROJECT_A, ASSISTANT_B)
            .await
            .unwrap()
            .is_none());

        let stale = repo
            .apply_creative_agent_proposal(ApplyCreativeAgentProposalParams {
                owner_id: &owner_id,
                project_id: CREATIVE_PROJECT_A,
                assistant_message_id: ASSISTANT_C,
                assistant_message_content_json: &assistant_contents[2],
                ops_fingerprint: &"d".repeat(64),
                ops_json: r#"[{"type":"add_node"}]"#,
                results_json: r#"[{"type":"node_added","node_id":"never"}]"#,
                expected_revision: 1,
                document_json: &changed_doc,
                node_count: 2,
                connection_count: 0,
                now: 500,
            })
            .await
            .unwrap_err();
        assert!(matches!(stale, DbError::Conflict(message) if message.contains("revision conflict")));
        assert!(repo
            .get_creative_agent_proposal_receipt(&owner_id, CREATIVE_PROJECT_A, ASSISTANT_C)
            .await
            .unwrap()
            .is_none());
        let current = repo
            .get_creative_project(CREATIVE_PROJECT_A)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.revision, 2);
        assert_eq!(current.node_count, 1);
        repo.delete_creative_project(CREATIVE_PROJECT_A)
            .await
            .unwrap();
        let remaining_receipts: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM creative_studio_agent_proposal_receipts WHERE project_id = ?",
        )
        .bind(CREATIVE_PROJECT_A)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(remaining_receipts, 0, "project deletion must cascade receipts");
    }

    #[tokio::test]
    async fn concurrent_creative_agent_proposal_claims_execute_once() {
        const ASSISTANT: &str = "0190f5fe-7c00-7a00-8abc-000000000183";
        let dir = tempfile::tempdir().unwrap();
        let db = crate::init_database(&dir.path().join("proposal-concurrency.db"))
            .await
            .unwrap();
        let repo = SqliteWorkshopRepository::new(db.pool().clone());
        let owner_id = crate::installation_owner_id(db.pool()).await.unwrap();
        let (initial, assistant_contents) =
            seed_agent_proposal_project(&repo, &db, CREATIVE_PROJECT_A, &[ASSISTANT]).await;
        let mut winner: Value = serde_json::from_str(&initial).unwrap();
        winner["nodes"] = serde_json::json!([{"candidate": "a"}]);
        let winner_doc = winner.to_string();
        let mut loser: Value = serde_json::from_str(&initial).unwrap();
        loser["nodes"] = serde_json::json!([{"candidate": "b"}]);
        let loser_doc = loser.to_string();
        let repo_a = repo.clone();
        let repo_b = repo.clone();
        let call_a = async {
            repo_a
                .apply_creative_agent_proposal(ApplyCreativeAgentProposalParams {
                    owner_id: &owner_id,
                    project_id: CREATIVE_PROJECT_A,
                    assistant_message_id: ASSISTANT,
                    assistant_message_content_json: &assistant_contents[0],
                    ops_fingerprint: &"d".repeat(64),
                    ops_json: r#"[{"type":"add_node"}]"#,
                    results_json: r#"[{"type":"node_added","node_id":"a"}]"#,
                    expected_revision: 1,
                    document_json: &winner_doc,
                    node_count: 1,
                    connection_count: 0,
                    now: 200,
                })
                .await
        };
        let call_b = async {
            repo_b
                .apply_creative_agent_proposal(ApplyCreativeAgentProposalParams {
                    owner_id: &owner_id,
                    project_id: CREATIVE_PROJECT_A,
                    assistant_message_id: ASSISTANT,
                    assistant_message_content_json: &assistant_contents[0],
                    ops_fingerprint: &"d".repeat(64),
                    ops_json: r#"[{"type":"add_node"}]"#,
                    results_json: r#"[{"type":"node_added","node_id":"b"}]"#,
                    expected_revision: 1,
                    document_json: &loser_doc,
                    node_count: 1,
                    connection_count: 0,
                    now: 201,
                })
                .await
        };
        let (a, b) = tokio::join!(call_a, call_b);
        let a = a.unwrap();
        let b = b.unwrap();
        assert_ne!(a.replayed, b.replayed);
        assert_eq!(a.receipt.results_json, b.receipt.results_json);
        let current = repo
            .get_creative_project(CREATIVE_PROJECT_A)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.revision, 2);
        assert_eq!(current.node_count, 1);
    }

    #[tokio::test]
    async fn creative_workflow_crud_and_revision_compare_and_swap() {
        let (repo, _db) = repo().await;
        let definition = format!(
            r#"{{"id":"{CREATIVE_WORKFLOW_A}","revision":1,"metadata":{{"name":"海报","description":"","category":"电商","visibility":"private","tags":[],"createdAt":100,"updatedAt":100}},"output":{{"kind":"single-image"}},"variables":[],"templates":[],"steps":[]}}"#
        );
        let created = repo
            .create_creative_workflow(&CreativeStudioWorkflowRow {
                id: 0,
                workflow_id: CREATIVE_WORKFLOW_A.into(),
                revision: 1,
                name: "海报".into(),
                description: String::new(),
                category: "电商".into(),
                visibility: "private".into(),
                definition_json: definition,
                created_at: 100,
                updated_at: 100,
            })
            .await
            .unwrap();
        assert_eq!(created.revision, 1);

        let changed = CreativeStudioWorkflowRow {
            id: created.id,
            workflow_id: CREATIVE_WORKFLOW_A.into(),
            revision: 2,
            name: "海报 2".into(),
            description: "更新".into(),
            category: "营销".into(),
            visibility: "public".into(),
            definition_json: format!(
                r#"{{"id":"{CREATIVE_WORKFLOW_A}","revision":2,"metadata":{{"name":"海报 2"}}}}"#
            ),
            created_at: 100,
            updated_at: 200,
        };
        let saved = repo
            .save_creative_workflow(CREATIVE_WORKFLOW_A, 1, &changed)
            .await
            .unwrap();
        assert_eq!(saved.revision, 2);
        assert_eq!(saved.name, "海报 2");

        let stale = repo
            .save_creative_workflow(CREATIVE_WORKFLOW_A, 1, &changed)
            .await
            .unwrap_err();
        assert!(matches!(stale, DbError::Conflict(_)));
        assert_eq!(repo.list_creative_workflows().await.unwrap().len(), 1);

        repo.delete_creative_workflow(CREATIVE_WORKFLOW_A)
            .await
            .unwrap();
        assert!(repo
            .get_creative_workflow(CREATIVE_WORKFLOW_A)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn creative_workflow_run_crud_filter_and_revision_compare_and_swap() {
        let (repo, _db) = repo().await;
        let aggregate = |revision: i64, status: &str| {
            serde_json::json!({
                "kind": "nomifun.creative-studio.workflow-run",
                "version": 1,
                "revision": revision,
                "workflowSnapshot": {
                    "id": CREATIVE_WORKFLOW_A,
                    "revision": 3
                },
                "request": {
                    "id": CREATIVE_WORKFLOW_RUN_A,
                    "workflowId": CREATIVE_WORKFLOW_A,
                    "workflowRevision": 3,
                    "referenceAssetIds": [ASSET_1]
                },
                "record": {
                    "requestId": CREATIVE_WORKFLOW_RUN_A,
                    "workflowId": CREATIVE_WORKFLOW_A,
                    "status": status
                }
            })
            .to_string()
        };
        let requested_row = CreativeStudioWorkflowRunRow {
            id: 0,
            workflow_run_id: CREATIVE_WORKFLOW_RUN_A.into(),
            workflow_id: CREATIVE_WORKFLOW_A.into(),
            workflow_revision: 3,
            revision: 1,
            status: "requested".into(),
            step_ids_json: serde_json::to_string(&[CREATIVE_WORKFLOW_STEP_A]).unwrap(),
            aggregate_json: aggregate(1, "requested"),
            created_at: 100,
            updated_at: 100,
        };
        let missing_asset = repo
            .create_creative_workflow_run(&requested_row, &[ASSET_1.into()])
            .await
            .unwrap_err();
        assert!(matches!(missing_asset, DbError::Conflict(_)));
        repo.create_asset(&sample_asset(0, ASSET_1, "image", "run input"))
            .await
            .unwrap();
        let created = repo
            .create_creative_workflow_run(&requested_row, &[ASSET_1.into()])
            .await
            .unwrap();
        assert_eq!(created.revision, 1);
        assert!(matches!(
            repo.delete_asset(ASSET_1).await,
            Err(DbError::Conflict(message)) if message.contains(CREATIVE_WORKFLOW_RUN_A)
        ));
        assert_eq!(
            repo.list_creative_workflow_runs(Some(CREATIVE_WORKFLOW_A))
                .await
                .unwrap()
                .len(),
            1
        );

        let replacement = CreativeStudioWorkflowRunRow {
            revision: 2,
            status: "queued".into(),
            aggregate_json: aggregate(2, "queued"),
            updated_at: 110,
            ..created
        };
        let saved = repo
            .save_creative_workflow_run(CREATIVE_WORKFLOW_RUN_A, 1, &replacement)
            .await
            .unwrap();
        assert_eq!(saved.revision, 2);
        assert_eq!(saved.status, "queued");

        let stale = repo
            .save_creative_workflow_run(CREATIVE_WORKFLOW_RUN_A, 1, &replacement)
            .await
            .unwrap_err();
        assert!(matches!(stale, DbError::Conflict(_)));
        let missing = repo
            .save_creative_workflow_run(
                "0190f5fe-7c00-7a00-8abc-000000000175",
                1,
                &CreativeStudioWorkflowRunRow {
                    workflow_run_id: "0190f5fe-7c00-7a00-8abc-000000000175".into(),
                    ..replacement
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(missing, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn creative_archive_import_rolls_back_project_and_assets_together() {
        let (repo, _db) = repo().await;
        let document_json = format!(
            r#"{{"schema":"nomifun.creative-studio/v1","projectId":"{CREATIVE_PROJECT_A}","nodes":[]}}"#
        );
        let project = CreativeStudioProjectRow {
            id: 0,
            project_id: CREATIVE_PROJECT_A.into(),
            title: "原子导入".into(),
            revision: 1,
            node_count: 0,
            connection_count: 0,
            document_json,
            created_at: 100,
            updated_at: 100,
        };
        let first = sample_asset(0, ASSET_1, "image", "first");
        let duplicate = sample_asset(0, ASSET_1, "image", "duplicate");

        let error = repo
            .import_creative_project_with_assets(&project, &[first, duplicate])
            .await
            .unwrap_err();
        assert!(matches!(error, DbError::Query(_)));
        assert!(
            repo.get_creative_project(CREATIVE_PROJECT_A)
                .await
                .unwrap()
                .is_none(),
            "project insert must roll back with the asset failure"
        );
        assert!(
            repo.get_asset(ASSET_1).await.unwrap().is_none(),
            "the first asset insert must also roll back"
        );
    }

    #[tokio::test]
    async fn asset_crud_and_filters() {
        let (repo, _db) = repo().await;
        repo.create_asset(&sample_asset(1, ASSET_1, "image", "红色卖点图")).await.unwrap();
        repo.create_asset(&sample_asset(2, ASSET_2, "video", "宣传视频")).await.unwrap();
        let mut text = sample_asset(3, ASSET_3, "text", "描述");
        text.rel_path = None;
        text.in_library = false;
        repo.create_asset(&text).await.unwrap();

        // no filter → all 3
        let (items, total) = repo
            .list_assets(ListAssetsParams { page: 1, page_size: 50, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(total, 3);
        assert_eq!(items.len(), 3);

        // kind filter
        let (items, total) = repo
            .list_assets(ListAssetsParams { kind: Some("image"), page: 1, page_size: 50, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(items[0].id, 1);
        assert_eq!(items[0].asset_id, ASSET_1);

        // in_library filter
        let (_, total) = repo
            .list_assets(ListAssetsParams { in_library: Some(false), page: 1, page_size: 50, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(total, 1);

        // substring q filter (case-insensitive)
        let (_, total) = repo
            .list_assets(ListAssetsParams { q: Some("视频"), page: 1, page_size: 50, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(total, 1);

        // pagination: page 1 size 2 → 2 of 3
        let (items, total) = repo
            .list_assets(ListAssetsParams { page: 1, page_size: 2, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(total, 3);
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn asset_origin_requires_exact_canonical_task_owner() {
        let (repo, db) = repo().await;
        let provider_id = nomifun_common::generate_id();
        sqlx::query(
            "INSERT INTO providers \
             (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, created_at, updated_at) \
             VALUES (?, 'test', 'origin provider', 'https://example.invalid', 'bearer', '', 1, 1)",
        )
        .bind(&provider_id)
        .execute(db.pool())
        .await
        .unwrap();
        let creation_task_id = nomifun_common::generate_id();
        let project_id = nomifun_common::generate_id();
        let node_id = nomifun_common::generate_id();
        let document = serde_json::json!({
            "schema": "nomifun.creative-studio/v1",
            "projectId": project_id,
            "nodes": []
        });
        sqlx::query(
            "INSERT INTO creative_studio_projects \
             (project_id, title, revision, node_count, connection_count, document_json, created_at, updated_at) \
             VALUES (?, 'Asset Origin Project', 1, 0, 0, ?, 1, 1)",
        )
        .bind(&project_id)
        .bind(document.to_string())
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO creation_tasks \
             (creation_task_id, project_id, node_id, provider_id, model, capability, params, status, \
              submitted_at, request_fingerprint) \
             VALUES (?, ?, ?, ?, 'model', 'image', '{}', 'succeeded', 1, \
              '{\"asset_origin_fixture\":true}')",
        )
        .bind(&creation_task_id)
        .bind(&project_id)
        .bind(&node_id)
        .bind(&provider_id)
        .execute(db.pool())
        .await
        .unwrap();

        let mut valid = sample_asset(1, ASSET_1, "image", "business origin");
        valid.origin = Some(
            serde_json::json!({
                "project_id": project_id,
                "node_id": node_id,
                "creation_task_id": creation_task_id.clone()
            })
            .to_string(),
        );
        let created = repo.create_asset(&valid).await.unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(created.origin.as_deref().unwrap())
                .unwrap()["creation_task_id"],
            creation_task_id
        );

        let standalone_task_id = nomifun_common::generate_id();
        sqlx::query(
            "INSERT INTO creation_tasks \
             (creation_task_id, project_id, workbench_kind, provider_id, model, capability, params, \
              input_bindings, status, submitted_at, request_fingerprint) \
             VALUES (?, ?, 'video', ?, 'model', 't2v', '{}', '[]', 'running', 1, \
              '{\"asset_origin_fixture\":\"standalone\"}')",
        )
        .bind(&standalone_task_id)
        .bind(&project_id)
        .bind(&provider_id)
        .execute(db.pool())
        .await
        .unwrap();
        let standalone_asset_id = nomifun_common::generate_id();
        let mut standalone = sample_asset(
            2,
            &standalone_asset_id,
            "video",
            "standalone task owner",
        );
        standalone.origin = Some(
            serde_json::json!({
                "project_id": project_id,
                "workbench_kind": "video",
                "creation_task_id": standalone_task_id
            })
            .to_string(),
        );
        let standalone_created = repo.create_asset(&standalone).await.unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(standalone_created.origin.as_deref().unwrap())
                .unwrap()["workbench_kind"],
            "video"
        );

        let mut wrong_owner = sample_asset(2, ASSET_2, "image", "wrong task owner");
        wrong_owner.origin = Some(
            serde_json::json!({ "creation_task_id": creation_task_id.clone() }).to_string(),
        );
        assert!(
            matches!(
                repo.create_asset(&wrong_owner).await,
                Err(DbError::Conflict(message))
                    if message.contains("owner does not match creation task")
            ),
            "origin ownership must exactly match the referenced task"
        );

        let mut missing_parent = sample_asset(2, ASSET_2, "image", "missing task");
        missing_parent.origin = Some(
            serde_json::json!({ "creation_task_id": nomifun_common::generate_id() })
                .to_string(),
        );
        assert!(
            matches!(
                repo.create_asset(&missing_parent).await,
                Err(DbError::Conflict(message))
                    if message.contains("references missing creation task")
            ),
            "origin.creation_task_id must resolve through creation_tasks.creation_task_id"
        );

        for (label, origin) in [
            ("unsupported task_id integer", serde_json::json!({ "task_id": 1 })),
            (
                "unsupported task_id numeric string",
                serde_json::json!({ "task_id": "1" }),
            ),
            (
                "unsupported task_id UUIDv7",
                serde_json::json!({ "task_id": nomifun_common::generate_id() }),
            ),
            (
                "explicit null canvas_id",
                serde_json::json!({ "canvas_id": null }),
            ),
            (
                "explicit null node_id",
                serde_json::json!({ "node_id": null }),
            ),
            (
                "camel-case canvasId",
                serde_json::json!({ "canvasId": nomifun_common::generate_id() }),
            ),
            (
                "integer creation_task_id",
                serde_json::json!({ "creation_task_id": 1 }),
            ),
            (
                "numeric-string creation_task_id",
                serde_json::json!({ "creation_task_id": "1" }),
            ),
            (
                "prefixed creation_task_id",
                serde_json::json!({
                    "creation_task_id": format!("task_{}", nomifun_common::generate_id())
                }),
            ),
            (
                "uuidv4 creation_task_id",
                serde_json::json!({
                    "creation_task_id": "550e8400-e29b-41d4-a716-446655440000"
                }),
            ),
            (
                "uppercase creation_task_id",
                serde_json::json!({
                    "creation_task_id": nomifun_common::generate_id().to_ascii_uppercase()
                }),
            ),
        ] {
            let invalid_asset_id = nomifun_common::generate_id();
            let mut invalid = sample_asset(2, &invalid_asset_id, "image", label);
            invalid.origin = Some(origin.to_string());
            assert!(
                repo.create_asset(&invalid).await.is_err(),
                "{label} must be rejected"
            );
        }
    }

    #[tokio::test]
    async fn list_assets_ungrouped_filter() {
        let (repo, _db) = repo().await;
        // Two ungrouped (NULL and empty-string collection) + one grouped.
        repo.create_asset(&sample_asset(1, ASSET_NULL, "image", "no collection")).await.unwrap();
        let mut empty = sample_asset(2, ASSET_EMPTY, "image", "empty collection");
        empty.collection = Some(String::new());
        repo.create_asset(&empty).await.unwrap();
        let mut grouped = sample_asset(3, ASSET_GRP, "image", "grouped");
        grouped.collection = Some("角色".to_string());
        repo.create_asset(&grouped).await.unwrap();

        // ungrouped=true → the NULL + empty-string rows only.
        let (items, total) = repo
            .list_assets(ListAssetsParams { ungrouped: true, page: 1, page_size: 50, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(total, 2);
        let ids: std::collections::BTreeSet<&str> = items.iter().map(|a| a.asset_id.as_str()).collect();
        assert!(ids.contains(ASSET_NULL) && ids.contains(ASSET_EMPTY));
        assert!(!ids.contains(ASSET_GRP));

        // named collection filter still returns only the grouped row.
        let (_, total) = repo
            .list_assets(ListAssetsParams { collection: Some("角色"), page: 1, page_size: 50, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(total, 1);

        // ungrouped composes with other filters (kind).
        let (_, total) = repo
            .list_assets(ListAssetsParams {
                ungrouped: true,
                kind: Some("image"),
                page: 1,
                page_size: 50,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(total, 2);
    }

    #[tokio::test]
    async fn asset_update_partial_and_delete() {
        let (repo, _db) = repo().await;
        repo.create_asset(&sample_asset(1, ASSET_X, "image", "old")).await.unwrap();
        let updated = repo
            .update_asset(
                ASSET_X,
                UpdateAssetParams {
                    title: Some("new"),
                    collection: Some(Some("角色")),
                    in_library: Some(false),
                    ..Default::default()
                },
                2000,
            )
            .await
            .unwrap();
        assert_eq!(updated.title, "new");
        assert_eq!(updated.collection.as_deref(), Some("角色"));
        assert!(!updated.in_library);
        assert_eq!(updated.updated_at, 2000);
        // unchanged field preserved
        assert_eq!(updated.mime.as_deref(), Some("image/png"));

        repo.delete_asset(ASSET_X).await.unwrap();
        assert!(repo.get_asset(ASSET_X).await.unwrap().is_none());
        assert!(matches!(repo.delete_asset(ASSET_X).await.unwrap_err(), DbError::NotFound(_)));
        assert!(matches!(
            repo.update_asset("nope", UpdateAssetParams::default(), 1).await.unwrap_err(),
            DbError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn task_input_and_result_assets_remain_restricted_after_retirement() {
        let (repo, db) = repo().await;
        repo.create_creative_project(
            CREATIVE_PROJECT_A,
            "task assets",
            &format!(
                r#"{{"schema":"nomifun.creative-studio/v1","projectId":"{CREATIVE_PROJECT_A}","nodes":[]}}"#
            ),
            1,
        )
        .await
        .unwrap();
        repo.create_asset(&sample_asset(1, ASSET_1, "image", "input"))
            .await
            .unwrap();
        repo.create_asset(&sample_asset(2, ASSET_2, "image", "result"))
            .await
            .unwrap();
        let provider_id = nomifun_common::generate_id();
        sqlx::query(
            "INSERT INTO providers \
             (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, created_at, updated_at) \
             VALUES (?, 'test', 'retire provider', 'https://example.invalid', 'bearer', '', 1, 1)",
        )
        .bind(&provider_id)
        .execute(db.pool())
        .await
        .unwrap();
        let task_id = nomifun_common::generate_id();
        let inputs = serde_json::json!([{
            "asset_id": ASSET_1,
            "kind": "image",
            "role": "reference"
        }]);
        sqlx::query(
            "INSERT INTO creation_tasks \
             (creation_task_id, project_id, workbench_kind, provider_id, model, capability, params, \
              input_bindings, status, result_asset_ids, submitted_at, finished_at, deleted_at, request_fingerprint) \
             VALUES (?, ?, 'image', ?, 'model', 'i2i', '{}', ?, 'succeeded', ?, 1, 2, 3, '{}')",
        )
        .bind(&task_id)
        .bind(CREATIVE_PROJECT_A)
        .bind(&provider_id)
        .bind(inputs.to_string())
        .bind(serde_json::to_string(&[ASSET_2]).unwrap())
        .execute(db.pool())
        .await
        .unwrap();

        for (asset_id, kind) in [(ASSET_1, "input"), (ASSET_2, "result")] {
            assert!(matches!(
                repo.delete_asset(asset_id).await,
                Err(DbError::Conflict(message)) if message.contains(kind) && message.contains(&task_id)
            ));
        }
        assert!(
            sqlx::query("DELETE FROM workshop_assets WHERE asset_id = ?")
                .bind(ASSET_2)
                .execute(db.pool())
                .await
                .is_err(),
            "database trigger is the final task-result deletion guard"
        );
    }

    #[tokio::test]
    async fn project_delete_rejects_live_tasks_but_keeps_terminal_history() {
        let (repo, db) = repo().await;
        repo.create_creative_project(
            CREATIVE_PROJECT_A,
            "live task owner",
            &format!(
                r#"{{"schema":"nomifun.creative-studio/v1","projectId":"{CREATIVE_PROJECT_A}","nodes":[]}}"#
            ),
            1,
        )
        .await
        .unwrap();
        let provider_id = nomifun_common::generate_id();
        sqlx::query(
            "INSERT INTO providers \
             (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, created_at, updated_at) \
             VALUES (?, 'test', 'project gate provider', 'https://example.invalid', 'bearer', '', 1, 1)",
        )
        .bind(&provider_id)
        .execute(db.pool())
        .await
        .unwrap();
        let task_id = nomifun_common::generate_id();
        sqlx::query(
            "INSERT INTO creation_tasks \
             (creation_task_id, project_id, workbench_kind, provider_id, model, capability, params, \
              input_bindings, status, submitted_at, request_fingerprint) \
             VALUES (?, ?, 'video', ?, 'model', 't2v', '{}', '[]', 'queued', 1, '{}')",
        )
        .bind(&task_id)
        .bind(CREATIVE_PROJECT_A)
        .bind(&provider_id)
        .execute(db.pool())
        .await
        .unwrap();
        assert!(matches!(
            repo.delete_creative_project(CREATIVE_PROJECT_A).await,
            Err(DbError::Conflict(message)) if message.contains("live creation task")
        ));
        sqlx::query(
            "UPDATE creation_tasks SET status = 'failed', finished_at = 2, deleted_at = 3 \
             WHERE creation_task_id = ?",
        )
        .bind(&task_id)
        .execute(db.pool())
        .await
        .unwrap();
        repo.delete_creative_project(CREATIVE_PROJECT_A)
            .await
            .unwrap();
        let retained: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM creation_tasks WHERE creation_task_id = ? AND deleted_at = 3",
        )
        .bind(&task_id)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(retained, 1, "terminal tombstone survives project deletion");
    }

    #[tokio::test]
    async fn set_asset_thumb() {
        let (repo, _db) = repo().await;
        repo.create_asset(&sample_asset(1, ASSET_T, "image", "img")).await.unwrap();
        let thumb = format!("workshop/assets/thumbs/{ASSET_T}.jpg");
        repo.set_asset_thumb(ASSET_T, &thumb, 6).await.unwrap();
        let a = repo.get_asset(ASSET_T).await.unwrap().unwrap();
        assert_eq!(a.thumb_rel_path.as_deref(), Some(thumb.as_str()));
        assert!(matches!(repo.set_asset_thumb("nope", "x", 1).await.unwrap_err(), DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn list_all_assets_returns_every_row() {
        let (repo, _db) = repo().await;
        repo.create_asset(&sample_asset(1, ASSET_1, "image", "a")).await.unwrap();
        let mut internal = sample_asset(2, ASSET_2, "image", "b");
        internal.in_library = false;
        repo.create_asset(&internal).await.unwrap();
        let all = repo.list_all_assets().await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn list_assets_tag_filter_exact_match() {
        let (repo, _db) = repo().await;
        let mut a = sample_asset(1, ASSET_TA, "image", "带标签");
        a.tags = r#"["人物","场景"]"#.to_string();
        repo.create_asset(&a).await.unwrap();
        let mut b = sample_asset(2, ASSET_TB, "image", "另一个");
        b.tags = r#"["场景"]"#.to_string();
        repo.create_asset(&b).await.unwrap();

        // "人物" → only the asset with ASSET_TA
        let (items, total) = repo
            .list_assets(ListAssetsParams { tag: Some("人物"), page: 1, page_size: 50, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(items[0].id, 1);
        assert_eq!(items[0].asset_id, ASSET_TA);

        // "场景" → both
        let (_, total) = repo
            .list_assets(ListAssetsParams { tag: Some("场景"), page: 1, page_size: 50, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(total, 2);

        // exact match: a partial "人" must NOT match "人物"
        let (_, total) = repo
            .list_assets(ListAssetsParams { tag: Some("人"), page: 1, page_size: 50, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn list_assets_sort_variants() {
        let (repo, _db) = repo().await;
        let mut a = sample_asset(1, ASSET_S1, "image", "Banana");
        (a.created_at, a.updated_at, a.bytes) = (100, 400, Some(50));
        repo.create_asset(&a).await.unwrap();
        let mut b = sample_asset(2, ASSET_S2, "image", "apple");
        (b.created_at, b.updated_at, b.bytes) = (200, 300, Some(999));
        repo.create_asset(&b).await.unwrap();
        let mut c = sample_asset(3, ASSET_S3, "image", "Cherry");
        (c.created_at, c.updated_at, c.bytes) = (300, 100, Some(10));
        repo.create_asset(&c).await.unwrap();

        let ids = |items: &[WorkshopAssetRow]| items.iter().map(|r| r.asset_id.clone()).collect::<Vec<_>>();
        let list = |sort: AssetSort| ListAssetsParams { sort, page: 1, page_size: 50, ..Default::default() };

        let (items, _) = repo.list_assets(list(AssetSort::CreatedDesc)).await.unwrap();
        assert_eq!(ids(&items), [ASSET_S3, ASSET_S2, ASSET_S1]);
        let (items, _) = repo.list_assets(list(AssetSort::CreatedAsc)).await.unwrap();
        assert_eq!(ids(&items), [ASSET_S1, ASSET_S2, ASSET_S3]);
        let (items, _) = repo.list_assets(list(AssetSort::UpdatedDesc)).await.unwrap();
        assert_eq!(ids(&items), [ASSET_S1, ASSET_S2, ASSET_S3]); // updated 400,300,100
        let (items, _) = repo.list_assets(list(AssetSort::TitleAsc)).await.unwrap();
        assert_eq!(ids(&items), [ASSET_S2, ASSET_S1, ASSET_S3]); // apple,Banana,Cherry (NOCASE)
        let (items, _) = repo.list_assets(list(AssetSort::SizeDesc)).await.unwrap();
        assert_eq!(ids(&items), [ASSET_S2, ASSET_S1, ASSET_S3]); // 999,50,10
    }

    #[tokio::test]
    async fn rename_collection_bulk_and_ungroup() {
        let (repo, _db) = repo().await;
        for (id, asset_id, coll) in [
            (1, ASSET_C1, "旧集合"),
            (2, ASSET_C2, "旧集合"),
            (3, ASSET_C3, "其他"),
        ] {
            let mut row = sample_asset(id, asset_id, "image", asset_id);
            row.collection = Some(coll.to_string());
            repo.create_asset(&row).await.unwrap();
        }

        // rename 旧集合 → 新集合 (2 rows)
        let updated = repo.rename_collection("旧集合", Some("新集合"), 5000).await.unwrap();
        assert_eq!(updated, 2);
        let (_, total) = repo
            .list_assets(ListAssetsParams { collection: Some("新集合"), page: 1, page_size: 50, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(total, 2);

        // ungroup 其他 (to = None → NULL)
        let updated = repo.rename_collection("其他", None, 6000).await.unwrap();
        assert_eq!(updated, 1);
        let (_, total) = repo
            .list_assets(ListAssetsParams { ungrouped: true, page: 1, page_size: 50, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(total, 1);

        // no match → 0 rows updated
        let updated = repo.rename_collection("不存在", Some("x"), 7000).await.unwrap();
        assert_eq!(updated, 0);
    }
}
