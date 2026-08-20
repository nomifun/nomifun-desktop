//! `/api/creation/tasks` route handlers (contract §3.3). Owner-only — mounted
//! behind the app's authenticated router (same auth extractor as the workshop
//! routes).

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

use crate::dto::CreationTask;
use crate::service::NewCreationTask;
use crate::state::CreationRouterState;
use crate::types::CreationInput;

pub fn creation_routes(state: CreationRouterState) -> Router {
    Router::new()
        .route("/api/creation/tasks", get(list_tasks).post(create_task))
        .route("/api/creation/tasks/{creation_task_id}", get(get_task))
        .route("/api/creation/tasks/{creation_task_id}/cancel", post(cancel_task))
        .with_state(state)
}

#[derive(Deserialize)]
struct InputRef {
    asset_id: String,
    #[serde(default = "default_role")]
    role: String,
}

fn default_role() -> String {
    "reference".to_string()
}

#[derive(Deserialize)]
struct CreateTaskRequest {
    #[serde(default)]
    project_id: Option<String>,
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
    let mut idempotency_values = headers.get_all("idempotency-key").iter();
    let idempotency_value = idempotency_values.next();
    if idempotency_values.next().is_some() {
        return Err(AppError::BadRequest(
            "Idempotency-Key must be sent exactly once".into(),
        ));
    }
    let idempotency_key = idempotency_value
        .map(|value| {
            value
                .to_str()
                .map(str::to_owned)
                .map_err(|_| AppError::BadRequest("Idempotency-Key must be visible ASCII".into()))
        })
        .transpose()?;
    let task = match req.project_id {
        Some(project_id) => {
            let idempotency_key = idempotency_key.ok_or_else(|| {
                AppError::BadRequest(
                    "Creative Studio task creation requires Idempotency-Key".into(),
                )
            })?;
            state
                .service
                .create_creative_project_task(project_id, idempotency_key, task_request)
                .await?
        }
        None => {
            if idempotency_key.is_some() {
                return Err(AppError::BadRequest(
                    "Idempotency-Key is reserved for canonical project_id task creation".into(),
                ));
            }
            state.service.create_task(task_request).await?
        }
    };
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(task))))
}

#[derive(Deserialize)]
struct ListTasksQuery {
    canvas_id: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
}

#[derive(serde::Serialize)]
struct TaskListResponse {
    tasks: Vec<CreationTask>,
}

async fn list_tasks(
    State(state): State<CreationRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Query(query): Query<ListTasksQuery>,
) -> Result<Json<ApiResponse<TaskListResponse>>, AppError> {
    let tasks = state
        .service
        .list_tasks(query.canvas_id.as_deref(), query.status.as_deref(), query.limit.unwrap_or(100))
        .await?;
    Ok(Json(ApiResponse::ok(TaskListResponse { tasks })))
}

async fn get_task(
    State(state): State<CreationRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(creation_task_id): Path<String>,
) -> Result<Json<ApiResponse<CreationTask>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.get_task(&creation_task_id).await?)))
}

async fn cancel_task(
    State(state): State<CreationRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(creation_task_id): Path<String>,
) -> Result<Json<ApiResponse<CreationTask>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.cancel_task(&creation_task_id).await?)))
}
