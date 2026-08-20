//! Disjoint legacy `/api/creation/tasks` and canonical
//! `/api/creative-studio/tasks` handlers. Both are owner-only behind the app's
//! authenticated router; only the canonical surface accepts tagged owners.

use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Json, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use serde::Deserialize;
use serde_json::Value;

use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;

use crate::dto::{CreationTask, CreativeCreationTask};
use crate::service::{CreativeTaskOwner, NewCreationTask};
use crate::state::CreationRouterState;
use crate::types::CreationInput;

pub fn creation_routes(state: CreationRouterState) -> Router {
    Router::new()
        .route("/api/creation/tasks", get(list_tasks).post(create_task))
        .route("/api/creation/tasks/{creation_task_id}", get(get_task))
        .route("/api/creation/tasks/{creation_task_id}/cancel", post(cancel_task))
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
    #[serde(default = "default_role")]
    role: String,
}

fn default_role() -> String {
    "reference".to_string()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateTaskRequest {
    #[serde(default)]
    canvas_id: Option<String>,
    #[serde(default)]
    node_id: Option<String>,
    provider_id: String,
    model: String,
    capability: String,
    #[serde(default)]
    params: Value,
    #[serde(default)]
    inputs: Vec<InputRef>,
}

async fn create_task(
    State(state): State<CreationRouterState>,
    Extension(_user): Extension<CurrentUser>,
    headers: HeaderMap,
    body: Result<Json<CreateTaskRequest>, JsonRejection>,
) -> Result<impl IntoResponse, AppError> {
    if headers.contains_key("idempotency-key") {
        return Err(AppError::BadRequest(
            "Idempotency-Key is reserved for /api/creative-studio/tasks".into(),
        ));
    }
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let task_request = NewCreationTask {
        canvas_id: req.canvas_id,
        node_id: req.node_id,
        provider_id: req.provider_id,
        model: req.model,
        capability: req.capability,
        params: req.params,
        inputs: req
            .inputs
            .into_iter()
            .map(|i| CreationInput { asset_id: i.asset_id, role: i.role })
            .collect(),
    };
    let task = state.service.create_task(task_request).await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(task))))
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum CreativeTaskOwnerRequest {
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
                canvas_id: None,
                node_id: None,
                provider_id: req.provider_id,
                model: req.model,
                capability: req.capability,
                params: req.params,
                inputs: req
                    .inputs
                    .into_iter()
                    .map(|input| CreationInput {
                        asset_id: input.asset_id,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListTasksQuery {
    canvas_id: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
}

#[derive(serde::Serialize)]
struct TaskListResponse {
    tasks: Vec<CreationTask>,
}

fn is_legacy_task(task: &CreationTask) -> bool {
    task.project_id.is_none()
        && task.workflow_id.is_none()
        && task.workflow_run_id.is_none()
        && task.workflow_step_id.is_none()
}

fn require_legacy_task(task: CreationTask) -> Result<CreationTask, AppError> {
    if is_legacy_task(&task) {
        Ok(task)
    } else {
        Err(AppError::NotFound("creation task not found".into()))
    }
}

fn require_creative_task(task: CreationTask) -> Result<CreativeCreationTask, AppError> {
    if is_legacy_task(&task) {
        return Err(AppError::NotFound("creative task not found".into()));
    }
    CreativeCreationTask::try_from(task)
}

async fn list_tasks(
    State(state): State<CreationRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Query(query): Query<ListTasksQuery>,
) -> Result<Json<ApiResponse<TaskListResponse>>, AppError> {
    let tasks = state
        .service
        .list_tasks(query.canvas_id.as_deref(), query.status.as_deref(), query.limit.unwrap_or(100))
        .await?
        .into_iter()
        .filter(is_legacy_task)
        .collect();
    Ok(Json(ApiResponse::ok(TaskListResponse { tasks })))
}

async fn get_task(
    State(state): State<CreationRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(creation_task_id): Path<String>,
) -> Result<Json<ApiResponse<CreationTask>>, AppError> {
    let task = require_legacy_task(state.service.get_task(&creation_task_id).await?)?;
    Ok(Json(ApiResponse::ok(task)))
}

async fn cancel_task(
    State(state): State<CreationRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(creation_task_id): Path<String>,
) -> Result<Json<ApiResponse<CreationTask>>, AppError> {
    require_legacy_task(state.service.get_task(&creation_task_id).await?)?;
    let task = require_legacy_task(state.service.cancel_task(&creation_task_id).await?)?;
    Ok(Json(ApiResponse::ok(task)))
}

async fn get_creative_task(
    State(state): State<CreationRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(creation_task_id): Path<String>,
) -> Result<Json<ApiResponse<CreativeCreationTask>>, AppError> {
    let task = state.service.get_task(&creation_task_id).await?;
    Ok(Json(ApiResponse::ok(require_creative_task(task)?)))
}

async fn cancel_creative_task(
    State(state): State<CreationRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(creation_task_id): Path<String>,
) -> Result<Json<ApiResponse<CreativeCreationTask>>, AppError> {
    require_creative_task(state.service.get_task(&creation_task_id).await?)?;
    let task = state.service.cancel_task(&creation_task_id).await?;
    Ok(Json(ApiResponse::ok(require_creative_task(task)?)))
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
    fn legacy_and_creative_task_surfaces_are_disjoint() {
        let legacy = CreationTask {
            creation_task_id: "0190f5fe-7c00-7a00-8000-000000000001".into(),
            project_id: None,
            workflow_id: None,
            workflow_run_id: None,
            workflow_step_id: None,
            canvas_id: Some("0190f5fe-7c00-7a00-8000-000000000002".into()),
            node_id: Some("0190f5fe-7c00-7a00-8000-000000000003".into()),
            provider_id: "0190f5fe-7c00-7a00-8000-000000000004".into(),
            model: "image-model-v1".into(),
            capability: "t2i".into(),
            params: json!({}),
            status: "queued".into(),
            error: None,
            result_asset_ids: Vec::new(),
            attempt: 0,
            submitted_at: 1,
            started_at: None,
            finished_at: None,
        };
        let mut creative = legacy.clone();
        creative.canvas_id = None;
        creative.project_id = Some("0190f5fe-7c00-7a00-8000-000000000005".into());

        assert!(require_legacy_task(legacy.clone()).is_ok());
        assert!(matches!(
            require_creative_task(legacy),
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            require_legacy_task(creative.clone()),
            Err(AppError::NotFound(_))
        ));
        assert!(require_creative_task(creative).is_ok());
    }
}
