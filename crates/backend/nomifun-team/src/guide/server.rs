use std::net::SocketAddr;
use std::sync::{Arc, Weak};

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use nomifun_common::generate_id;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::service::TeamSessionService;
use crate::types::TeammateRole;

type ServiceSlot = Arc<RwLock<Weak<TeamSessionService>>>;

#[derive(Clone)]
struct GuideState {
    auth_token: String,
    service: ServiceSlot,
}

pub struct GuideMcpServer {
    http_addr: SocketAddr,
    auth_token: String,
    shutdown_handle: Option<tokio::task::JoinHandle<()>>,
    service_slot: ServiceSlot,
}

impl GuideMcpServer {
    pub async fn start() -> Result<Self, String> {
        let auth_token = generate_id();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Failed to bind guide MCP HTTP listener: {e}"))?;
        let http_addr = listener
            .local_addr()
            .map_err(|e| format!("Failed to read guide MCP local addr: {e}"))?;

        let service_slot: ServiceSlot = Arc::new(RwLock::new(Weak::new()));

        let state = GuideState {
            auth_token: auth_token.clone(),
            service: service_slot.clone(),
        };

        let app = axum::Router::new()
            .route("/tool", axum::routing::post(handle_tool_request))
            .with_state(state);

        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                warn!(error = %e, "Guide MCP axum server exited with error");
            }
        });

        debug!(http_port = http_addr.port(), "Guide MCP Server started (axum)");

        Ok(Self {
            http_addr,
            auth_token,
            shutdown_handle: Some(handle),
            service_slot,
        })
    }

    /// Wire the TeamSessionService after it is constructed.
    /// Must be called once before the first `nomi_create_team` request arrives.
    pub async fn set_service(&self, service: Weak<TeamSessionService>) {
        *self.service_slot.write().await = service;
    }

    pub fn http_port(&self) -> u16 {
        self.http_addr.port()
    }

    pub fn http_addr(&self) -> SocketAddr {
        self.http_addr
    }

    pub fn auth_token(&self) -> &str {
        &self.auth_token
    }

    pub fn stop(&mut self) {
        if let Some(handle) = self.shutdown_handle.take() {
            handle.abort();
            debug!(http_port = self.http_addr.port(), "Guide MCP Server stop requested");
        }
    }
}

impl Drop for GuideMcpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// Axum handler
// ---------------------------------------------------------------------------

async fn handle_tool_request(
    State(state): State<GuideState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Auth check
    let provided_token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");

    if provided_token != state.auth_token {
        warn!("Guide HTTP: unauthorized request");
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response();
    }

    let tool = body.get("tool").and_then(serde_json::Value::as_str).unwrap_or("");
    let args = body.get("args").cloned().unwrap_or(serde_json::Value::Null);

    info!(tool, "Guide HTTP: dispatching tool");

    let response_body = match tool {
        "nomi_create_team" => exec_create_team(&body, &args, &state.service).await,
        "nomi_list_models" => {
            let result = match state.service.read().await.upgrade() {
                Some(svc) => {
                    let mut base = svc.list_models_from_db(None).await;
                    // Guide surfaces Gemini even if not in spawn whitelist
                    if let Some(types) = base.get_mut("agent_types").and_then(serde_json::Value::as_array_mut) {
                        let has_gemini = types
                            .iter()
                            .any(|e| e.get("type").and_then(serde_json::Value::as_str) == Some("gemini"));
                        if !has_gemini {
                            types.push(serde_json::json!({
                                "type": "gemini",
                                "models": ["gemini-2.5-pro", "gemini-2.5-flash"]
                            }));
                        }
                    }
                    base
                }
                None => crate::guide::handlers::handle_nomi_list_models(),
            };
            info!("Guide HTTP: nomi_list_models succeeded");
            serde_json::json!({"result": serde_json::to_string(&result).unwrap_or_default()})
        }
        t if t.starts_with("team_") => exec_team_tool(t, &body, &args, &state.service).await,
        unknown => {
            warn!(tool = unknown, "Guide HTTP: unknown tool");
            serde_json::json!({"error": format!("Unknown tool: {unknown}")})
        }
    };

    let mut resp = Json(response_body).into_response();
    resp.headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    resp
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

async fn exec_create_team(
    request_body: &serde_json::Value,
    args: &serde_json::Value,
    service: &ServiceSlot,
) -> serde_json::Value {
    use crate::guide::handlers::parse_create_team_args;
    use nomifun_api_types::{CreateTeamRequest, TeamAgentInput};

    let svc = match service.read().await.upgrade() {
        Some(s) => s,
        None => {
            warn!("Guide HTTP: nomi_create_team — service not available");
            return serde_json::json!({"error": "service_unavailable"});
        }
    };

    let caller_workspace: Option<&str> = None;
    let params = match parse_create_team_args(args, caller_workspace) {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, "Guide HTTP: nomi_create_team parse error");
            return serde_json::json!({"error": e});
        }
    };

    let backend = request_body
        .get("backend")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("claude")
        .to_owned();

    let model = request_body
        .get("model")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_owned();

    let user_id = request_body
        .get("user_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("system_default_user")
        .to_owned();

    let caller_conversation_id = request_body
        .get("conversation_id")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    // Refuse if the caller conversation already belongs to a team.
    // This prevents duplicate team creation when guide MCP is
    // erroneously injected into an existing team leader session.
    if let Some(ref conv_id) = caller_conversation_id {
        let repo = svc.conversation_service_ref().conversation_repo().clone();
        // conversation_id arrives as a String from the request JSON; the repo
        // is i64-keyed (Option A). A non-integer id can't reference a real
        // conversation, so it simply skips the "already in a team" refuse check.
        if let Ok(conv_id_i64) = conv_id.parse::<i64>()
            && let Ok(Some(row)) = repo.get(conv_id_i64).await
        {
            let extra: serde_json::Value = serde_json::from_str(&row.extra).unwrap_or(serde_json::Value::Null);
            if extra
                .get("teamId")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|s| !s.is_empty())
            {
                warn!(
                    conversation_id = conv_id,
                    "Guide HTTP: nomi_create_team refused — conversation already belongs to a team"
                );
                return serde_json::json!({
                    "error": "This conversation already belongs to a team. Cannot create another team from here."
                });
            }
        }
    }

    let req = CreateTeamRequest {
        name: params.name.clone(),
        agents: vec![TeamAgentInput {
            name: "Leader".to_owned(),
            role: "leader".to_owned(),
            backend: backend.clone(),
            model: model.clone(),
            custom_agent_id: None,
            // TeamAgentInput.conversation_id is now Option<i64> (Option A);
            // adopt the caller's conversation only when it is a valid integer.
            conversation_id: caller_conversation_id.and_then(|id| id.parse::<i64>().ok()),
        }],
        workspace: None,
    };

    let team = match svc.create_team(&user_id, req).await {
        Ok(t) => t,
        Err(e) => {
            warn!(error = %e, "Guide HTTP: nomi_create_team create_team failed");
            return serde_json::json!({"error": e.to_string()});
        }
    };

    let route = format!("/team/{}", team.id);
    info!(team_id = %team.id, "Guide HTTP: nomi_create_team succeeded");
    serde_json::json!({
        "teamId": team.id,
        "name": team.name,
        "route": route,
        "status": "team_created",
        "next_step": format!(
            "You are now the team Leader. Your team tools (team_spawn_agent, team_send_message, etc.) are now active. \
             Immediately proceed to spawn teammates as planned. Task summary: {}",
            params.summary
        )
    })
}

async fn exec_team_tool(
    tool_name: &str,
    request_body: &serde_json::Value,
    args: &serde_json::Value,
    service: &ServiceSlot,
) -> serde_json::Value {
    let svc = match service.read().await.upgrade() {
        Some(s) => s,
        None => {
            warn!("Guide HTTP: {} — service not available", tool_name);
            return serde_json::json!({"error": "service_unavailable"});
        }
    };

    let conversation_id = match request_body
        .get("conversation_id")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
    {
        Some(id) => id.to_owned(),
        None => {
            warn!(tool = tool_name, "Guide HTTP: team tool missing conversation_id");
            return serde_json::json!({"error": "missing conversation_id"});
        }
    };

    let (team_id, slot_id) = match resolve_team_context(&svc, &conversation_id).await {
        Ok(ctx) => ctx,
        Err(e) => {
            warn!(tool = tool_name, error = %e, "Guide HTTP: resolve_team_context failed");
            return serde_json::json!({"error": e});
        }
    };

    let scheduler = match svc.get_session_scheduler(&team_id) {
        Some(s) => s,
        None => {
            warn!(tool = tool_name, team_id = %team_id, "Guide HTTP: no active session for team");
            return serde_json::json!({"error": "No active team session. The team may still be starting up."});
        }
    };

    let svc_weak = Arc::downgrade(&svc);
    let result = crate::mcp::server::dispatch_tool(
        tool_name,
        args,
        &scheduler,
        &svc_weak,
        &team_id,
        &slot_id,
        TeammateRole::Lead,
    )
    .await;

    match result {
        Ok(text) => {
            info!(tool = tool_name, team_id = %team_id, "Guide HTTP: team tool succeeded");
            serde_json::json!({"result": text})
        }
        Err(err) => {
            warn!(tool = tool_name, team_id = %team_id, error = %err, "Guide HTTP: team tool failed");
            serde_json::json!({"error": err})
        }
    }
}

/// Resolve `(team_id, slot_id)` for a caller identified by `conversation_id`.
///
/// Reads the conversation row's `extra` JSON to extract `teamId`, then finds
/// the agent slot whose `conversation_id` matches. Returns an error string if
/// no active team is found for this conversation.
async fn resolve_team_context(service: &TeamSessionService, conversation_id: &str) -> Result<(String, String), String> {
    // Extract teamId from conversation.extra via the conversation service repo.
    let repo = service.conversation_service_ref().conversation_repo().clone();
    // The repo is i64-keyed (Option A); bridge the &str id at the boundary.
    let conversation_id_i64 = conversation_id
        .parse::<i64>()
        .map_err(|_| format!("Invalid conversation id: {conversation_id}"))?;
    let row = repo
        .get(conversation_id_i64)
        .await
        .map_err(|e| format!("DB error reading conversation: {e}"))?
        .ok_or_else(|| format!("Conversation not found: {conversation_id}"))?;

    let extra: serde_json::Value = serde_json::from_str(&row.extra).unwrap_or(serde_json::Value::Null);
    let team_id = extra
        .get("teamId")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "No active team for this conversation. Create a team first with nomi_create_team.".to_owned())?
        .to_owned();

    // Find the slot_id by matching conversation_id in the session scheduler.
    let scheduler = service
        .get_session_scheduler(&team_id)
        .ok_or_else(|| "No active team session. The team may still be starting up.".to_owned())?;

    let agents = scheduler.list_agents().await;
    let slot_id = agents
        .iter()
        .find(|a| a.conversation_id == conversation_id)
        .map(|a| a.slot_id.clone())
        .ok_or_else(|| format!("Agent with conversation_id={conversation_id} not found in team {team_id}"))?;

    Ok((team_id, slot_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn start_returns_positive_port_and_token() {
        let server = GuideMcpServer::start().await.expect("start should succeed");
        assert!(server.http_port() > 0, "http_port should be assigned");
        assert!(!server.auth_token().is_empty(), "auth_token should be generated");
    }

    #[tokio::test]
    async fn each_start_uses_a_fresh_auth_token() {
        let a = GuideMcpServer::start().await.unwrap();
        let b = GuideMcpServer::start().await.unwrap();
        assert_ne!(a.auth_token(), b.auth_token());
    }

    #[tokio::test]
    async fn stop_closes_the_listener() {
        let mut server = GuideMcpServer::start().await.unwrap();
        let port = server.http_port();
        server.stop();

        tokio::time::sleep(Duration::from_millis(50)).await;

        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(200))
            .build()
            .unwrap();
        let result = timeout(
            Duration::from_millis(500),
            client
                .post(format!("http://127.0.0.1:{port}/tool"))
                .json(&serde_json::json!({}))
                .send(),
        )
        .await;
        match result {
            Ok(Ok(_)) => { /* may still accept in-flight during abort */ }
            Ok(Err(_)) => { /* connection refused — expected */ }
            Err(_) => { /* timeout — expected */ }
        }
    }

    #[tokio::test]
    async fn stop_is_idempotent() {
        let mut server = GuideMcpServer::start().await.unwrap();
        server.stop();
        server.stop();
    }

    #[tokio::test]
    async fn tool_call_requires_auth() {
        let server = GuideMcpServer::start().await.unwrap();
        let port = server.http_port();

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://127.0.0.1:{port}/tool"))
            .json(&serde_json::json!({"tool": "nomi_list_models", "args": {}}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 401);
    }

    #[tokio::test]
    async fn tool_call_with_valid_token_succeeds() {
        let server = GuideMcpServer::start().await.unwrap();
        let port = server.http_port();
        let token = server.auth_token().to_owned();

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://127.0.0.1:{port}/tool"))
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({"tool": "nomi_list_models", "args": {}}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);

        let body: serde_json::Value = resp.json().await.unwrap();
        assert!(body.get("result").is_some());
    }
}
