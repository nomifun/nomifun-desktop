//! IDMM HTTP routes. Handlers do request/response transformation only; all
//! logic lives in `IdmmService`. Auth is layered externally in nomifun-app
//! (mirrors the requirement routes).

use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Json, Path, Query, State};
use axum::routing::{get, post};

use nomifun_api_types::{
    ApiResponse, IdmmConfig, IdmmSettings, IdmmState, IdmmTargetKind, InterventionRecord, SetIdmmRequest,
};
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;
use serde::Deserialize;

use crate::state::IdmmRouterState;

/// Default `?limit` for `GET .../log` — matches the per-target eviction cap, so
/// the timeline shows every record the aggressive pruning keeps.
const DEFAULT_LOG_LIMIT: i64 = 30;

/// Default `?limit` for the cross-session activity feed (`GET /api/idmm/activity`).
const DEFAULT_ACTIVITY_LIMIT: i64 = 50;

/// Query string for `GET .../log`.
#[derive(Debug, Deserialize)]
struct LogQuery {
    /// Max rows to return (most-recent-first). Defaults to [`DEFAULT_LOG_LIMIT`].
    limit: Option<i64>,
}

/// Query string for `GET /api/idmm/activity`.
#[derive(Debug, Deserialize)]
struct ActivityQuery {
    /// Max rows to return (most-recent-first). Defaults to [`DEFAULT_ACTIVITY_LIMIT`].
    limit: Option<i64>,
}

pub fn idmm_routes(state: IdmmRouterState) -> Router {
    Router::new()
        .route("/api/idmm", post(set_idmm))
        .route("/api/idmm/settings", get(get_settings).put(set_settings))
        .route("/api/idmm/activity", get(get_activity).delete(clear_activity))
        .route("/api/idmm/{kind}/{target_id}", get(get_idmm))
        .route("/api/idmm/{kind}/{target_id}/intervene", post(intervene))
        .route("/api/idmm/{kind}/{target_id}/log", get(get_log).delete(clear_log))
        .with_state(state)
}

/// Resolve + ownership-check a `{kind}/{target_id}` pair.
async fn resolve_owned(
    state: &IdmmRouterState,
    kind: &str,
    target_id: &str,
    user_id: &str,
) -> Result<IdmmTargetKind, AppError> {
    let kind =
        IdmmTargetKind::parse(kind).ok_or_else(|| AppError::BadRequest(format!("unknown idmm target kind: {kind}")))?;
    if kind == IdmmTargetKind::Terminal {
        state.service.verify_terminal_owner(target_id, user_id).await?;
    }
    Ok(kind)
}

async fn set_idmm(
    State(state): State<IdmmRouterState>,
    Extension(user): Extension<CurrentUser>,
    body: Result<Json<SetIdmmRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<IdmmState>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    if req.kind == IdmmTargetKind::Terminal {
        state.service.verify_terminal_owner(&req.target_id, &user.id).await?;
    }
    let cfg: IdmmConfig = req.config;
    state.service.save_config(req.kind, &req.target_id, &cfg).await?;
    let st = state.service.build_state(req.kind, &req.target_id).await?;
    Ok(Json(ApiResponse::ok(st)))
}

async fn get_idmm(
    State(state): State<IdmmRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path((kind, target_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<IdmmState>>, AppError> {
    let kind = resolve_owned(&state, &kind, &target_id, &user.id).await?;
    let st = state.service.build_state(kind, &target_id).await?;
    Ok(Json(ApiResponse::ok(st)))
}

async fn intervene(
    State(state): State<IdmmRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path((kind, target_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<IdmmState>>, AppError> {
    let kind = resolve_owned(&state, &kind, &target_id, &user.id).await?;
    state.service.intervene_now(kind, &target_id).await?;
    let st = state.service.build_state(kind, &target_id).await?;
    Ok(Json(ApiResponse::ok(st)))
}

async fn get_log(
    State(state): State<IdmmRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path((kind, target_id)): Path<(String, String)>,
    Query(q): Query<LogQuery>,
) -> Result<Json<ApiResponse<Vec<InterventionRecord>>>, AppError> {
    let kind = resolve_owned(&state, &kind, &target_id, &user.id).await?;
    let limit = q.limit.unwrap_or(DEFAULT_LOG_LIMIT);
    let log = state.service.log(kind, &target_id, limit).await?;
    Ok(Json(ApiResponse::ok(log)))
}

async fn clear_log(
    State(state): State<IdmmRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path((kind, target_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<u64>>, AppError> {
    let kind = resolve_owned(&state, &kind, &target_id, &user.id).await?;
    let removed = state.service.clear_log(kind, &target_id).await?;
    Ok(Json(ApiResponse::ok(removed)))
}

async fn get_activity(
    State(state): State<IdmmRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Query(q): Query<ActivityQuery>,
) -> Result<Json<ApiResponse<Vec<InterventionRecord>>>, AppError> {
    let limit = q.limit.unwrap_or(DEFAULT_ACTIVITY_LIMIT);
    let activity = state.service.recent_activity(limit).await?;
    Ok(Json(ApiResponse::ok(activity)))
}

async fn clear_activity(
    State(state): State<IdmmRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<u64>>, AppError> {
    let removed = state.service.clear_all_activity().await?;
    Ok(Json(ApiResponse::ok(removed)))
}

async fn get_settings(
    State(state): State<IdmmRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<IdmmSettings>>, AppError> {
    let s = state.service.get_settings().await?;
    Ok(Json(ApiResponse::ok(s)))
}

async fn set_settings(
    State(state): State<IdmmRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<IdmmSettings>, JsonRejection>,
) -> Result<Json<ApiResponse<IdmmSettings>>, AppError> {
    let Json(settings) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state.service.set_settings(&settings).await?;
    Ok(Json(ApiResponse::ok(state.service.get_settings().await?)))
}
