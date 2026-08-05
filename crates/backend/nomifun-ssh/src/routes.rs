//! HTTP routes for the SSH host book: owner-scoped CRUD plus a test-connection
//! probe. Mounted under the instance-owner guard by the app router (mirrors
//! `remote_agent_routes`). Every handler scopes to `CurrentUser.id`, so a
//! cross-owner id is indistinguishable from NotFound.
use std::sync::Arc;

use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Json, Path, State};
use axum::routing::{get, post};
use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use nomifun_common::{AppError, SshHostId};

use crate::dto::{CreateSshHostRequest, SshHostResponse, UpdateSshHostRequest};
use crate::service::{SshHostService, SshServiceError};

/// Router state: just the host-book service (cheap to clone).
#[derive(Clone)]
pub struct SshHostRouterState {
    pub service: SshHostService,
    /// Connection prober for test-connection (optional; None → probe unsupported).
    pub provider: Option<Arc<dyn nomifun_ai_agent::SshBackendProvider>>,
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
    /// Present on success: the host's SHA256 fingerprint (also persisted).
    message: String,
}

async fn test_connection(
    State(state): State<SshHostRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(ssh_host_id): Path<SshHostId>,
) -> Result<Json<ApiResponse<TestConnectionResult>>, AppError> {
    let Some(provider) = &state.provider else {
        return Err(AppError::BadRequest(
            "SSH support is not configured on this server".into(),
        ));
    };
    // Connect with the remote $HOME (".") and run a trivial probe. A dropped
    // connection is closed when the returned backend is dropped at scope end.
    match provider
        .connect(user.id.as_str(), ssh_host_id.as_str(), ".")
        .await
    {
        Ok(backend) => match backend.run_command("true", 15_000).await {
            Ok(_) => Ok(Json(ApiResponse::ok(TestConnectionResult {
                ok: true,
                message: "connection succeeded".into(),
            }))),
            Err(e) => Ok(Json(ApiResponse::ok(TestConnectionResult {
                ok: false,
                message: format!("connected but probe failed: {e}"),
            }))),
        },
        Err(e) => Ok(Json(ApiResponse::ok(TestConnectionResult {
            ok: false,
            message: e,
        }))),
    }
}
