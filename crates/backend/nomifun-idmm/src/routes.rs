//! IDMM HTTP routes. Handlers do request/response transformation only; all
//! logic lives in `IdmmService`. Auth is layered externally in nomifun-app
//! (mirrors the requirement routes).

use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Json, Path, Query, State};
use axum::routing::{get, post};

use nomifun_api_types::{
    ApiResponse, IdmmConfig, IdmmState, IdmmTargetKind, InterventionRecord, SetIdmmRequest,
};
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;
use serde::Deserialize;

use crate::state::IdmmRouterState;

/// Default `?limit` for `GET .../log` — matches the per-target eviction cap, so
/// the timeline shows every record the aggressive pruning keeps.
const DEFAULT_LOG_LIMIT: i64 = 30;

/// Query string for `GET .../log`.
#[derive(Debug, Deserialize)]
struct LogQuery {
    /// Max rows to return (most-recent-first). Defaults to [`DEFAULT_LOG_LIMIT`].
    limit: Option<i64>,
}

pub fn idmm_routes(state: IdmmRouterState) -> Router {
    Router::new()
        .route("/api/idmm", post(set_idmm))
        .route("/api/idmm/{kind}/{target_id}", get(get_idmm))
        .route("/api/idmm/{kind}/{target_id}/intervene", post(intervene))
        .route("/api/idmm/{kind}/{target_id}/log", get(get_log).delete(clear_log))
        .with_state(state)
}

/// Resolve a `{kind}` path segment. Ownership is enforced inside every service
/// operation, so handlers cannot accidentally separate authorization from use.
fn parse_kind(kind: &str) -> Result<IdmmTargetKind, AppError> {
    IdmmTargetKind::parse(kind)
        .ok_or_else(|| AppError::BadRequest(format!("unknown idmm target kind: {kind}")))
}

async fn set_idmm(
    State(state): State<IdmmRouterState>,
    Extension(user): Extension<CurrentUser>,
    body: Result<Json<SetIdmmRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<IdmmState>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let cfg: IdmmConfig = req.config;
    state
        .service
        .save_config(&user.id, req.kind, &req.target_id, &cfg)
        .await?;
    let st = state
        .service
        .build_state(&user.id, req.kind, &req.target_id)
        .await?;
    Ok(Json(ApiResponse::ok(st)))
}

async fn get_idmm(
    State(state): State<IdmmRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path((kind, target_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<IdmmState>>, AppError> {
    let kind = parse_kind(&kind)?;
    let st = state.service.build_state(&user.id, kind, &target_id).await?;
    Ok(Json(ApiResponse::ok(st)))
}

async fn intervene(
    State(state): State<IdmmRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path((kind, target_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<IdmmState>>, AppError> {
    let kind = parse_kind(&kind)?;
    state
        .service
        .intervene_now(&user.id, kind, &target_id)
        .await?;
    let st = state.service.build_state(&user.id, kind, &target_id).await?;
    Ok(Json(ApiResponse::ok(st)))
}

async fn get_log(
    State(state): State<IdmmRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path((kind, target_id)): Path<(String, String)>,
    Query(q): Query<LogQuery>,
) -> Result<Json<ApiResponse<Vec<InterventionRecord>>>, AppError> {
    let kind = parse_kind(&kind)?;
    let limit = q.limit.unwrap_or(DEFAULT_LOG_LIMIT);
    let log = state.service.log(&user.id, kind, &target_id, limit).await?;
    Ok(Json(ApiResponse::ok(log)))
}

async fn clear_log(
    State(state): State<IdmmRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path((kind, target_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<u64>>, AppError> {
    let kind = parse_kind(&kind)?;
    let removed = state.service.clear_log(&user.id, kind, &target_id).await?;
    Ok(Json(ApiResponse::ok(removed)))
}
