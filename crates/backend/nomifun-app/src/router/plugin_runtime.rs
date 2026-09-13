use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use nomifun_api_types::{
    ApiResponse, BuildPluginRuntimeRequest, CancelPluginRuntimeBuildRequest,
    ClosePluginRuntimeSurfaceRequest, CreatePluginRuntimeProjectRequest, DeletePluginRuntimeRequest,
    DurableOperationSummaryDto, ImportPluginRuntimeArtifactRequest, ImportPluginRuntimeShareRequest,
    ExportPluginRuntimeBackupRequest, ImportPluginRuntimeBackupRequest,
    PluginRuntimeLibraryResponseDto, PluginRuntimeSourceFileDto, PluginRuntimeSurfaceLaunchDescriptorDto,
    PluginRuntimeWorkshopDto, OpenPluginRuntimeSurfaceRequest, PublishPluginRuntimeRequest,
    ReplacePluginRuntimeSourceFileRequest, RestorePluginRuntimeRequest, RetryPluginRuntimeDeleteRequest,
    RetryPluginRuntimeServiceRequest, RollbackPluginRuntimeRequest,
    SetPluginRuntimeEnabledRequest, SetPluginRuntimePublishModeRequest, SetPluginRuntimeServiceRunningRequest,
    SharePluginRuntimeRequest, TestPluginRuntimeReleaseRequest, TrashPluginRuntimeRequest,
};
use nomifun_agent_contracts::{MiniAppBridgeRequest, StrictJsonValue};
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;
use nomifun_plugin_platform::runtime::{
    content_type_for_surface_path, PluginRuntimeM1ApplicationError,
    PluginRuntimeM1ApplicationService,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginRuntimeSurfaceBridgeHttpRequest {
    surface_capability: String,
    active_release_epoch: u64,
    expected_release_digest: String,
    request: MiniAppBridgeRequest,
}

#[derive(Clone)]
pub struct PluginRuntimeM1RouterState {
    application: Arc<PluginRuntimeM1ApplicationService>,
    pub(super) product: Option<Arc<super::plugin_product::PluginRuntimeProductService>>,
}

impl PluginRuntimeM1RouterState {
    pub(crate) fn new(application: Arc<PluginRuntimeM1ApplicationService>) -> Self {
        Self { application, product: None }
    }

    pub(crate) fn with_product(mut self, product: super::plugin_product::PluginRuntimeProductService) -> Self {
        self.product = Some(Arc::new(product)); self
    }
}

pub(crate) fn miniapp_m1_read_routes(
    state: PluginRuntimeM1RouterState,
) -> Router {
    Router::new()
        .route("/api/plugins/runtimes", get(list_miniapps))
        .merge(super::plugin_product::read_routes())
        .route(
            "/api/plugins/runtimes/{miniapp_id}/workshop",
            get(get_workshop),
        )
        .with_state(state)
}

pub(crate) fn miniapp_m1_write_routes(
    state: PluginRuntimeM1RouterState,
) -> Router {
    Router::new()
        .route("/api/plugins/runtimes/projects", post(create_project))
        .merge(super::plugin_product::write_routes())
        .route(
            "/api/plugins/runtimes/{miniapp_id}/source/files/{*source_path}",
            get(get_source_file),
        )
        .route(
            "/api/plugins/runtimes/{miniapp_id}/source/edit",
            post(replace_source_file),
        )
        .route("/api/plugins/runtimes/import/share", post(import_share))
        .route("/api/plugins/runtimes/import/artifact", post(import_artifact))
        .route("/api/plugins/runtimes/import/backup", post(import_backup))
        .route(
            "/api/plugins/runtimes/{miniapp_id}/build",
            post(build_miniapp),
        )
        .route(
            "/api/plugins/runtimes/{miniapp_id}/operations/{operation_id}/cancel",
            post(cancel_miniapp_build),
        )
        .route(
            "/api/plugins/runtimes/{miniapp_id}/publish",
            post(publish_miniapp),
        )
        .route(
            "/api/plugins/runtimes/{miniapp_id}/test",
            post(test_miniapp_release),
        )
        .route("/api/plugins/runtimes/{miniapp_id}/share", post(export_share))
        .route("/api/plugins/runtimes/{miniapp_id}/backup", post(export_backup))
        .route(
            "/api/plugins/runtimes/{miniapp_id}/rollback",
            post(rollback_miniapp),
        )
        .route(
            "/api/plugins/runtimes/{miniapp_id}/enabled",
            post(set_miniapp_enabled),
        )
        .route(
            "/api/plugins/runtimes/{miniapp_id}/publish-mode",
            post(set_miniapp_publish_mode),
        )
        .route(
            "/api/plugins/runtimes/{miniapp_id}/service/running",
            post(set_miniapp_service_running),
        )
        .route(
            "/api/plugins/runtimes/{miniapp_id}/service/retry",
            post(retry_miniapp_service),
        )
        .route(
            "/api/plugins/runtimes/{miniapp_id}/trash",
            post(trash_miniapp),
        )
        .route(
            "/api/plugins/runtimes/{miniapp_id}/restore",
            post(restore_miniapp),
        )
        .route(
            "/api/plugins/runtimes/{miniapp_id}/delete",
            post(delete_miniapp),
        )
        .route(
            "/api/plugins/runtimes/{miniapp_id}/delete/retry",
            post(retry_delete_miniapp),
        )
        .route(
            "/api/plugins/runtimes/{miniapp_id}/surface/open",
            post(open_surface),
        )
        .route(
            "/api/plugins/runtimes/{miniapp_id}/surface/bridge",
            post(call_surface_bridge),
        )
        .route(
            "/api/plugins/runtimes/{miniapp_id}/surface/close",
            post(close_surface),
        )
        .with_state(state)
}

pub(crate) fn miniapp_m1_surface_routes(state: PluginRuntimeM1RouterState) -> Router {
    Router::new()
        .route(
            "/api/plugins/runtimes/{miniapp_id}/surface/assets/{capability_id}/{active_release_epoch}/{release_digest}/{*asset_path}",
            get(get_surface_asset),
        )
        .with_state(state)
}

async fn list_miniapps(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<PluginRuntimeLibraryResponseDto>>, AppError> {
    let library = state
        .application
        .library(user.id.as_str())
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(library)))
}

async fn create_project(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<CreatePluginRuntimeProjectRequest>,
) -> Result<Json<ApiResponse<PluginRuntimeWorkshopDto>>, AppError> {
    let workshop = state
        .application
        .create(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn get_source_file(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path((miniapp_id, source_path)): Path<(String, String)>,
) -> Result<Json<ApiResponse<PluginRuntimeSourceFileDto>>, AppError> {
    let source = state
        .application
        .source_file(user.id.as_str(), &miniapp_id, &source_path)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(source)))
}

async fn replace_source_file(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<ReplacePluginRuntimeSourceFileRequest>,
) -> Result<Json<ApiResponse<PluginRuntimeWorkshopDto>>, AppError> {
    require_route_id("plugin_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .replace_source_file(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn import_share(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<ImportPluginRuntimeShareRequest>,
) -> Result<Json<ApiResponse<PluginRuntimeWorkshopDto>>, AppError> {
    let workshop = state
        .application
        .import_share(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn import_artifact(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<ImportPluginRuntimeArtifactRequest>,
) -> Result<Json<ApiResponse<PluginRuntimeWorkshopDto>>, AppError> {
    let workshop = state
        .application
        .import_prebuilt(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn import_backup(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Json(request): Json<ImportPluginRuntimeBackupRequest>,
) -> Result<Json<ApiResponse<PluginRuntimeWorkshopDto>>, AppError> {
    let workshop = state
        .application
        .import_backup(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn get_workshop(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
) -> Result<Json<ApiResponse<PluginRuntimeWorkshopDto>>, AppError> {
    let workshop = state
        .application
        .workshop(user.id.as_str(), &miniapp_id)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn open_surface(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<OpenPluginRuntimeSurfaceRequest>,
) -> Result<Json<ApiResponse<PluginRuntimeSurfaceLaunchDescriptorDto>>, AppError> {
    require_route_id("plugin_id", &miniapp_id, &request.miniapp_id)?;
    let descriptor = state
        .application
        .open_surface(user.id.as_str(), &miniapp_id)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(descriptor)))
}

async fn build_miniapp(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<BuildPluginRuntimeRequest>,
) -> Result<Json<ApiResponse<PluginRuntimeWorkshopDto>>, AppError> {
    require_route_id("plugin_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .build(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn cancel_miniapp_build(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path((miniapp_id, operation_id)): Path<(String, String)>,
    Json(request): Json<CancelPluginRuntimeBuildRequest>,
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
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<PublishPluginRuntimeRequest>,
) -> Result<Json<ApiResponse<PluginRuntimeWorkshopDto>>, AppError> {
    require_route_id("plugin_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .publish(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn test_miniapp_release(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<TestPluginRuntimeReleaseRequest>,
) -> Result<Json<ApiResponse<PluginRuntimeWorkshopDto>>, AppError> {
    require_route_id("plugin_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .test_ready_service(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn export_share(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<SharePluginRuntimeRequest>,
) -> Result<Json<ApiResponse<DurableOperationSummaryDto>>, AppError> {
    require_route_id("plugin_id", &miniapp_id, &request.miniapp_id)?;
    let operation = state
        .application
        .export_share(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(operation)))
}

async fn export_backup(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<ExportPluginRuntimeBackupRequest>,
) -> Result<Json<ApiResponse<DurableOperationSummaryDto>>, AppError> {
    require_route_id("plugin_id", &miniapp_id, &request.miniapp_id)?;
    let operation = state
        .application
        .export_backup(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(operation)))
}

async fn rollback_miniapp(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<RollbackPluginRuntimeRequest>,
) -> Result<Json<ApiResponse<PluginRuntimeWorkshopDto>>, AppError> {
    require_route_id("plugin_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .rollback(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn set_miniapp_enabled(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<SetPluginRuntimeEnabledRequest>,
) -> Result<Json<ApiResponse<PluginRuntimeWorkshopDto>>, AppError> {
    require_route_id("plugin_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .set_enabled(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn set_miniapp_publish_mode(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<SetPluginRuntimePublishModeRequest>,
) -> Result<Json<ApiResponse<PluginRuntimeWorkshopDto>>, AppError> {
    require_route_id("plugin_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .set_publish_mode(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn set_miniapp_service_running(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<SetPluginRuntimeServiceRunningRequest>,
) -> Result<Json<ApiResponse<PluginRuntimeWorkshopDto>>, AppError> {
    require_route_id("plugin_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .set_service_running(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn retry_miniapp_service(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<RetryPluginRuntimeServiceRequest>,
) -> Result<Json<ApiResponse<PluginRuntimeWorkshopDto>>, AppError> {
    require_route_id("plugin_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .retry_service(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn trash_miniapp(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<TrashPluginRuntimeRequest>,
) -> Result<Json<ApiResponse<PluginRuntimeWorkshopDto>>, AppError> {
    require_route_id("plugin_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .trash(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn restore_miniapp(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<RestorePluginRuntimeRequest>,
) -> Result<Json<ApiResponse<PluginRuntimeWorkshopDto>>, AppError> {
    require_route_id("plugin_id", &miniapp_id, &request.miniapp_id)?;
    let workshop = state
        .application
        .restore(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    Ok(Json(ApiResponse::ok(workshop)))
}

async fn delete_miniapp(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<DeletePluginRuntimeRequest>,
) -> Result<Json<ApiResponse<PluginRuntimeLibraryResponseDto>>, AppError> {
    require_route_id("plugin_id", &miniapp_id, &request.miniapp_id)?;
    let library = state
        .application
        .delete(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    if let Some(product) = &state.product {
        product.cancel_app_jobs(user.id.as_str(), &miniapp_id).await;
    }
    Ok(Json(ApiResponse::ok(library)))
}

async fn retry_delete_miniapp(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<RetryPluginRuntimeDeleteRequest>,
) -> Result<Json<ApiResponse<PluginRuntimeLibraryResponseDto>>, AppError> {
    require_route_id("plugin_id", &miniapp_id, &request.miniapp_id)?;
    let library = state
        .application
        .retry_delete(user.id.as_str(), request)
        .await
        .map_err(application_error)?;
    if let Some(product) = &state.product {
        product.cancel_app_jobs(user.id.as_str(), &miniapp_id).await;
    }
    Ok(Json(ApiResponse::ok(library)))
}

async fn call_surface_bridge(
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(body): Json<PluginRuntimeSurfaceBridgeHttpRequest>,
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
    State(state): State<PluginRuntimeM1RouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(miniapp_id): Path<String>,
    Json(request): Json<ClosePluginRuntimeSurfaceRequest>,
) -> Result<Json<ApiResponse<bool>>, AppError> {
    require_route_id("plugin_id", &miniapp_id, &request.miniapp_id)?;
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
    State(state): State<PluginRuntimeM1RouterState>,
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

pub(super) fn application_error(error: PluginRuntimeM1ApplicationError) -> AppError {
    match error {
        PluginRuntimeM1ApplicationError::Invalid(message) => {
            AppError::BadRequest(format!("Plugin input is invalid: {message}"))
        }
        PluginRuntimeM1ApplicationError::NotFound => {
            AppError::NotFound("Plugin".to_owned())
        }
        PluginRuntimeM1ApplicationError::Runtime(message) => {
            AppError::Internal(format!("Plugin runtime failed: {message}"))
        }
        PluginRuntimeM1ApplicationError::Database(error) => error.into(),
    }
}

#[cfg(test)]
#[path = "plugin_runtime_tests.rs"]
mod tests;
