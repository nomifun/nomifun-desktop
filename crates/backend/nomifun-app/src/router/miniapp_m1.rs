use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use nomifun_api_types::{
    ApiResponse, CreateMiniAppProjectRequest, MiniAppLibraryResponseDto,
    MiniAppWorkshopDto,
};
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;
use nomifun_miniapp_platform::{
    MiniAppM1ApplicationError, MiniAppM1ApplicationService,
};

#[derive(Clone)]
pub struct MiniAppM1RouterState {
    application: Arc<MiniAppM1ApplicationService>,
}

impl MiniAppM1RouterState {
    pub(crate) fn new(
        application: Arc<MiniAppM1ApplicationService>,
    ) -> Self {
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

fn application_error(error: MiniAppM1ApplicationError) -> AppError {
    match error {
        MiniAppM1ApplicationError::Invalid(message) => {
            AppError::BadRequest(format!("MiniApp input is invalid: {message}"))
        }
        MiniAppM1ApplicationError::NotFound => {
            AppError::NotFound("MiniApp".to_owned())
        }
        MiniAppM1ApplicationError::Database(error) => error.into(),
    }
}

#[cfg(test)]
#[path = "miniapp_m1_tests.rs"]
mod tests;
