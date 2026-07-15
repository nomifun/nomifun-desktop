//! Wire DTO for the `/api/creation/tasks` surface (contract §3.3). snake_case
//! (serde default). Owned by this crate (the shared `api-types` crate is not in
//! this module's ownership).

use nomifun_common::{
    AppError, CreationTaskId, ProviderId, TimestampMs, WorkshopAssetId, WorkshopCanvasId,
    WorkshopNodeId,
};
use nomifun_db::CreationTaskRow;
use serde::Serialize;
use serde_json::Value;

/// A generation task as seen over the wire.
#[derive(Debug, Clone, Serialize)]
pub struct CreationTask {
    pub id: String,
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
        CreationTaskId::parse(&row.id).map_err(|error| corrupt_id("creation_tasks.id", error))?;
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

        Ok(Self {
            id: row.id,
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

fn corrupt_id(field: &str, error: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("invalid canonical ID in {field}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_dto_parses_json_columns() {
        let task_id = CreationTaskId::new().into_string();
        let canvas_id = WorkshopCanvasId::new().into_string();
        let provider_id = ProviderId::new().into_string();
        let asset_id = WorkshopAssetId::new().into_string();
        let row = CreationTaskRow {
            id: task_id,
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
        assert_eq!(dto.error.unwrap()["kind"], "adapter_unavailable");
        assert_eq!(dto.result_asset_ids, vec![asset_id]);
        assert_eq!(dto.finished_at, Some(2));
    }
}
