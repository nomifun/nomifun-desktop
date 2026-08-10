//! HTTP routes for mini-apps: owner-scoped CRUD plus the auth-exempt document
//! serve channel. The management router is mounted under the instance-owner
//! guard by the app router (mirrors `ssh_host_routes`); every handler there
//! scopes to `CurrentUser.id`, so a cross-owner id is indistinguishable from
//! NotFound.
use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Json, Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use nomifun_common::{AppError, MiniAppId};

use crate::dto::{CreateMiniAppRequest, MiniAppResponse, UpdateMiniAppRequest};
use crate::service::MiniAppServiceError;
use crate::state::MiniAppRouterState;

pub fn miniapp_routes(state: MiniAppRouterState) -> Router {
    Router::new()
        .route("/api/miniapps", get(list).post(create))
        .route(
            "/api/miniapps/{miniapp_id}",
            get(get_one).put(update).delete(delete_one),
        )
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
