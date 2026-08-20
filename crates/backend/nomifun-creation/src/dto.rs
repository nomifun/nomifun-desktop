//! Wire DTOs for the legacy `/api/creation/tasks` and canonical
//! `/api/creative-studio/tasks` surfaces. Both remain snake_case on the wire.

use nomifun_common::{
    AppError, CreationTaskId, CreativeStudioProjectId, CreativeStudioWorkflowId,
    CreativeStudioWorkflowRunId, CreativeStudioWorkflowStepId, ProviderId,
    TimestampMs, WorkshopAssetId, WorkshopCanvasId, WorkshopNodeId,
};
use nomifun_db::CreationTaskRow;
use serde::Serialize;
use serde_json::Value;

#[cfg(test)]
use nomifun_common::generate_id;

/// A generation task as seen over the wire.
#[derive(Debug, Clone, Serialize)]
pub struct CreationTask {
    pub creation_task_id: String,
    pub project_id: Option<String>,
    pub workflow_id: Option<String>,
    pub workflow_run_id: Option<String>,
    pub workflow_step_id: Option<String>,
    pub canvas_id: Option<String>,
    pub node_id: Option<String>,
    pub provider_id: String,
    pub model: String,
    pub capability: String,
    pub params: Value,
    pub status: String,
    pub error: Option<Value>,
    pub result_asset_ids: Vec<String>,
    pub attempt: i64,
    pub submitted_at: TimestampMs,
    pub started_at: Option<TimestampMs>,
    pub finished_at: Option<TimestampMs>,
}

impl TryFrom<CreationTaskRow> for CreationTask {
    type Error = AppError;

    fn try_from(row: CreationTaskRow) -> Result<Self, Self::Error> {
        CreationTaskId::parse(&row.creation_task_id)
            .map_err(|error| corrupt_id("creation_tasks.creation_task_id", error))?;
        if let Some(id) = row.project_id.as_deref() {
            CreativeStudioProjectId::parse(id)
                .map_err(|error| corrupt_id("creation_tasks.project_id", error))?;
        }
        if let Some(id) = row.workflow_id.as_deref() {
            CreativeStudioWorkflowId::parse(id)
                .map_err(|error| corrupt_id("creation_tasks.workflow_id", error))?;
        }
        if let Some(id) = row.workflow_run_id.as_deref() {
            CreativeStudioWorkflowRunId::parse(id)
                .map_err(|error| corrupt_id("creation_tasks.workflow_run_id", error))?;
        }
        if let Some(id) = row.workflow_step_id.as_deref() {
            CreativeStudioWorkflowStepId::parse(id)
                .map_err(|error| corrupt_id("creation_tasks.workflow_step_id", error))?;
        }
        if let Some(id) = row.canvas_id.as_deref() {
            WorkshopCanvasId::parse(id).map_err(|error| corrupt_id("creation_tasks.canvas_id", error))?;
        }
        if let Some(id) = row.node_id.as_deref() {
            WorkshopNodeId::parse(id).map_err(|error| corrupt_id("creation_tasks.node_id", error))?;
        }
        ProviderId::parse(&row.provider_id).map_err(|error| corrupt_id("creation_tasks.provider_id", error))?;

        let params = serde_json::from_str::<Value>(&row.params)
            .map_err(|error| AppError::Internal(format!("invalid creation_tasks.params JSON: {error}")))?;
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

        Ok(Self {
            creation_task_id: row.creation_task_id,
            project_id: row.project_id,
            workflow_id: row.workflow_id,
            workflow_run_id: row.workflow_run_id,
            workflow_step_id: row.workflow_step_id,
            canvas_id: row.canvas_id,
            node_id: row.node_id,
            provider_id: row.provider_id,
            model: row.model,
            capability: row.capability,
            params,
            status: row.status,
            error,
            result_asset_ids,
            attempt: row.attempt,
            submitted_at: row.submitted_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
        })
    }
}

/// Tagged owner emitted only by `/api/creative-studio/tasks`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CreativeCreationTaskOwner {
    CanvasNode {
        project_id: String,
        node_id: String,
    },
    WorkflowStep {
        workflow_id: String,
        workflow_run_id: String,
        workflow_step_id: String,
    },
}

/// Canonical Creative Studio wire task. Legacy canvas/global columns are not
/// nullable fields here: a response has exactly one tagged owner or fails.
#[derive(Debug, Clone, Serialize)]
pub struct CreativeCreationTask {
    pub creation_task_id: String,
    pub owner: CreativeCreationTaskOwner,
    pub provider_id: String,
    pub model: String,
    pub capability: String,
    pub params: Value,
    pub status: String,
    pub error: Option<Value>,
    pub result_asset_ids: Vec<String>,
    pub attempt: i64,
    pub submitted_at: TimestampMs,
    pub started_at: Option<TimestampMs>,
    pub finished_at: Option<TimestampMs>,
}

impl TryFrom<CreationTask> for CreativeCreationTask {
    type Error = AppError;

    fn try_from(task: CreationTask) -> Result<Self, Self::Error> {
        let owner = match (
            task.project_id,
            task.workflow_id,
            task.workflow_run_id,
            task.workflow_step_id,
            task.canvas_id,
            task.node_id,
        ) {
            (Some(project_id), None, None, None, None, Some(node_id)) => {
                CreativeCreationTaskOwner::CanvasNode {
                    project_id,
                    node_id,
                }
            }
            (None, Some(workflow_id), Some(workflow_run_id), Some(workflow_step_id), None, None) => {
                CreativeCreationTaskOwner::WorkflowStep {
                    workflow_id,
                    workflow_run_id,
                    workflow_step_id,
                }
            }
            _ => {
                return Err(AppError::Internal(format!(
                    "creation task {} does not have one canonical Creative Studio owner",
                    task.creation_task_id
                )));
            }
        };
        Ok(Self {
            creation_task_id: task.creation_task_id,
            owner,
            provider_id: task.provider_id,
            model: task.model,
            capability: task.capability,
            params: task.params,
            status: task.status,
            error: task.error,
            result_asset_ids: task.result_asset_ids,
            attempt: task.attempt,
            submitted_at: task.submitted_at,
            started_at: task.started_at,
            finished_at: task.finished_at,
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
        let canvas_id = WorkshopCanvasId::new().into_string();
        let provider_id = ProviderId::new().into_string();
        let asset_id = WorkshopAssetId::new().into_string();
        let row = CreationTaskRow {
            creation_task_id: creation_task_id.clone(),
            project_id: None,
            workflow_id: None,
            workflow_run_id: None,
            workflow_step_id: None,
            canvas_id: Some(canvas_id),
            node_id: None,
            provider_id,
            model: "m".into(),
            capability: "t2i".into(),
            params: r#"{"prompt":"cat"}"#.into(),
            status: "failed".into(),
            error: Some(r#"{"kind":"adapter_unavailable","message":"x"}"#.into()),
            result_asset_ids: serde_json::to_string(&[&asset_id]).unwrap(),
            remote_task_id: None,
            attempt: 0,
            submitted_at: 1,
            started_at: None,
            finished_at: Some(2),
        };
        let dto = CreationTask::try_from(row).unwrap();
        assert_eq!(dto.params["prompt"], "cat");
        assert_eq!(dto.creation_task_id, creation_task_id);
        assert_eq!(dto.error.as_ref().unwrap()["kind"], "adapter_unavailable");
        assert_eq!(dto.result_asset_ids, vec![asset_id]);
        assert_eq!(dto.finished_at, Some(2));

        let wire = serde_json::to_value(&dto).unwrap();
        assert_eq!(wire["creation_task_id"], dto.creation_task_id.as_str());
        assert!(wire.get("task_id").is_none());
    }

    #[test]
    fn succeeded_without_artifacts_fails_closed() {
        let row = CreationTaskRow {
            creation_task_id: generate_id(),
            project_id: None,
            workflow_id: None,
            workflow_run_id: None,
            workflow_step_id: None,
            canvas_id: None,
            node_id: None,
            provider_id: ProviderId::new().into_string(),
            model: "m".into(),
            capability: "t2i".into(),
            params: "{}".into(),
            status: "succeeded".into(),
            error: None,
            result_asset_ids: "[]".into(),
            remote_task_id: None,
            attempt: 0,
            submitted_at: 1,
            started_at: Some(1),
            finished_at: Some(2),
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
                creation_task_id: creation_task_id.into(),
                project_id: None,
                workflow_id: None,
                workflow_run_id: None,
                workflow_step_id: None,
                canvas_id: None,
                node_id: None,
                provider_id: ProviderId::new().into_string(),
                model: "m".into(),
                capability: "t2i".into(),
                params: "{}".into(),
                status: "failed".into(),
                error: None,
                result_asset_ids: "[]".into(),
                remote_task_id: None,
                attempt: 0,
                submitted_at: 1,
                started_at: None,
                finished_at: Some(2),
            };
            assert!(matches!(CreationTask::try_from(row), Err(AppError::Internal(_))));
        }
    }
}
