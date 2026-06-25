//! Webhook HTTP routes. Handlers do request/response transformation only; all
//! logic lives in `WebhookService`. Auth is layered externally in nomifun-app
//! (mirrors the requirement / idmm routes).

use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Json, Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};

use nomifun_api_types::{
    ApiResponse, CreateWebhookRequest, TagSetting, UpdateWebhookRequest, UpsertTagSettingRequest, Webhook,
};
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;

use crate::state::WebhookRouterState;

pub fn webhook_routes(state: WebhookRouterState) -> Router {
    Router::new()
        .route("/api/webhooks", get(list_webhooks).post(create_webhook))
        .route(
            "/api/webhooks/{id}",
            get(get_webhook).put(update_webhook).delete(delete_webhook),
        )
        .route("/api/webhooks/{id}/test", post(test_webhook))
        .route("/api/tags/{tag}/settings", get(get_tag_setting).put(upsert_tag_setting))
        .with_state(state)
}

/// Parse the `{id}` path segment (always a string on the wire) into the i64
/// webhook primary key, surfacing a clean 400 on malformed input.
fn parse_id(id: &str) -> Result<i64, AppError> {
    id.parse::<i64>()
        .map_err(|_| AppError::BadRequest(format!("invalid webhook id: {id}")))
}

async fn list_webhooks(
    State(state): State<WebhookRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<Webhook>>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.list().await?)))
}

async fn get_webhook(
    State(state): State<WebhookRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Webhook>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.get(parse_id(&id)?).await?)))
}

async fn create_webhook(
    State(state): State<WebhookRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<CreateWebhookRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ApiResponse<Webhook>>), AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let created = state.service.create(req).await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(created))))
}

async fn update_webhook(
    State(state): State<WebhookRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<String>,
    body: Result<Json<UpdateWebhookRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<Webhook>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(Json(ApiResponse::ok(state.service.update(parse_id(&id)?, req).await?)))
}

async fn delete_webhook(
    State(state): State<WebhookRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    state.service.delete(parse_id(&id)?).await?;
    Ok(Json(ApiResponse::success()))
}

async fn test_webhook(
    State(state): State<WebhookRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    state.service.test(parse_id(&id)?).await?;
    Ok(Json(ApiResponse::success()))
}

async fn get_tag_setting(
    State(state): State<WebhookRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(tag): Path<String>,
) -> Result<Json<ApiResponse<TagSetting>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.get_tag_setting(&tag).await?)))
}

async fn upsert_tag_setting(
    State(state): State<WebhookRouterState>,
    Extension(_user): Extension<CurrentUser>,
    Path(tag): Path<String>,
    body: Result<Json<UpsertTagSettingRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<TagSetting>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(Json(ApiResponse::ok(
        state.service.upsert_tag_setting(&tag, req).await?,
    )))
}
