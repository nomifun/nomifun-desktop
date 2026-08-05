//! HTTP routes for the SSH host book: owner-scoped CRUD plus a test-connection
//! probe. Mounted under the instance-owner guard by the app router (mirrors
//! `remote_agent_routes`). Every handler scopes to `CurrentUser.id`, so a
//! cross-owner id is indistinguishable from NotFound.
use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Json, Path, State};
use axum::routing::{get, post};
use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use nomifun_common::{AppError, SshHostId};

use crate::dto::{CreateSshHostRequest, SshHostResponse, UpdateSshHostRequest};
use crate::pool::SshConnectionPool;
use crate::service::{SshHostService, SshServiceError};

/// Router state: the host book plus the process connection pool (both cheap
/// handles). The pool is the same one the agent factory dials through, so a probe
/// here and a session there cannot disagree about a host.
#[derive(Clone)]
pub struct SshHostRouterState {
    pub service: SshHostService,
    /// `None` → this server has no SSH support wired, and test-connection is
    /// refused rather than answered with a guess.
    pub pool: Option<SshConnectionPool>,
}

pub fn ssh_host_routes(state: SshHostRouterState) -> Router {
    Router::new()
        .route("/api/ssh-hosts", get(list).post(create))
        .route(
            "/api/ssh-hosts/{ssh_host_id}",
            get(get_one).put(update).delete(delete_one),
        )
        .route(
            "/api/ssh-hosts/{ssh_host_id}/test-connection",
            post(test_connection),
        )
        .with_state(state)
}

fn map_err(e: SshServiceError) -> AppError {
    match e {
        SshServiceError::NotFound => AppError::NotFound("ssh_host".into()),
        SshServiceError::BadRequest(m) => AppError::BadRequest(m),
        SshServiceError::Internal(m) => AppError::Internal(m),
    }
}

async fn list(
    State(state): State<SshHostRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<SshHostResponse>>>, AppError> {
    let items = state.service.list(user.id.as_str()).await.map_err(map_err)?;
    Ok(Json(ApiResponse::ok(items)))
}

async fn get_one(
    State(state): State<SshHostRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(ssh_host_id): Path<SshHostId>,
) -> Result<Json<ApiResponse<SshHostResponse>>, AppError> {
    let host = state
        .service
        .get(user.id.as_str(), &ssh_host_id)
        .await
        .map_err(map_err)?;
    Ok(Json(ApiResponse::ok(host)))
}

async fn create(
    State(state): State<SshHostRouterState>,
    Extension(user): Extension<CurrentUser>,
    body: Result<Json<CreateSshHostRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<SshHostResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let host = state
        .service
        .create(user.id.as_str(), req)
        .await
        .map_err(map_err)?;
    Ok(Json(ApiResponse::ok(host)))
}

async fn update(
    State(state): State<SshHostRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(ssh_host_id): Path<SshHostId>,
    body: Result<Json<UpdateSshHostRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<SshHostResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let host = state
        .service
        .update(user.id.as_str(), &ssh_host_id, req)
        .await
        .map_err(map_err)?;
    Ok(Json(ApiResponse::ok(host)))
}

async fn delete_one(
    State(state): State<SshHostRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(ssh_host_id): Path<SshHostId>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    state
        .service
        .delete(user.id.as_str(), &ssh_host_id)
        .await
        .map_err(map_err)?;
    Ok(Json(ApiResponse::ok(())))
}

/// Result of a test-connection probe.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TestConnectionResult {
    ok: bool,
    /// Why, in the operator's terms — the probe's own detail, so a failure names
    /// the setting to fix instead of a generic "could not connect".
    message: String,
}

async fn test_connection(
    State(state): State<SshHostRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(ssh_host_id): Path<SshHostId>,
) -> Result<Json<ApiResponse<TestConnectionResult>>, AppError> {
    let Some(pool) = &state.pool else {
        return Err(AppError::BadRequest(
            "SSH support is not configured on this server".into(),
        ));
    };
    // The pool's probe dials, runs one trivial command and closes with the same
    // forensics a session close uses — so "test" proves the connection instead of
    // leaving a socket behind for the runtime to trip over.
    let outcome = pool.probe(user.id.as_str(), &ssh_host_id).await;
    Ok(Json(ApiResponse::ok(TestConnectionResult {
        ok: outcome.ok,
        message: outcome.detail,
    })))
}
