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

use crate::dto::{
    CreateSshHostRequest, ImportSshHostsRequest, SshHostResponse, SshImportResult, SshStatusEvent,
    UpdateSshHostRequest,
};
use crate::pool::SshConnectionPool;
use crate::service::{SshHostService, SshServiceError};
use crate::ssh_config::SshConfigScan;

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
        // Before the `{ssh_host_id}` capture for the reader's sake; axum 0.8
        // prefers a literal segment over a capture regardless of order, and no
        // real host id can spell `statuses` because every one of them is a uuid.
        .route("/api/ssh-hosts/statuses", get(statuses))
        .route("/api/ssh-hosts/import-candidates", get(import_candidates))
        .route("/api/ssh-hosts/import", post(import_from_ssh_config))
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
    // The row is gone, so the links must go too: a live transport outlives its
    // host row happily, the supervisor's probe keeps succeeding, and the agent
    // goes on running commands on a machine the operator just deleted — with no
    // pill on screen, because the pill needs the row to render.
    if let Some(pool) = &state.pool {
        pool.close_for_host(&ssh_host_id).await;
    }
    Ok(Json(ApiResponse::ok(())))
}

/// Every live link the caller owns, in the *same* wire shape the realtime
/// `ssh.status` event carries. A client that missed an event and re-fetches can
/// therefore never be told a different story than the one it was pushed.
///
/// Unlike test-connection, a missing pool is answered with an empty list rather
/// than an error: a build with no SSH support truthfully owns no links, and this
/// route is polled whenever a session opens — turning "not configured" into a
/// failed request would break screens that are merely uninterested in SSH.
async fn statuses(
    State(state): State<SshHostRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<SshStatusEvent>>>, AppError> {
    let items = state
        .pool
        .as_ref()
        .map(|pool| pool.snapshot(user.id.as_str()))
        .unwrap_or_default();
    Ok(Json(ApiResponse::ok(items)))
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

/// Hosts in this account's `~/.ssh/config` that could be added to the book.
///
/// Non-secret by construction: a candidate names the identity *file*, never its
/// contents (`the_candidate_list_never_carries_private_key_material` pins that).
/// Read-only, and the config is the only file read.
///
/// The `CurrentUser` extractor is kept even though the scan is per-machine rather
/// than per-owner: this router is mounted under the instance-owner guard, and a
/// handler that asks for no identity is one refactor away from being mounted
/// somewhere that grants none.
async fn import_candidates(
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<SshConfigScan>>, AppError> {
    let scan = crate::ssh_config::scan_default_ssh_config()
        .map_err(|e| AppError::Internal(format!("read ssh config: {e}")))?;
    Ok(Json(ApiResponse::ok(scan)))
}

/// Add the confirmed candidates to the book.
async fn import_from_ssh_config(
    State(state): State<SshHostRouterState>,
    Extension(user): Extension<CurrentUser>,
    body: Result<Json<ImportSshHostsRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<SshImportResult>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    // Re-scan instead of trusting the client's copy of the candidates. The only
    // files this route may read are the ones this account's own ssh config names;
    // accepting host/port/key paths from the request body would turn an import
    // into an arbitrary-file-read primitive.
    let scan = crate::ssh_config::scan_default_ssh_config()
        .map_err(|e| AppError::Internal(format!("read ssh config: {e}")))?;
    let result = state
        .service
        .import_hosts(user.id.as_str(), &req.aliases, &scan.hosts)
        .await
        .map_err(map_err)?;
    Ok(Json(ApiResponse::ok(result)))
}
