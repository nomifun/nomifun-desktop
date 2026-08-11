//! `/api/crawl/*` route handlers.

use axum::Router;
use axum::extract::{Extension, Json, Path, Query, State};
use axum::routing::{get, post};
use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use nomifun_common::{AppError, CrawlJobId};
use serde::Deserialize;

use crate::model::TaskStatus;
use crate::service::{JobView, NewJob, TaskView};
use crate::state::CrawlRouterState;

const DEFAULT_TASK_LIMIT: u32 = 200;
const MAX_TASK_LIMIT: u32 = 1_000;

pub fn crawl_routes(state: CrawlRouterState) -> Router {
    Router::new()
        .route("/api/crawl/jobs", get(list_jobs).post(create_job))
        .route("/api/crawl/jobs/{job_id}", get(get_job).delete(delete_job))
        .route("/api/crawl/jobs/{job_id}/start", post(start_job))
        .route("/api/crawl/jobs/{job_id}/cancel", post(cancel_job))
        .route("/api/crawl/jobs/{job_id}/retry-failed", post(retry_failed))
        .route("/api/crawl/jobs/{job_id}/tasks", get(list_tasks))
        .with_state(state)
}

fn parse_job_id(raw: &str) -> Result<CrawlJobId, AppError> {
    CrawlJobId::parse(raw).map_err(|e| AppError::BadRequest(format!("invalid job id: {e}")))
}

async fn list_jobs(
    State(state): State<CrawlRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<JobView>>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.list(&user.id).await?)))
}

async fn create_job(
    State(state): State<CrawlRouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(req): Json<NewJob>,
) -> Result<Json<ApiResponse<JobView>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.create(&user.id, req).await?)))
}

async fn get_job(
    State(state): State<CrawlRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(job_id): Path<String>,
) -> Result<Json<ApiResponse<JobView>>, AppError> {
    let id = parse_job_id(&job_id)?;
    Ok(Json(ApiResponse::ok(state.service.get(&user.id, &id).await?)))
}

async fn start_job(
    State(state): State<CrawlRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(job_id): Path<String>,
) -> Result<Json<ApiResponse<JobView>>, AppError> {
    let id = parse_job_id(&job_id)?;
    Ok(Json(ApiResponse::ok(state.service.start(&user.id, &id).await?)))
}

async fn cancel_job(
    State(state): State<CrawlRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(job_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let id = parse_job_id(&job_id)?;
    state.service.cancel(&user.id, &id).await?;
    Ok(Json(ApiResponse::ok(())))
}

async fn delete_job(
    State(state): State<CrawlRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(job_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let id = parse_job_id(&job_id)?;
    state.service.delete(&user.id, &id).await?;
    Ok(Json(ApiResponse::ok(())))
}

async fn retry_failed(
    State(state): State<CrawlRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(job_id): Path<String>,
) -> Result<Json<ApiResponse<u64>>, AppError> {
    let id = parse_job_id(&job_id)?;
    Ok(Json(ApiResponse::ok(state.service.retry_failed(&user.id, &id).await?)))
}

#[derive(Deserialize)]
struct TaskQuery {
    status: Option<String>,
    limit: Option<u32>,
}

async fn list_tasks(
    State(state): State<CrawlRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(job_id): Path<String>,
    Query(query): Query<TaskQuery>,
) -> Result<Json<ApiResponse<Vec<TaskView>>>, AppError> {
    let id = parse_job_id(&job_id)?;
    let status = match query.status.as_deref() {
        None | Some("") | Some("all") => None,
        Some(raw) => Some(
            TaskStatus::parse(raw)
                .ok_or_else(|| AppError::BadRequest(format!("unknown task status: {raw}")))?,
        ),
    };
    let limit = query.limit.unwrap_or(DEFAULT_TASK_LIMIT).clamp(1, MAX_TASK_LIMIT);
    Ok(Json(ApiResponse::ok(
        state.service.tasks(&user.id, &id, status, limit).await?,
    )))
}
