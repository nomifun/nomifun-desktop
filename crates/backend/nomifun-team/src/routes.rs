use std::sync::Arc;

use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Json, Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};

use nomifun_api_types::{
    AddAgentRequest, ApiResponse, CreateTeamRequest, RenameAgentRequest, RenameTeamRequest, SendAgentMessageRequest,
    SendTeamMessageRequest, SetModeRequest, TeamAgentResponse, TeamListResponse, TeamResponse,
};
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;

use crate::service::TeamSessionService;

#[derive(Clone)]
pub struct TeamRouterState {
    pub service: Arc<TeamSessionService>,
}

pub fn team_routes(state: TeamRouterState) -> Router {
    Router::new()
        .route("/api/teams", post(create_team).get(list_teams))
        .route("/api/teams/{id}", get(get_team).delete(remove_team))
        .route("/api/teams/{id}/name", axum::routing::patch(rename_team))
        .route("/api/teams/{id}/agents", post(add_agent))
        .route("/api/teams/{id}/agents/{slot_id}", axum::routing::delete(remove_agent))
        .route(
            "/api/teams/{id}/agents/{slot_id}/name",
            axum::routing::patch(rename_agent),
        )
        .route("/api/teams/{id}/messages", post(send_message))
        .route("/api/teams/{id}/agents/{slot_id}/messages", post(send_message_to_agent))
        .route("/api/teams/{id}/session", post(ensure_session).delete(stop_session))
        .route("/api/teams/{id}/session-mode", post(set_session_mode))
        .with_state(state)
}

async fn create_team(
    State(state): State<TeamRouterState>,
    Extension(user): Extension<CurrentUser>,
    body: Result<Json<CreateTeamRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ApiResponse<TeamResponse>>), AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let team = state.service.create_team(&user.id, req).await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(team))))
}

async fn list_teams(State(state): State<TeamRouterState>) -> Result<Json<ApiResponse<TeamListResponse>>, AppError> {
    let teams = state.service.list_teams().await?;
    Ok(Json(ApiResponse::ok(teams)))
}

async fn get_team(
    State(state): State<TeamRouterState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<TeamResponse>>, AppError> {
    let team = state.service.get_team(&id).await?;
    Ok(Json(ApiResponse::ok(team)))
}

async fn remove_team(
    State(state): State<TeamRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    state.service.remove_team(&user.id, &id).await?;
    Ok(Json(ApiResponse::success()))
}

async fn rename_team(
    State(state): State<TeamRouterState>,
    Path(id): Path<String>,
    body: Result<Json<RenameTeamRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state.service.rename_team(&id, &req.name).await?;
    Ok(Json(ApiResponse::success()))
}

#[derive(serde::Deserialize)]
struct AgentPathParams {
    id: String,
    slot_id: String,
}

async fn add_agent(
    State(state): State<TeamRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
    body: Result<Json<AddAgentRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ApiResponse<TeamAgentResponse>>), AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    let agent = state.service.add_agent(&user.id, &id, req).await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(agent))))
}

async fn remove_agent(
    State(state): State<TeamRouterState>,
    Extension(user): Extension<CurrentUser>,
    Path(params): Path<AgentPathParams>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    state
        .service
        .remove_agent(&user.id, &params.id, &params.slot_id)
        .await?;
    Ok(Json(ApiResponse::success()))
}

async fn rename_agent(
    State(state): State<TeamRouterState>,
    Path(params): Path<AgentPathParams>,
    body: Result<Json<RenameAgentRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state
        .service
        .rename_agent(&params.id, &params.slot_id, &req.name)
        .await?;
    Ok(Json(ApiResponse::success()))
}

async fn send_message(
    State(state): State<TeamRouterState>,
    Path(id): Path<String>,
    body: Result<Json<SendTeamMessageRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state.service.send_message(&id, &req.content, req.files).await?;
    Ok(Json(ApiResponse::success()))
}

async fn send_message_to_agent(
    State(state): State<TeamRouterState>,
    Path(params): Path<AgentPathParams>,
    body: Result<Json<SendAgentMessageRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state
        .service
        .send_message_to_agent(&params.id, &params.slot_id, &req.content, req.files)
        .await?;
    Ok(Json(ApiResponse::success()))
}

async fn set_session_mode(
    State(state): State<TeamRouterState>,
    Path(id): Path<String>,
    body: Result<Json<SetModeRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state.service.set_session_mode(&id, &req.mode).await?;
    Ok(Json(ApiResponse::success()))
}

async fn ensure_session(
    State(state): State<TeamRouterState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    state.service.ensure_session(&id).await?;
    Ok(Json(ApiResponse::success()))
}

async fn stop_session(
    State(state): State<TeamRouterState>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    state.service.stop_session(&id);
    Ok(Json(ApiResponse::success()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_router_state_is_clone() {
        fn assert_clone<T: Clone>() {}
        assert_clone::<TeamRouterState>();
    }
}
