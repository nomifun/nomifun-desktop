//! Agent-related API routes.
//!
//! Endpoints:
//!
//! - `GET  /api/agents`                        — list available agents
//! - `POST /api/agents/refresh`                — refresh agent availability
//! - `POST /api/agents/provider-health-check`  — probe a model provider connection

use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Json, State};
use axum::routing::{get, post};

use nomifun_api_types::{
    AgentMetadata, ApiResponse, ProviderHealthCheckRequest, ProviderHealthCheckResponse,
};
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;

use crate::routes::state::AgentRouterState;

pub fn agent_routes(state: AgentRouterState) -> Router {
    Router::new()
        .route("/api/agents", get(list_agents))
        .route("/api/agents/refresh", post(refresh_agents))
        .route("/api/agents/provider-health-check", post(provider_health_check))
        .with_state(state)
}

async fn list_agents(
    State(state): State<AgentRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<AgentMetadata>>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.list_agents().await?)))
}

async fn refresh_agents(
    State(state): State<AgentRouterState>,
    Extension(_user): Extension<CurrentUser>,
) -> Result<Json<ApiResponse<Vec<AgentMetadata>>>, AppError> {
    Ok(Json(ApiResponse::ok(state.service.refresh_agents().await?)))
}

async fn provider_health_check(
    State(state): State<AgentRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<ProviderHealthCheckRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<ProviderHealthCheckResponse>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    Ok(Json(ApiResponse::ok(state.service.provider_health_check(req).await?)))
}
