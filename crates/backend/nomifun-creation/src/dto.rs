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
    pub creation_task_id: String,
    pub canvas_id: Option<String>,
    pub workbench_kind: Option<String>,
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
        match (
            row.project_id.as_ref(),
            row.workbench_kind.as_deref(),
            row.template_id.as_ref(),
            row.template_run_id.as_ref(),
            row.template_step_id.as_ref(),
            row.node_id.as_ref(),
        ) {
            (Some(_), None, None, None, None, Some(_))
            | (_, Some("image" | "video" | "audio"), None, None, None, None)
            | (None, None, Some(_), Some(_), Some(_), None) => {}
            _ => {
                return Err(AppError::Internal(format!(
                    "creation task {} does not have one canonical Creative Studio owner",
                    row.creation_task_id
                )));
            }
        }
        if row.deleted_at.is_some_and(|deleted_at| {
            deleted_at < row.submitted_at
                || row.workbench_kind.is_none()
                || !matches!(row.status.as_str(), "failed" | "canceled" | "succeeded")
        }) {
            return Err(AppError::Internal(format!(
                "creation task {} has an invalid retirement tombstone",
                row.creation_task_id
            )));
        }

        Ok(Self {
            creation_task_id: row.creation_task_id,
            // The repository stores the canvas owner in its project_id column.
            canvas_id: row.project_id,
            workbench_kind: row.workbench_kind,
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
    CanvasNode {
        canvas_id: String,
        node_id: String,
    },
    StandaloneWorkbench {
        workbench_kind: String,
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

#[derive(Debug, Clone, Serialize)]
pub struct CreativeCreationTaskPage {
    pub items: Vec<CreativeCreationTask>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreativeCreationTaskRetireResult {
    pub retired_task_ids: Vec<String>,
}

impl CreativeCreationTaskPage {
    pub fn try_new(
        tasks: Vec<CreationTask>,
        next_cursor: Option<String>,
    ) -> Result<Self, AppError> {
        Ok(Self {
            items: tasks
                .into_iter()
                .map(CreativeCreationTask::try_from)
                .collect::<Result<_, _>>()?,
            next_cursor,
        })
    }
}

impl TryFrom<CreationTask> for CreativeCreationTask {
    type Error = AppError;

    fn try_from(task: CreationTask) -> Result<Self, Self::Error> {
        let owner = match (
            task.canvas_id,
            task.workbench_kind,
            task.template_id,
            task.template_run_id,
            task.template_step_id,
            task.node_id,
        ) {
            (Some(canvas_id), None, None, None, None, Some(node_id)) => {
                CreativeCreationTaskOwner::CanvasNode {
                    canvas_id,
                    node_id,
                }
            }
            (_, Some(workbench_kind), None, None, None, None) => {
                CreativeCreationTaskOwner::StandaloneWorkbench {
                    workbench_kind,
                }
            }
            (None, None, Some(template_id), Some(template_run_id), Some(template_step_id), None) => {
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
        };
        Ok(Self {
            creation_task_id: task.creation_task_id,
            owner,
            provider_id: task.provider_id,
            model: task.model,
            capability: task.capability,
            params: task.params,
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
            creation_task_id: creation_task_id.clone(),
            project_id: Some(canvas_id.clone()),
            workbench_kind: None,
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
    fn standalone_owner_and_ordered_inputs_round_trip_without_guessing() {
        let input_asset_id = WorkshopAssetId::new().into_string();
        let row = CreationTaskRow {
            creation_task_id: generate_id(),
            project_id: None,
            workbench_kind: Some("video".into()),
            template_id: None,
            template_run_id: None,
            template_step_id: None,
            node_id: None,
            provider_id: ProviderId::new().into_string(),
            model: "video-model".into(),
            capability: "i2v".into(),
            params: r#"{"prompt":"Aurora","seconds":5}"#.into(),
            input_bindings: Some(
                serde_json::json!([{
                    "asset_id": input_asset_id,
                    "kind": "image",
                    "role": "first_frame"
                }])
                .to_string(),
            ),
            status: "failed".into(),
            error: Some(r#"{"kind":"provider_error","message":"failed"}"#.into()),
            result_asset_ids: "[]".into(),
            remote_task_id: None,
            attempt: 1,
            submitted_at: 1,
            started_at: Some(2),
            finished_at: Some(3),
            deleted_at: Some(3),
        };
        let task = CreationTask::try_from(row).unwrap();
        assert_eq!(task.inputs.as_ref().unwrap()[0].asset_id, input_asset_id);
        let page = CreativeCreationTaskPage::try_new(
            vec![task],
            Some("3:0190f5fe-7c00-7a00-8000-000000000001".into()),
        )
        .unwrap();
        let wire = serde_json::to_value(page).unwrap();
        assert_eq!(wire["items"][0]["owner"]["kind"], "standalone_workbench");
        assert!(wire["items"][0]["owner"].get("project_id").is_none());
        assert_eq!(wire["items"][0]["owner"]["workbench_kind"], "video");
        assert_eq!(wire["items"][0]["inputs"][0]["asset_id"], input_asset_id);
        assert_eq!(wire["items"][0]["inputs"][0]["kind"], "image");
        assert_eq!(wire["items"][0]["inputs"][0]["role"], "first_frame");
        assert_eq!(wire["items"][0]["deleted_at"], 3);
        assert!(wire["next_cursor"].as_str().is_some());
    }

    #[test]
    fn succeeded_without_artifacts_fails_closed() {
        let row = CreationTaskRow {
            creation_task_id: generate_id(),
            project_id: Some(CreativeStudioCanvasId::new().into_string()),
            workbench_kind: None,
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
                creation_task_id: creation_task_id.into(),
                project_id: Some(CreativeStudioCanvasId::new().into_string()),
                workbench_kind: None,
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
