use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use nomifun_api_types::{
    ApiResponse, BuildMiniAppRequest, CancelMiniAppBuildRequest,
    CloseMiniAppSurfaceRequest, CreateMiniAppProjectRequest, DeleteMiniAppRequest,
    DurableOperationSummaryDto, ImportMiniAppArtifactRequest, ImportMiniAppShareRequest,
    MiniAppLibraryResponseDto, MiniAppSurfaceLaunchDescriptorDto,
    MiniAppWorkshopDto, OpenMiniAppSurfaceRequest, PublishMiniAppRequest, RollbackMiniAppRequest,
    RestoreMiniAppRequest, RetryMiniAppDeleteRequest, RetryMiniAppServiceRequest,
    SetMiniAppEnabledRequest, SetMiniAppPublishModeRequest, SetMiniAppServiceRunningRequest,
    ShareMiniAppRequest, TestMiniAppReleaseRequest, TrashMiniAppRequest,
};
use nomifun_agent_contracts::{MiniAppBridgeRequest, StrictJsonValue};
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;
use nomifun_miniapp_platform::{
    content_type_for_surface_path, MiniAppM1ApplicationError,
    MiniAppM1ApplicationService,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MiniAppSurfaceBridgeHttpRequest {
    surface_capability: String,
    active_release_epoch: u64,
    expected_release_digest: String,
    request: MiniAppBridgeRequest,
}

#[derive(Clone)]
pub struct MiniAppM1RouterState {
    application: Arc<MiniAppM1ApplicationService>,
}

impl MiniAppM1RouterState {
    pub(crate) fn new(application: Arc<MiniAppM1ApplicationService>) -> Self {
        Self { application }
    }
}

pub(crate) fn miniapp_m1_read_routes(
    state: MiniAppM1RouterState,
) -> Router {
    Router::new()
        .route("/api/miniapps", get(list_miniapps))
        .route(
            "/api/miniapps/{miniapp_id}/workshop",
            get(get_workshop),
        )
        .with_state(state)
}

pub(crate) fn miniapp_m1_write_routes(
    state: MiniAppM1RouterState,
) -> Router {
    Router::new()
        .route("/api/miniapps/projects", post(create_project))
        .route("/api/miniapps/import/share", post(import_share))
        .route("/api/miniapps/import/artifact", post(import_artifact))
        .route(
            "/api/miniapps/{miniapp_id}/build",
            post(build_miniapp),
        )
        .route(
            "/api/miniapps/{miniapp_id}/operations/{operation_id}/cancel",
            post(cancel_miniapp_build),
        )
        .route(
            "/api/miniapps/{miniapp_id}/publish",
            post(publish_miniapp),
        )
        .route(
            "/api/miniapps/{miniapp_id}/test",
            post(test_miniapp_release),
        )
        .route("/api/miniapps/{miniapp_id}/share", post(export_share))
        .route(
            "/api/miniapps/{miniapp_id}/rollback",
            post(rollback_miniapp),
        )
        .route(
            "/api/miniapps/{miniapp_id}/enabled",
            post(set_miniapp_enabled),
        )
        .route(
            "/api/miniapps/{miniapp_id}/publish-mode",
            post(set_miniapp_publish_mode),
        )
        .route(
            "/api/miniapps/{miniapp_id}/service/running",
            post(set_miniapp_service_running),
        )
        .route(
            "/api/miniapps/{miniapp_id}/service/retry",
            post(retry_miniapp_service),
        )
        .route(
            "/api/miniapps/{miniapp_id}/trash",
            post(trash_miniapp),
        )
        .route(
            "/api/miniapps/{miniapp_id}/restore",
            post(restore_miniapp),
        )
        .route(
            "/api/miniapps/{miniapp_id}/delete",
            post(delete_miniapp),
        )
        .route(
            "/api/miniapps/{miniapp_id}/delete/retry",
            post(retry_delete_miniapp),
        )
        .route(
            "/api/miniapps/{miniapp_id}/surface/open",
            post(open_surface),
        )
        .route(
            "/api/miniapps/{miniapp_id}/surface/bridge",
            post(call_surface_bridge),
        )
        .route(
            "/api/miniapps/{miniapp_id}/surface/close",
            post(close_surface),
        )
        .with_state(state)
}

pub(crate) fn miniapp_m1_surface_routes(state: MiniAppM1RouterState) -> Router {
    Router::new()
        .route(
            "/api/miniapps/{miniapp_id}/surface/assets/{capability_id}/{active_release_epoch}/{release_digest}/{*asset_path}",
            get(get_surface_asset),
        )
        .with_state(state)
}

async fn list_miniapps(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<MiniAppLibraryResponseDto>>, AppError> {
    let library = state
        .application
        .library(user.id.as_str())
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(library)))
}

async fn create_project(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<CreateMiniAppProjectRequest>,
) -> Result<Json<ApiResponse<MiniAppWorkshopDto>>, AppError> {
    let workshop = state
        .application
        .create(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn import_share(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<ImportMiniAppShareRequest>,
) -> Result<Json<ApiResponse<MiniAppWorkshopDto>>, AppError> {
    let workshop = state
        .application
        .import_share(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn import_artifact(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<ImportMiniAppArtifactRequest>,
) -> Result<Json<ApiResponse<MiniAppWorkshopDto>>, AppError> {
    let workshop = state
        .application
        .import_prebuilt(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn get_workshop(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
) -> Result<Json<ApiResponse<MiniAppWorkshopDto>>, AppError> {
    let workshop = state
        .application
        .workshop(user.id.as_str(), &miniapp_id)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn open_surface(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<OpenMiniAppSurfaceRequest>,
) -> Result<Json<ApiResponse<MiniAppSurfaceLaunchDescriptorDto>>, AppError> {
    require_route_id("miniapp_id", &miniapp_id, &request.miniapp_id)?;
    let descriptor = state
        .application
        .open_surface(user.id.as_str(), &miniapp_id)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(descriptor)))
}

async fn build_miniapp(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<BuildMiniAppRequest>,
) -> Result<Json<ApiResponse<MiniAppWorkshopDto>>, AppError> {
    require_route_id("miniapp_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .build(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn cancel_miniapp_build(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path((miniapp_id, operation_id)): Path<(String, String)>,
    Json(request): Json<CancelMiniAppBuildRequest>,
) -> Result<Json<ApiResponse<DurableOperationSummaryDto>>, AppError> {
    let operation = state
        .application
        .cancel_build(
            user.id.as_str(),
            &miniapp_id,
            &operation_id,
            request.expected_operation_revision,
        )
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(operation)))
}

async fn publish_miniapp(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<PublishMiniAppRequest>,
) -> Result<Json<ApiResponse<MiniAppWorkshopDto>>, AppError> {
    require_route_id("miniapp_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .publish(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn test_miniapp_release(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<TestMiniAppReleaseRequest>,
) -> Result<Json<ApiResponse<MiniAppWorkshopDto>>, AppError> {
    require_route_id("miniapp_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .test_ready_service(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn export_share(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<ShareMiniAppRequest>,
) -> Result<Json<ApiResponse<DurableOperationSummaryDto>>, AppError> {
    require_route_id("miniapp_id", &miniapp_id, &request.miniapp_id)?;
    let operation = state
        .application
        .export_share(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(operation)))
}

async fn rollback_miniapp(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<RollbackMiniAppRequest>,
) -> Result<Json<ApiResponse<MiniAppWorkshopDto>>, AppError> {
    require_route_id("miniapp_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .rollback(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn set_miniapp_enabled(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<SetMiniAppEnabledRequest>,
) -> Result<Json<ApiResponse<MiniAppWorkshopDto>>, AppError> {
    require_route_id("miniapp_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .set_enabled(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn set_miniapp_publish_mode(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<SetMiniAppPublishModeRequest>,
) -> Result<Json<ApiResponse<MiniAppWorkshopDto>>, AppError> {
    require_route_id("miniapp_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .set_publish_mode(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn set_miniapp_service_running(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<SetMiniAppServiceRunningRequest>,
) -> Result<Json<ApiResponse<MiniAppWorkshopDto>>, AppError> {
    require_route_id("miniapp_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .set_service_running(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn retry_miniapp_service(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<RetryMiniAppServiceRequest>,
) -> Result<Json<ApiResponse<MiniAppWorkshopDto>>, AppError> {
    require_route_id("miniapp_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .retry_service(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn trash_miniapp(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<TrashMiniAppRequest>,
) -> Result<Json<ApiResponse<MiniAppWorkshopDto>>, AppError> {
    require_route_id("miniapp_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .trash(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn restore_miniapp(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<RestoreMiniAppRequest>,
) -> Result<Json<ApiResponse<MiniAppWorkshopDto>>, AppError> {
    require_route_id("miniapp_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .restore(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn delete_miniapp(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<DeleteMiniAppRequest>,
) -> Result<Json<ApiResponse<MiniAppLibraryResponseDto>>, AppError> {
    require_route_id("miniapp_id", &miniapp_id, &request.miniapp_id)?;
    let library = state
        .application
        .delete(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(library)))
}

async fn retry_delete_miniapp(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<RetryMiniAppDeleteRequest>,
) -> Result<Json<ApiResponse<MiniAppLibraryResponseDto>>, AppError> {
    require_route_id("miniapp_id", &miniapp_id, &request.miniapp_id)?;
    let library = state
        .application
        .retry_delete(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(library)))
}

async fn call_surface_bridge(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(body): Json<MiniAppSurfaceBridgeHttpRequest>,
) -> Result<Json<ApiResponse<StrictJsonValue>>, AppError> {
    let result = state
        .application
        .surface_bridge_request(
            user.id.as_str(),
            &miniapp_id,
            &body.surface_capability,
            body.active_release_epoch,
            &body.expected_release_digest,
            body.request,
        )
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(result)))
}

async fn close_surface(
    State(state): State<MiniAppM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<CloseMiniAppSurfaceRequest>,
) -> Result<Json<ApiResponse<bool>>, AppError> {
    require_route_id("miniapp_id", &miniapp_id, &request.miniapp_id)?;
    let closed = state
        .application
        .close_surface(
            user.id.as_str(),
            &miniapp_id,
            &request.surface_session_id,
            &request.surface_capability,
        )
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(closed)))
}

async fn get_surface_asset(
    State(state): State<MiniAppM1RouterState>,
    Path((
        miniapp_id,
        capability_id,
        active_release_epoch,
        release_digest,
        asset_path,
    )): Path<(String, String, u64, String, String)>,
) -> Result<Response, AppError> {
    let asset = state
        .application
        .surface_asset(
            &miniapp_id,
            &capability_id,
            active_release_epoch,
            &release_digest,
            &asset_path,
        )
        .await
        .map_err(application_error)?;
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            content_type_for_surface_path(&asset.normalized_relative_path),
        )
        .header(header::CACHE_CONTROL, "private, no-store")
        .body(Body::from(asset.bytes))
        .map_err(|error| AppError::Internal(error.to_string()))
}

fn require_route_id(field: &'static str, route: &str, body: &str) -> Result<(), AppError> {
    if route == body {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!(
            "{field} must match the route identity"
        )))
    }
}

fn application_error(error: MiniAppM1ApplicationError) -> AppError {
    match error {
        MiniAppM1ApplicationError::Invalid(message) => {
            AppError::BadRequest(format!("MiniApp input is invalid: {message}"))
        }
        MiniAppM1ApplicationError::NotFound => {
            AppError::NotFound("MiniApp".to_owned())
        }
        MiniAppM1ApplicationError::Runtime(message) => {
            AppError::Internal(format!("MiniApp runtime failed: {message}"))
        }
        MiniAppM1ApplicationError::Database(error) => error.into(),
    }
}

#[cfg(test)]
#[path = "miniapp_m1_tests.rs"]
mod tests;
