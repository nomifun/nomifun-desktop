use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Json, State};
use axum::routing::post;

use nomifun_api_types::{ApiResponse, TestBedrockConnectionRequest};
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;

use super::service::ConnectionTestService;

/// Router state for connection test routes.
#[derive(Clone)]
pub struct ConnectionTestRouterState {
    pub service: ConnectionTestService,
}

/// Build the connection test router.
///
/// Routes:
/// - `POST /api/bedrock/test-connection` — test AWS Bedrock credentials
///
/// All routes require authentication (applied by the caller).
pub fn connection_test_routes(state: ConnectionTestRouterState) -> Router {
    Router::new()
        .route("/api/bedrock/test-connection", post(test_bedrock))
        .with_state(state)
}

/// POST /api/bedrock/test-connection
///
/// Test AWS Bedrock credentials with a lightweight API call.
/// Returns 200 on success, 400 for validation errors, 422-equivalent for
/// invalid credentials (mapped to 400 with descriptive message).
async fn test_bedrock(
    State(state): State<ConnectionTestRouterState>,
    Extension(_user): Extension<CurrentUser>,
    body: Result<Json<TestBedrockConnectionRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    let Json(req) = body.map_err(|e| AppError::BadRequest(e.to_string()))?;
    state.service.test_bedrock_connection(req.bedrock_config).await?;
    Ok(Json(ApiResponse::message("Connection successful")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_router_state_clone() {
        let state = ConnectionTestRouterState {
            service: ConnectionTestService::new(reqwest::Client::new()),
        };
        let _cloned = state.clone();
    }

    #[test]
    fn test_router_construction() {
        let state = ConnectionTestRouterState {
            service: ConnectionTestService::new(reqwest::Client::new()),
        };
        let _router = connection_test_routes(state);
    }
}
