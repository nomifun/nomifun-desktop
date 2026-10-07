//! Canonical Creative Studio creation-task DTOs. Wire fields remain snake_case.

use nomifun_common::{
    AppError, CreationTaskId, CreativeStudioCanvasId, CreativeStudioNodeId,
    CreativeStudioTemplateId, CreativeStudioTemplateRunId, CreativeStudioTemplateStepId,
    ProviderId, TimestampMs, WorkshopAssetId,
};
use nomifun_db::CreationTaskRow;
use serde::Serialize;
use serde_json::Value;

use crate::types::CreationInput;

#[cfg(test)]
use nomifun_common::generate_id;

/// Canonical persisted task state used by the service and tagged wire adapter.
#[derive(Debug, Clone, Serialize)]
pub struct CreationTask {
    pub conversation_id: Option<String>,
    pub message_id: Option<String>,
    pub creation_task_id: String,
    pub canvas_id: Option<String>,

    pub template_id: Option<String>,
    pub template_run_id: Option<String>,
    pub template_step_id: Option<String>,
    pub node_id: Option<String>,
    pub provider_id: String,
    pub model: String,
    pub capability: String,
    pub params: Value,
    pub inputs: Option<Vec<CreationInput>>,
    pub status: String,
    pub error: Option<Value>,
    pub result_asset_ids: Vec<String>,
    pub attempt: i64,
    pub submitted_at: TimestampMs,
    pub started_at: Option<TimestampMs>,
    pub finished_at: Option<TimestampMs>,
    pub deleted_at: Option<TimestampMs>,
}

impl TryFrom<CreationTaskRow> for CreationTask {
    type Error = AppError;

    fn try_from(row: CreationTaskRow) -> Result<Self, Self::Error> {
        CreationTaskId::parse(&row.creation_task_id)
            .map_err(|error| corrupt_id("creation_tasks.creation_task_id", error))?;
        if let Some(id) = row.project_id.as_deref() {
            CreativeStudioCanvasId::parse(id)
                .map_err(|error| corrupt_id("creation_tasks.canvas_id", error))?;
        }
        if let Some(id) = row.template_id.as_deref() {
            CreativeStudioTemplateId::parse(id)
                .map_err(|error| corrupt_id("creation_tasks.template_id", error))?;
        }
        if let Some(id) = row.template_run_id.as_deref() {
            CreativeStudioTemplateRunId::parse(id)
                .map_err(|error| corrupt_id("creation_tasks.template_run_id", error))?;
        }
        if let Some(id) = row.template_step_id.as_deref() {
            CreativeStudioTemplateStepId::parse(id)
                .map_err(|error| corrupt_id("creation_tasks.template_step_id", error))?;
        }
        if let Some(id) = row.node_id.as_deref() {
            CreativeStudioNodeId::parse(id)
                .map_err(|error| corrupt_id("creation_tasks.node_id", error))?;
        }
        ProviderId::parse(&row.provider_id).map_err(|error| corrupt_id("creation_tasks.provider_id", error))?;

        let params = serde_json::from_str::<Value>(&row.params)
            .map_err(|error| AppError::Internal(format!("invalid creation_tasks.params JSON: {error}")))?;
        let inputs = row
            .input_bindings
            .as_deref()
            .map(serde_json::from_str::<Vec<CreationInput>>)
            .transpose()
            .map_err(|error| {
                AppError::Internal(format!(
                    "invalid creation_tasks.input_bindings JSON: {error}"
                ))
            })?;
        let error = row
            .error
            .as_deref()
            .map(serde_json::from_str::<Value>)
            .transpose()
            .map_err(|error| AppError::Internal(format!("invalid creation_tasks.error JSON: {error}")))?;
        let result_asset_ids = serde_json::from_str::<Vec<String>>(&row.result_asset_ids)
            .map_err(|error| AppError::Internal(format!("invalid creation_tasks.result_asset_ids JSON: {error}")))?;
        for id in &result_asset_ids {
            WorkshopAssetId::parse(id)
                .map_err(|error| corrupt_id("creation_tasks.result_asset_ids[]", error))?;
        }
        if row.status == "succeeded" && result_asset_ids.is_empty() {
            return Err(AppError::Internal(format!(
                "managed creation task {} is succeeded without result artifacts",
                row.creation_task_id
            )));
        }
        let conversation_owner = row.conversation_id.is_some() && row.message_id.is_some();
        if row.conversation_id.is_some() != row.message_id.is_some() {
            return Err(AppError::Internal("creation task has an incomplete conversation owner".into()));
        }
        if conversation_owner {
            nomifun_common::ConversationId::parse(row.conversation_id.as_deref().unwrap()).map_err(|e| corrupt_id("conversation_id", e))?;
            nomifun_common::MessageId::parse(row.message_id.as_deref().unwrap()).map_err(|e| corrupt_id("message_id", e))?;
            if row.project_id.is_some() || row.node_id.is_some() || row.template_id.is_some() || row.template_run_id.is_some() || row.template_step_id.is_some() {
                return Err(AppError::Internal("creation task has multiple owners".into()));
            }
        } else { match (
            row.project_id.as_ref(),
            row.template_id.as_ref(),
            row.template_run_id.as_ref(),
            row.template_step_id.as_ref(),
            row.node_id.as_ref(),
        ) {
            (Some(_), None, None, None, Some(_))
            | (None, Some(_), Some(_), Some(_), None) => {}
            _ => {
                return Err(AppError::Internal(format!(
                    "creation task {} does not have one canonical Creative Studio owner",
                    row.creation_task_id
                )));
            }
        }
        }
        if row.deleted_at.is_some_and(|deleted_at| {
            deleted_at < row.submitted_at
                || !conversation_owner
                || !matches!(row.status.as_str(), "failed" | "canceled" | "succeeded")
        }) {
            return Err(AppError::Internal(format!(
                "creation task {} has an invalid retirement tombstone",
                row.creation_task_id
            )));
        }

        Ok(Self {
            conversation_id: row.conversation_id,
            message_id: row.message_id,
            creation_task_id: row.creation_task_id,
            // The repository stores the canvas owner in its project_id column.
            canvas_id: row.project_id,
            template_id: row.template_id,
            template_run_id: row.template_run_id,
            template_step_id: row.template_step_id,
            node_id: row.node_id,
            provider_id: row.provider_id,
            model: row.model,
            capability: row.capability,
            params,
            inputs,
            status: row.status,
            error,
            result_asset_ids,
            attempt: row.attempt,
            submitted_at: row.submitted_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
            deleted_at: row.deleted_at,
        })
    }
}

/// Tagged owner emitted only by `/api/creative-studio/tasks`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CreativeCreationTaskOwner {
    ConversationTurn { conversation_id: String, message_id: String },
    CanvasNode {
        canvas_id: String,
        node_id: String,
    },
    TemplateStep {
        template_id: String,
        template_run_id: String,
        template_step_id: String,
    },
}

/// Canonical Creative Studio wire task. A response has exactly one tagged
/// owner or fails closed.
#[derive(Debug, Clone, Serialize)]
pub struct CreativeCreationTask {
    pub creation_task_id: String,
    pub owner: CreativeCreationTaskOwner,
    pub provider_id: String,
    pub model: String,
    pub capability: String,
    pub params: Value,
    /// `null` is an explicit legacy-unprovable snapshot, never an inferred
    /// empty input list.
    pub inputs: Option<Vec<CreationInput>>,
    pub status: String,
    pub error: Option<Value>,
    pub result_asset_ids: Vec<String>,
    pub attempt: i64,
    pub submitted_at: TimestampMs,
    pub started_at: Option<TimestampMs>,
    pub finished_at: Option<TimestampMs>,
    pub deleted_at: Option<TimestampMs>,
}

impl TryFrom<CreationTask> for CreativeCreationTask {
    type Error = AppError;

    fn try_from(task: CreationTask) -> Result<Self, Self::Error> {
        let owner = if let (Some(conversation_id), Some(message_id)) = (task.conversation_id, task.message_id) {
            CreativeCreationTaskOwner::ConversationTurn { conversation_id, message_id }
        } else { match (
            task.canvas_id,
            task.template_id,
            task.template_run_id,
            task.template_step_id,
            task.node_id,
        ) {
            (Some(canvas_id), None, None, None, Some(node_id)) => {
                CreativeCreationTaskOwner::CanvasNode {
                    canvas_id,
                    node_id,
                }
            }
            (None, Some(template_id), Some(template_run_id), Some(template_step_id), None) => {
                CreativeCreationTaskOwner::TemplateStep {
                    template_id,
                    template_run_id,
                    template_step_id,
                }
            }
            _ => {
                return Err(AppError::Internal(format!(
                    "creation task {} does not have one canonical Creative Studio owner",
                    task.creation_task_id
                )));
            }
        }};
        let mut params = task.params;
        if let Some(object) = params.as_object_mut() {
            object.retain(|key, _| !key.starts_with("_nomifun"));
        }
        Ok(Self {
            creation_task_id: task.creation_task_id,
            owner,
            provider_id: task.provider_id,
            model: task.model,
            capability: task.capability,
            params,
            inputs: task.inputs,
            status: task.status,
            error: task.error,
            result_asset_ids: task.result_asset_ids,
            attempt: task.attempt,
            submitted_at: task.submitted_at,
            started_at: task.started_at,
            finished_at: task.finished_at,
            deleted_at: task.deleted_at,
        })
    }
}

fn corrupt_id(field: &str, error: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("invalid canonical ID in {field}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_dto_parses_json_columns() {
        let creation_task_id = generate_id();
        let canvas_id = CreativeStudioCanvasId::new().into_string();
        let node_id = CreativeStudioNodeId::new().into_string();
        let provider_id = ProviderId::new().into_string();
        let asset_id = WorkshopAssetId::new().into_string();
        let row = CreationTaskRow {
            conversation_id: None,
            message_id: None,
            creation_task_id: creation_task_id.clone(),
            project_id: Some(canvas_id.clone()),

            template_id: None,
            template_run_id: None,
            template_step_id: None,
            node_id: Some(node_id),
            provider_id,
            model: "m".into(),
            capability: "t2i".into(),
            params: r#"{"prompt":"cat"}"#.into(),
            input_bindings: Some("[]".into()),
            status: "failed".into(),
            error: Some(r#"{"kind":"adapter_unavailable","message":"x"}"#.into()),
            result_asset_ids: serde_json::to_string(&[&asset_id]).unwrap(),
            remote_task_id: None,
            attempt: 0,
            submitted_at: 1,
            started_at: None,
            finished_at: Some(2),
            deleted_at: None,
        };
        let dto = CreationTask::try_from(row).unwrap();
        assert_eq!(dto.params["prompt"], "cat");
        assert_eq!(dto.creation_task_id, creation_task_id);
        assert_eq!(dto.canvas_id.as_deref(), Some(canvas_id.as_str()));
        assert_eq!(dto.error.as_ref().unwrap()["kind"], "adapter_unavailable");
        assert_eq!(dto.result_asset_ids, vec![asset_id]);
        assert_eq!(dto.finished_at, Some(2));

        let wire = serde_json::to_value(&dto).unwrap();
        assert_eq!(wire["creation_task_id"], dto.creation_task_id.as_str());
        assert_eq!(wire["canvas_id"], canvas_id);
        assert!(wire.get("project_id").is_none());
        assert!(wire.get("task_id").is_none());

        let canonical = serde_json::to_value(CreativeCreationTask::try_from(dto).unwrap()).unwrap();
        assert_eq!(canonical["owner"]["kind"], "canvas_node");
        assert_eq!(canonical["owner"]["canvas_id"], canvas_id);
        assert!(canonical["owner"].get("project_id").is_none());
    }


    #[test]
    fn succeeded_without_artifacts_fails_closed() {
        let row = CreationTaskRow {
            conversation_id: None,
            message_id: None,
            creation_task_id: generate_id(),
            project_id: Some(CreativeStudioCanvasId::new().into_string()),

            template_id: None,
            template_run_id: None,
            template_step_id: None,
            node_id: Some(CreativeStudioNodeId::new().into_string()),
            provider_id: ProviderId::new().into_string(),
            model: "m".into(),
            capability: "t2i".into(),
            params: "{}".into(),
            input_bindings: Some("[]".into()),
            status: "succeeded".into(),
            error: None,
            result_asset_ids: "[]".into(),
            remote_task_id: None,
            attempt: 0,
            submitted_at: 1,
            started_at: Some(1),
            finished_at: Some(2),
            deleted_at: None,
        };
        assert!(matches!(
            CreationTask::try_from(row),
            Err(AppError::Internal(message)) if message.contains("without result artifacts")
        ));
    }

    #[test]
    fn task_dto_rejects_non_uuidv7_business_ids() {
        for creation_task_id in [
            "1",
            "task_0190f5fe-7c00-7a00-8000-000000000001",
            "0190f5fe-7c00-4a00-8000-000000000001",
            "0190F5FE-7C00-7A00-8000-000000000001",
            "0190f5fe7c007a008000000000000001",
            "0190f5fe-7c00-7a00-8000-000000000001 ",
        ] {
            let row = CreationTaskRow {
                conversation_id: None,
                message_id: None,
                creation_task_id: creation_task_id.into(),
                project_id: Some(CreativeStudioCanvasId::new().into_string()),

                template_id: None,
                template_run_id: None,
                template_step_id: None,
                node_id: Some(CreativeStudioNodeId::new().into_string()),
                provider_id: ProviderId::new().into_string(),
                model: "m".into(),
                capability: "t2i".into(),
                params: "{}".into(),
                input_bindings: Some("[]".into()),
                status: "failed".into(),
                error: None,
                result_asset_ids: "[]".into(),
                remote_task_id: None,
                attempt: 0,
                submitted_at: 1,
                started_at: None,
                finished_at: Some(2),
                deleted_at: None,
            };
            assert!(matches!(CreationTask::try_from(row), Err(AppError::Internal(_))));
        }
    }
}
