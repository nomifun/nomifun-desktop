use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Json, Path, State, RawQuery};
use axum::Extension;
use std::sync::Arc;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};

use nomifun_api_types::{
    ApiResponse, BatchImportMcpServersRequest, CreateMcpServerRequest, DetectedMcpServerResponse, ErrorResponse,
    McpConnectionTestErrorCode, McpConnectionTestResult, McpServerId, McpServerResponse,
    McpAuthStatus, McpOAuthAttempt, McpOAuthStartRequest, TestMcpConnectionRequest,
    UpdateMcpServerRequest,
};

use crate::connection_test::McpConnectionTestService;
use crate::McpError;
use crate::{auth_service::McpAuthService, oauth_failure::McpAuthFailure};
use crate::service::McpConfigService;
use crate::sync_service::McpSyncService;
use crate::types::McpServerTransport;

// ---------------------------------------------------------------------------
// Router state
// ---------------------------------------------------------------------------

/// Shared state for MCP route handlers.
#[derive(Clone)]
pub struct McpRouterState {
    pub config_service: McpConfigService,
    pub sync_service: McpSyncService,
    pub connection_test_service: McpConnectionTestService,
    pub auth_service: Arc<McpAuthService>,
}

/// Injected only by the application's authenticated local-shell composition.
#[derive(Clone, Copy)]
pub struct McpNativeCallback;

// ---------------------------------------------------------------------------
// Router builder
// ---------------------------------------------------------------------------

/// Build the MCP router with all `/api/mcp/*` routes.
///
/// Includes CRUD routes, agent config detection, connection tests, and OAuth.
/// All routes require authentication (applied by the caller).
pub fn mcp_routes(state: McpRouterState) -> Router {
    Router::new()
        .route("/api/mcp/servers", get(list_servers).post(add_server))
        .route("/api/mcp/servers/import", post(batch_import))
        .route("/api/mcp/catalog/refresh", post(refresh_catalog))
        .route(
            "/api/mcp/servers/{mcp_server_id}",
            get(get_server).put(edit_server).delete(delete_server),
        )
        .route(
            "/api/mcp/servers/{mcp_server_id}/toggle",
            post(toggle_server),
        )
        // Connection test route
        .route("/api/mcp/test-connection", post(test_connection))
        // Agent config discovery route
        .route("/api/mcp/agent-configs", get(get_agent_configs))
        // OAuth routes
        .route("/api/mcp/servers/{mcp_server_id}/auth", get(auth_status).delete(auth_logout))
        .route("/api/mcp/servers/{mcp_server_id}/auth/attempts", post(auth_start))
        .route("/api/mcp/servers/{mcp_server_id}/auth/attempts/{attempt_id}", get(auth_attempt).delete(auth_cancel))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// CRUD Handlers
// ---------------------------------------------------------------------------

async fn refresh_catalog(
    State(state): State<McpRouterState>,
) -> Result<Json<ApiResponse<()>>, McpError> {
    state.config_service.refresh_catalog().await?;
    Ok(Json(ApiResponse::success()))
}

/// `GET /api/mcp/servers` — list all MCP servers.
async fn list_servers(
    State(state): State<McpRouterState>,
) -> Result<Json<ApiResponse<Vec<McpServerResponse>>>, McpError> {
    let servers = state.config_service.list_servers().await?;
    Ok(Json(ApiResponse::ok(servers)))
}

/// `GET /api/mcp/servers/:mcp_server_id` — get a single MCP server.
async fn get_server(
    State(state): State<McpRouterState>,
    Path(mcp_server_id): Path<McpServerId>,
) -> Result<Json<ApiResponse<McpServerResponse>>, McpError> {
    let server = state.config_service.get_server(&mcp_server_id).await?;
    Ok(Json(ApiResponse::ok(server)))
}

/// `POST /api/mcp/servers` — create (or upsert by name) an MCP server.
async fn add_server(
    State(state): State<McpRouterState>,
    body: Result<Json<CreateMcpServerRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<ApiResponse<McpServerResponse>>), McpError> {
    let Json(req) = body.map_err(|e| McpError::InvalidEdit(e.to_string()))?;
    let server = state.config_service.add_server(req).await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::ok(server))))
}

/// `PUT /api/mcp/servers/:mcp_server_id` — partial update an MCP server.
async fn edit_server(
    State(state): State<McpRouterState>,
    Path(mcp_server_id): Path<McpServerId>,
    body: Result<Json<UpdateMcpServerRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<McpServerResponse>>, McpError> {
    let Json(req) = body.map_err(|e| McpError::InvalidEdit(e.to_string()))?;
    let server = state
        .config_service
        .edit_server(&mcp_server_id, req)
        .await?;
    Ok(Json(ApiResponse::ok(server)))
}

/// `DELETE /api/mcp/servers/:mcp_server_id` — delete an MCP server.
async fn delete_server(
    State(state): State<McpRouterState>,
    Path(mcp_server_id): Path<McpServerId>,
) -> Result<Json<ApiResponse<()>>, McpError> {
    state.config_service.delete_server(&mcp_server_id).await?;
    Ok(Json(ApiResponse::success()))
}

/// `POST /api/mcp/servers/:mcp_server_id/toggle` — toggle enabled state.
async fn toggle_server(
    State(state): State<McpRouterState>,
    Path(mcp_server_id): Path<McpServerId>,
) -> Result<Json<ApiResponse<McpServerResponse>>, McpError> {
    let server = state.config_service.toggle_server(&mcp_server_id).await?;
    Ok(Json(ApiResponse::ok(server)))
}

/// `POST /api/mcp/servers/import` — batch import MCP servers.
async fn batch_import(
    State(state): State<McpRouterState>,
    body: Result<Json<BatchImportMcpServersRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<Vec<McpServerResponse>>>, McpError> {
    let Json(req) = body.map_err(|e| McpError::InvalidEdit(e.to_string()))?;
    let servers = state.config_service.batch_import(req).await?;
    Ok(Json(ApiResponse::ok(servers)))
}

// ---------------------------------------------------------------------------
// Connection Test Handler
// ---------------------------------------------------------------------------

/// `POST /api/mcp/test-connection` — test MCP server connectivity.
///
/// Creates a temporary MCP client, connects, lists tools, and closes.
async fn test_connection(
    State(state): State<McpRouterState>,
    body: Result<Json<TestMcpConnectionRequest>, JsonRejection>,
) -> Result<Response, McpError> {
    let Json(req) = body.map_err(|e| McpError::InvalidEdit(e.to_string()))?;
    let transport = McpServerTransport::from(req.transport);
    let probe = match req.mcp_server_id.as_ref() {
        Some(id) => state.config_service.begin_probe(id, &req.name, &transport).await?,
        None => None,
    };
    let auth_revision = if probe.is_some() {
        state.auth_service.binding_revision(req.mcp_server_id.as_ref().expect("saved probe").as_str())
            .await.map_err(McpError::Auth)?
    } else { String::new() };
    let result = if probe.is_some() {
        state.connection_test_service.test_saved_connection(req.mcp_server_id.as_ref().expect("saved probe"), &req.name, &transport).await
    } else {
        state.connection_test_service.test_connection(&req.name, &transport).await
    };
    if let Some(probe) = probe {
        state.config_service.finish_authenticated_probe(probe, &result, auth_revision).await?;
    }
    if result.success || result.needs_auth == Some(true) {
        return Ok(Json(ApiResponse::ok(result)).into_response());
    }

    Ok(connection_test_error_response(result))
}

fn connection_test_error_response(result: McpConnectionTestResult) -> Response {
    let status = result.code.map(connection_test_failure_status).unwrap_or(StatusCode::BAD_GATEWAY);
    let error = result.error.clone().unwrap_or_else(|| "MCP connection test failed".to_owned());
    let code = result
        .code
        .map(McpConnectionTestErrorCode::as_str)
        .unwrap_or("MCP_CONNECTION_FAILED");

    (
        status,
        Json(ErrorResponse::new_with_details(error, code, result.details.clone())),
    )
        .into_response()
}

fn connection_test_failure_status(code: McpConnectionTestErrorCode) -> StatusCode {
    match code {
        McpConnectionTestErrorCode::CommandNotFound
        | McpConnectionTestErrorCode::CommandPermissionDenied
        | McpConnectionTestErrorCode::CommandStartFailed => StatusCode::UNPROCESSABLE_ENTITY,
        McpConnectionTestErrorCode::Timeout => StatusCode::GATEWAY_TIMEOUT,
        McpConnectionTestErrorCode::ConnectionFailed
        | McpConnectionTestErrorCode::HttpError
        | McpConnectionTestErrorCode::RpcError
        | McpConnectionTestErrorCode::ProtocolError => StatusCode::BAD_GATEWAY,
    }
}

// ---------------------------------------------------------------------------
// Agent Sync Handlers
// ---------------------------------------------------------------------------

/// `GET /api/mcp/agent-configs` — scan all installed Agent CLIs
/// and return their current MCP server configurations.
async fn get_agent_configs(
    State(state): State<McpRouterState>,
) -> Result<Json<ApiResponse<Vec<DetectedMcpServerResponse>>>, McpError> {
    let configs = state.sync_service.get_agent_configs().await?;
    Ok(Json(ApiResponse::ok(configs)))
}

// ---------------------------------------------------------------------------
// OAuth Handlers
// ---------------------------------------------------------------------------

async fn auth_status(
    State(state): State<McpRouterState>, Path(id): Path<McpServerId>,
) -> Result<Json<ApiResponse<McpAuthStatus>>, McpAuthFailure> {
    Ok(Json(ApiResponse::ok(state.auth_service.status(id.as_str()).await?)))
}

async fn auth_start(
    State(state): State<McpRouterState>, Path(id): Path<McpServerId>,
    native: Option<Extension<McpNativeCallback>>,
    body: Result<Json<McpOAuthStartRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<McpOAuthAttempt>>, Response> {
    let Json(req) = body.map_err(|_| McpAuthFailure::new(
        nomifun_api_types::McpAuthErrorCode::InvalidConfiguration,
        nomifun_api_types::McpAuthStage::Authorization,
        "Invalid MCP OAuth login request",
    ).into_response())?;
    let result = state.auth_service.start(id.as_str(), req, native.is_some()).await.map_err(IntoResponse::into_response)?;
    Ok(Json(ApiResponse::ok(result)))
}

async fn auth_logout(
    State(state): State<McpRouterState>, Path(id): Path<McpServerId>,
) -> Result<Json<ApiResponse<()>>, McpAuthFailure> {
    state.auth_service.logout(id.as_str()).await?;
    Ok(Json(ApiResponse::success()))
}

async fn auth_attempt(
    State(state): State<McpRouterState>, Path((id, attempt_id)): Path<(McpServerId, String)>,
) -> Result<Json<ApiResponse<McpOAuthAttempt>>, McpAuthFailure> {
    Ok(Json(ApiResponse::ok(state.auth_service.attempt(id.as_str(), &attempt_id).await?)))
}

async fn auth_cancel(
    State(state): State<McpRouterState>, Path((id, attempt_id)): Path<(McpServerId, String)>,
) -> Result<Json<ApiResponse<()>>, McpAuthFailure> {
    state.auth_service.cancel_attempt(id.as_str(), &attempt_id).await?;
    Ok(Json(ApiResponse::success()))
}

/// Public callback ingress is bound to an unpredictable, one-use attempt state.
/// A callback cannot select the owner, server, issuer, or token destination.
pub fn oauth_callback_routes(auth: Arc<McpAuthService>) -> Router {
    Router::new().route("/api/mcp/oauth/callback", get(web_callback))
        .with_state(auth)
}

async fn web_callback(State(auth): State<Arc<McpAuthService>>, RawQuery(query): RawQuery) -> Response {
    crate::auth_service::callback_response(auth.complete_callback(None, query.as_deref().unwrap_or_default()).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runtime_failure_response_preserves_real_reason_without_download_choice() {
        let result = McpConnectionTestResult {
            success: false, tools: None, error: Some("Bun is unavailable".into()),
            code: Some(McpConnectionTestErrorCode::CommandStartFailed),
            details: Some(serde_json::json!({
                "owner_code":"MCP_RUNTIME_UNAVAILABLE",
                "runtime":"bun", "phase":"runtime_resolution", "retryable":true,
            })),
            needs_auth: None, auth_method: None,
        };
        let response = connection_test_error_response(result);
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["code"], "MCP_COMMAND_START_FAILED");
        assert_eq!(body["details"]["runtime"], "bun");
        assert_eq!(body["details"]["phase"], "runtime_resolution");
    }
}
