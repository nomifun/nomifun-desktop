//! HTTP routes for mini-apps: owner-scoped CRUD, the explicit publish action, the
//! idempotent workspace provision, and the auth-exempt document serve channel. The
//! management router is mounted under the instance-owner guard by the app router
//! (mirrors `ssh_host_routes`); every handler there scopes to `CurrentUser.id`, so
//! a cross-owner id is indistinguishable from NotFound.
use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Json, Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use nomifun_common::{AppError, MiniAppId};

use crate::dto::{
    CreateMiniAppRequest, MiniAppImportRequest, MiniAppImportResponse, MiniAppResponse,
    MiniAppWorkspaceResponse, UpdateMiniAppRequest,
};
use crate::service::MiniAppServiceError;
use crate::state::MiniAppRouterState;

pub fn miniapp_routes(state: MiniAppRouterState) -> Router {
    Router::new()
        .route("/api/miniapps", get(list).post(create))
        .route("/api/miniapps/validate", post(validate_candidate))
        .route("/api/miniapps/import", post(import_candidate))
        .route(
            "/api/miniapps/{miniapp_id}",
            get(get_one).put(update).delete(delete_one),
        )
        .route("/api/miniapps/{miniapp_id}/publish", post(publish))
        .route("/api/miniapps/{miniapp_id}/workspace", post(provision_workspace))
        .with_state(state)
}

/// Auth-EXEMPT read-only document serve route. GET-only, one path only, opaque
/// unguessable ids. Merged into the app's public router next to the other
/// auth-exempt serve routes (logos / office proxy / workshop assets). Every
/// write / list / read of metadata stays under auth in [`miniapp_routes`].
///
/// This handler MUST NOT extract `Extension<CurrentUser>`: an `<iframe>` document
/// load carries no trust header, so `trust_resolve_middleware` injects no
/// `CurrentUser` and that extractor would 500 the very requests this router
/// exists to serve.
pub fn miniapp_public_routes(state: MiniAppRouterState) -> Router {
    Router::new()
        .route("/api/miniapps/{miniapp_id}/serve", get(serve))
        .with_state(state)
}

/// The served document is a full HTML page, always UTF-8 (the generator writes
/// UTF-8 and the column stores TEXT).
const SERVE_CONTENT_TYPE: &str = "text/html; charset=utf-8";
/// `Cache-Control` for a served app: privately cacheable but revalidated every
/// load. Unlike an immutable asset, the same id serves a new document after every
/// re-solidify, and an iterating user must see the version they just saved.
const SERVE_CACHE_CONTROL: &str = "private, no-cache";

fn map_err(e: MiniAppServiceError) -> AppError {
    match e {
        MiniAppServiceError::NotFound => AppError::NotFound("miniapp".into()),
        MiniAppServiceError::BadRequest(m) => AppError::BadRequest(m),
        MiniAppServiceError::Internal(m) => AppError::Internal(m),
    }
}

async fn list(
    State(state): State<MiniAppRouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<MiniAppResponse>>>, AppError> {
    let items = state.service.list(user.id.as_str()).await.map_err(map_err)?;
    Ok(Json(ApiResponse::ok(items)))
}

async fn get_one(
    State(state): State<MiniAppRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<MiniAppId>,
) -> Result<Json<ApiResponse<MiniAppResponse>>, AppError> {
    let app = state
        .service
        .get(user.id.as_str(), &miniapp_id)
        .await
        .map_err(map_err)?;
    Ok(Json(ApiResponse::ok(app)))
}

async fn create(
    State(state): State<MiniAppRouterState>,
    Extension(user): Extension<CurrentUser>,
    body: Result<Json<CreateMiniAppRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<MiniAppResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let app = state
        .service
        .create(user.id.as_str(), req)
        .await
        .map_err(map_err)?;
    Ok(Json(ApiResponse::ok(app)))
}

async fn update(
    State(state): State<MiniAppRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<MiniAppId>,
    body: Result<Json<UpdateMiniAppRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<MiniAppResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let app = state
        .service
        .update(user.id.as_str(), &miniapp_id, req)
        .await
        .map_err(map_err)?;
    Ok(Json(ApiResponse::ok(app)))
}

async fn delete_one(
    State(state): State<MiniAppRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<MiniAppId>,
) -> Result<Json<ApiResponse<bool>>, AppError> {
    state
        .service
        .delete(user.id.as_str(), &miniapp_id)
        .await
        .map_err(map_err)?;
    Ok(Json(ApiResponse::ok(true)))
}

/// Publish the on-disk working copy into the served snapshot.
///
/// Takes no body: the client never names a path or a document. Where the working
/// copy lives is derived from `miniapp_id` alone, and what gets published is
/// whatever last edited it — a client-supplied body would be a second, unreviewed
/// way to write the served document.
///
/// A 400 when there is no working copy yet: "nothing to publish" is a state the
/// user can fix by iterating, not a missing app (404) and not a server fault.
async fn publish(
    State(state): State<MiniAppRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<MiniAppId>,
) -> Result<Json<ApiResponse<MiniAppResponse>>, AppError> {
    let app = state
        .service
        .publish(user.id.as_str(), &miniapp_id)
        .await
        .map_err(map_err)?;
    Ok(Json(ApiResponse::ok(app)))
}

/// Idempotently ensure the app's directory and working copy exist, and answer with
/// the absolute path of the working copy.
///
/// Takes no body for the same reason `publish` does not: the client never names a
/// path. Where the working copy lives is derived from `miniapp_id` and run through
/// the escape guard, so a client that could name it could point an agent at any
/// file on the machine — the client only *reads* the answer back.
///
/// Creates NO conversation. 「继续迭代」 calls this first and then starts an ordinary
/// conversation whose first message carries this path, so the mini-app owns its
/// source while the thread that edits it is an ordinary thread in an ordinary
/// workspace.
///
/// Safe to call on every open: the working copy is materialized from the published
/// snapshot only when it is missing, so an iteration in flight is never clobbered.
async fn provision_workspace(
    State(state): State<MiniAppRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<MiniAppId>,
) -> Result<Json<ApiResponse<MiniAppWorkspaceResponse>>, AppError> {
    let workspace = state
        .service
        .provision_workspace(user.id.as_str(), &miniapp_id)
        .await
        .map_err(map_err)?;
    Ok(Json(ApiResponse::ok(workspace)))
}

/// Report on a candidate without writing anything.
///
/// Registered before the `{miniapp_id}` capture so `validate` and `import` are
/// never read as ids.
async fn validate_candidate(
    State(state): State<MiniAppRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<MiniAppImportRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<MiniAppImportResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let report = state.service.validate_candidate(&req).await.map_err(map_err)?;
    Ok(Json(ApiResponse::ok(report)))
}

/// Adopt a candidate. A blocked candidate is a 400 whose body carries the report:
/// the client needs the findings, and a bare status would make it ask twice.
async fn import_candidate(
    State(state): State<MiniAppRouterState>,
    Extension(user): Extension<CurrentUser>,
    body: Result<Json<MiniAppImportRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let outcome = state
        .service
        .import_candidate(user.id.as_str(), req)
        .await
        .map_err(map_err)?;
    let status = if outcome.report.blocked {
        axum::http::StatusCode::BAD_REQUEST
    } else {
        axum::http::StatusCode::OK
    };
    Ok((status, Json(ApiResponse::ok(outcome))).into_response())
}

/// AUTH-EXEMPT (mounted on [`miniapp_public_routes`]): no `CurrentUser`
/// extractor. Hands out the stored HTML document for an iframe `src`; missing →
/// 404.
///
/// Sets no `Content-Security-Policy`: `security_headers_middleware` owns the
/// serve policy (`sandbox` without `allow-same-origin` + a narrow
/// `frame-ancestors`), exactly as it does for the office preview proxy. Two
/// sources would mean two CSP fields whose intersection nobody reviews.
async fn serve(
    State(state): State<MiniAppRouterState>,
    Path(miniapp_id): Path<MiniAppId>,
) -> Result<Response, AppError> {
    let html = state
        .service
        .serve_html(&miniapp_id)
        .await
        .map_err(map_err)?;
    Ok((
        [
            (header::CONTENT_TYPE, SERVE_CONTENT_TYPE),
            (header::CACHE_CONTROL, SERVE_CACHE_CONTROL),
        ],
        Body::from(html),
    )
        .into_response())
}
