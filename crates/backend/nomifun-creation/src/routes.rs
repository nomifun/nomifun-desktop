//! Canonical `/api/creative-studio/tasks` handlers. The retired unowned
//! `/api/creation/tasks` surface is deliberately not mounted.

use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Json, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::Value;

use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;

use crate::dto::CreativeCreationTask;
#[cfg(test)]
use crate::dto::CreationTask;
use crate::service::{CreativeTaskOwner, NewCreationTask};
use crate::state::CreationRouterState;
use crate::types::{CreationInput, CreationInputKind, StandaloneWorkbenchKind};

pub fn creation_routes(state: CreationRouterState) -> Router {
    Router::new()
        .route("/api/creative-studio/tasks", post(create_creative_task))
        .route(
            "/api/creative-studio/tasks/{creation_task_id}",
            get(get_creative_task),
        )
        .route(
            "/api/creative-studio/tasks/{creation_task_id}/cancel",
            post(cancel_creative_task),
        )
        .with_state(state)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputRef {
    asset_id: String,
    kind: CreationInputKind,
    #[serde(default = "default_role")]
    role: String,
}

fn default_role() -> String {
    "reference".to_string()
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum CreativeTaskOwnerRequest {
    CanvasNode {
        project_id: String,
        node_id: String,
    },
    StandaloneWorkbench {
        project_id: String,
        workbench_kind: StandaloneWorkbenchKind,
    },
    WorkflowStep {
        workflow_id: String,
        workflow_run_id: String,
        workflow_step_id: String,
    },
}

impl From<CreativeTaskOwnerRequest> for CreativeTaskOwner {
    fn from(owner: CreativeTaskOwnerRequest) -> Self {
        match owner {
            CreativeTaskOwnerRequest::CanvasNode {
                project_id,
                node_id,
            } => Self::CanvasNode {
                project_id,
                node_id,
            },
            CreativeTaskOwnerRequest::StandaloneWorkbench {
                project_id,
                workbench_kind,
            } => Self::StandaloneWorkbench {
                project_id,
                workbench_kind,
            },
            CreativeTaskOwnerRequest::WorkflowStep {
                workflow_id,
                workflow_run_id,
                workflow_step_id,
            } => Self::WorkflowStep {
                workflow_id,
                workflow_run_id,
                workflow_step_id,
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateCreativeTaskRequest {
    owner: CreativeTaskOwnerRequest,
    provider_id: String,
    model: String,
    capability: String,
    #[serde(default)]
    params: Value,
    #[serde(default)]
    inputs: Vec<InputRef>,
}

fn required_idempotency_key(headers: &HeaderMap) -> Result<String, AppError> {
    let mut values = headers.get_all("idempotency-key").iter();
    let value = values.next().ok_or_else(|| {
        AppError::BadRequest("Creative Studio task creation requires Idempotency-Key".into())
    })?;
    if values.next().is_some() {
        return Err(AppError::BadRequest(
            "Idempotency-Key must be sent exactly once".into(),
        ));
    }
    value
        .to_str()
        .map(str::to_owned)
        .map_err(|_| AppError::BadRequest("Idempotency-Key must be visible ASCII".into()))
}

async fn create_creative_task(
    State(state): State<CreationRouterState>,
    Extension(_user): Extension<CurrentUser>,
    headers: HeaderMap,
    body: Result<Json<CreateCreativeTaskRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    let Json(req) = body.map_err(|error| AppError::BadRequest(error.to_string()))?;
    let idempotency_key = required_idempotency_key(&headers)?;
    let task = state
        .service
        .create_creative_task(
            req.owner.into(),
            idempotency_key,
            NewCreationTask {
                provider_id: req.provider_id,
                model: req.model,
                capability: req.capability,
                params: req.params,
                inputs: req
                    .inputs
                    .into_iter()
                    .map(|input| CreationInput {
                        asset_id: input.asset_id,
                        kind: input.kind,
                        role: input.role,
                    })
                    .collect(),
            },
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(ApiResponse::ok(CreativeCreationTask::try_from(task)?)),
    ))
}

async fn get_creative_task(
    State(state): State<CreationRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(creation_task_id): Path<String>,
) -> Result<Json<ApiResponse<CreativeCreationTask>>, AppError> {
    let task = state.service.get_task(&creation_task_id).await?;
    Ok(Json(ApiResponse::ok(CreativeCreationTask::try_from(task)?)))
}

async fn cancel_creative_task(
    State(state): State<CreationRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(creation_task_id): Path<String>,
) -> Result<Json<ApiResponse<CreativeCreationTask>>, AppError> {
    let task = state.service.cancel_task(&creation_task_id).await?;
    Ok(Json(ApiResponse::ok(CreativeCreationTask::try_from(task)?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use serde_json::json;

    #[test]
    fn creative_task_request_accepts_only_the_tagged_owner_contract() {
        let parsed = serde_json::from_value::<CreateCreativeTaskRequest>(json!({
            "owner": {
                "kind": "workflow_step",
                "workflow_id": "0190f5fe-7c00-7a00-8000-000000000001",
                "workflow_run_id": "0190f5fe-7c00-7a00-8000-000000000002",
                "workflow_step_id": "0190f5fe-7c00-7a00-8000-000000000003"
            },
            "provider_id": "0190f5fe-7c00-7a00-8000-000000000004",
            "model": "image-model-v1",
            "capability": "t2i",
            "params": {"prompt": "Aurora"},
            "inputs": []
        }))
        .unwrap();
        assert!(matches!(
            parsed.owner,
            CreativeTaskOwnerRequest::WorkflowStep { .. }
        ));

        let standalone = serde_json::from_value::<CreateCreativeTaskRequest>(json!({
            "owner": {
                "kind": "standalone_workbench",
                "project_id": "0190f5fe-7c00-7a00-8000-000000000001",
                "workbench_kind": "video"
            },
            "provider_id": "0190f5fe-7c00-7a00-8000-000000000004",
            "model": "video-model-v1",
            "capability": "i2v",
            "params": {"prompt": "Aurora"},
            "inputs": [{
                "asset_id": "0190f5fe-7c00-7a00-8000-000000000006",
                "kind": "image",
                "role": "first_frame"
            }]
        }))
        .unwrap();
        assert!(matches!(
            standalone.owner,
            CreativeTaskOwnerRequest::StandaloneWorkbench {
                workbench_kind: StandaloneWorkbenchKind::Video,
                ..
            }
        ));

        for invalid in [
            json!({
                "project_id": "0190f5fe-7c00-7a00-8000-000000000001",
                "node_id": "0190f5fe-7c00-7a00-8000-000000000002",
                "provider_id": "0190f5fe-7c00-7a00-8000-000000000004",
                "model": "image-model-v1",
                "capability": "t2i"
            }),
            json!({
                "owner": {
                    "kind": "canvas_node",
                    "project_id": "0190f5fe-7c00-7a00-8000-000000000001",
                    "node_id": "0190f5fe-7c00-7a00-8000-000000000002",
                    "canvas_id": "0190f5fe-7c00-7a00-8000-000000000005"
                },
                "provider_id": "0190f5fe-7c00-7a00-8000-000000000004",
                "model": "image-model-v1",
                "capability": "t2i"
            }),
            json!({
                "owner": {
                    "kind": "standalone_workbench",
                    "project_id": "0190f5fe-7c00-7a00-8000-000000000001",
                    "workbench_kind": "video"
                },
                "provider_id": "0190f5fe-7c00-7a00-8000-000000000004",
                "model": "video-model-v1",
                "capability": "i2v",
                "inputs": [{
                    "asset_id": "0190f5fe-7c00-7a00-8000-000000000006",
                    "role": "first_frame"
                }]
            }),
        ] {
            assert!(serde_json::from_value::<CreateCreativeTaskRequest>(invalid).is_err());
        }
    }

    #[test]
    fn creative_task_idempotency_key_is_required_exactly_once() {
        let mut headers = HeaderMap::new();
        assert!(required_idempotency_key(&headers).is_err());

        headers.append(
            "idempotency-key",
            HeaderValue::from_static("0190f5fe-7c00-7a00-8000-000000000001"),
        );
        assert_eq!(
            required_idempotency_key(&headers).unwrap(),
            "0190f5fe-7c00-7a00-8000-000000000001"
        );

        headers.append(
            "idempotency-key",
            HeaderValue::from_static("0190f5fe-7c00-7a00-8000-000000000002"),
        );
        assert!(required_idempotency_key(&headers).is_err());
    }

    #[test]
    fn creative_task_surface_rejects_invalid_owner_rows() {
        let invalid = CreationTask {
            creation_task_id: "0190f5fe-7c00-7a00-8000-000000000001".into(),
            project_id: None,
            workbench_kind: None,
            workflow_id: None,
            workflow_run_id: None,
            workflow_step_id: None,
            node_id: None,
            provider_id: "0190f5fe-7c00-7a00-8000-000000000004".into(),
            model: "image-model-v1".into(),
            capability: "t2i".into(),
            params: json!({}),
            inputs: Some(Vec::new()),
            status: "queued".into(),
            error: None,
            result_asset_ids: Vec::new(),
            attempt: 0,
            submitted_at: 1,
            started_at: None,
            finished_at: None,
        };
        assert!(CreativeCreationTask::try_from(invalid).is_err());
    }
}
