//! In-process HTTP MCP server exposing requirement *declaration* tools to ACP
//! agent sessions (claude / codex / gemini CLIs).
//!
//! ## Why this exists
//!
//! AutoWork drives ACP sessions, but ACP CLIs have no in-process tool bus we
//! can register `RequirementCompleteTool` into (only the nomi engine does).
//! Without a declaration channel, a clean turn that did NOT actually finish the
//! requirement is silently recorded as `done` — the original "失败却标成成功"
//! bug. This server gives ACP agents the SAME `requirement_complete` /
//! `requirement_update_status` surface the nomi engine has natively, so the
//! AutoWork runner can park an un-declared clean turn as `needs_review` instead of
//! assuming success (`expects_verdict`).
//!
//! This is the in-process HTTP half. ACP CLIs spawn a SEPARATE stdio process
//! (`nomicore mcp-requirement-stdio`) that cannot share this process's
//! `RequirementService`; it forwards each tool call back here as an
//! authenticated `POST /tool`. The transport is stdio because claude / codex /
//! gemini advertise stdio-only MCP capabilities (HTTP/SSE servers are dropped
//! by the ACP capability filter), so a direct-HTTP injection would never reach
//! them.
//!
//! ## Security
//!
//! The process-local issuer never leaves this server. Each bridge child receives
//! a renewable lease bootstrap: short-lived access binds user, session, tools,
//! and requirement owner scope, while the renewal proof names no mutable scope.
//! Identity is derived only from verified claims; loose body fields do not exist
//! in the protocol.

use std::net::SocketAddr;
use std::sync::{Arc, Weak};

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use nomifun_api_types::{
    REQUIREMENT_CAPABILITY_DOMAIN, RequirementCapabilityClaims,
    RequirementCapabilityScope, RequirementMcpConfig, RequirementStatus,
};
use nomifun_common::{
    LOOPBACK_CAPABILITY_RENEW_PATH, LOOPBACK_CAPABILITY_REVOKE_PATH,
    LoopbackCapabilityIssuer, LoopbackCapabilityRenewalRequest,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::service::RequirementService;

/// Late-bound handle to the singleton `RequirementService`. Held as a `Weak` so
/// the server never keeps the service alive on its own (matches the guide
/// server's slot pattern). Wired via [`RequirementMcpServer::set_service`].
type ServiceSlot = Arc<RwLock<Weak<RequirementService>>>;

#[derive(Clone)]
struct ReqMcpState {
    issuer: Arc<LoopbackCapabilityIssuer>,
    service: ServiceSlot,
}

/// In-process HTTP MCP server for requirement declaration tools.
pub struct RequirementMcpServer {
    http_addr: SocketAddr,
    issuer: Arc<LoopbackCapabilityIssuer>,
    shutdown_handle: Option<tokio::task::JoinHandle<()>>,
    service_slot: ServiceSlot,
}

impl RequirementMcpServer {
    /// Bind a fresh `127.0.0.1:0` listener, create a process-local issuer, and
    /// start serving capability lifecycle routes plus `POST /tool`. The service
    /// must be wired separately via [`set_service`](Self::set_service) before
    /// the first tool call arrives.
    pub async fn start() -> Result<Self, String> {
        let issuer = Arc::new(LoopbackCapabilityIssuer::random()?);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Failed to bind requirement MCP HTTP listener: {e}"))?;
        let http_addr = listener
            .local_addr()
            .map_err(|e| format!("Failed to read requirement MCP local addr: {e}"))?;

        let service_slot: ServiceSlot = Arc::new(RwLock::new(Weak::new()));

        let state = ReqMcpState {
            issuer: issuer.clone(),
            service: service_slot.clone(),
        };

        let app = axum::Router::new()
            .route("/tool", axum::routing::post(handle_tool_request))
            .route(
                LOOPBACK_CAPABILITY_RENEW_PATH,
                axum::routing::post(handle_capability_renew),
            )
            .route(
                LOOPBACK_CAPABILITY_REVOKE_PATH,
                axum::routing::post(handle_capability_revoke),
            )
            .with_state(state);

        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                warn!(error = %e, "Requirement MCP axum server exited with error");
            }
        });

        debug!(http_port = http_addr.port(), "Requirement MCP Server started (axum)");

        Ok(Self {
            http_addr,
            issuer,
            shutdown_handle: Some(handle),
            service_slot,
        })
    }

    /// Wire the singleton `RequirementService` after it is constructed. Must be
    /// called once before the first tool request arrives.
    pub async fn set_service(&self, service: Weak<RequirementService>) {
        *self.service_slot.write().await = service;
    }

    pub fn http_port(&self) -> u16 {
        self.http_addr.port()
    }

    /// Build the process-private issuer config consumed by the Agent/Terminal
    /// assemblers. It is deliberately non-serializable and redacts its issuer.
    pub fn issuer_config(&self, binary_path: String) -> RequirementMcpConfig {
        RequirementMcpConfig::from_issuer(
            self.http_addr.port(),
            self.issuer.clone(),
            binary_path,
        )
    }

    pub fn stop(&mut self) {
        if let Some(handle) = self.shutdown_handle.take() {
            handle.abort();
            debug!(http_port = self.http_addr.port(), "Requirement MCP Server stop requested");
        }
    }
}

impl Drop for RequirementMcpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// Axum handler
// ---------------------------------------------------------------------------

async fn handle_tool_request(
    State(state): State<ReqMcpState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let presented_token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");

    let claims: RequirementCapabilityClaims = match body
        .get("session")
        .cloned()
        .and_then(|value| serde_json::from_value::<RequirementCapabilityClaims>(value).ok())
    {
        Some(claims)
            if state
                .issuer
                .verify_access(
                    REQUIREMENT_CAPABILITY_DOMAIN,
                    &claims,
                    presented_token,
                )
                .is_ok()
                && claims.scope.validate(&claims.session).is_ok() => claims,
        _ => {
            warn!("Requirement MCP: rejected invalid, expired, or missing scoped capability");
            return (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"})))
                .into_response();
        }
    };

    let tool = body.get("tool").and_then(Value::as_str).unwrap_or("");
    if !claims.allows(tool) {
        warn!(tool, "Requirement MCP: tool is outside signed capability scope");
        return (StatusCode::FORBIDDEN, Json(json!({"error": "forbidden"})))
            .into_response();
    }
    let args = body.get("args").cloned().unwrap_or(Value::Null);

    let svc = match state.service.read().await.upgrade() {
        Some(s) => s,
        None => {
            warn!(tool, "Requirement MCP: service not available");
            return finish(json!({"error": "service_unavailable"}));
        }
    };

    info!(tool, "Requirement MCP: dispatching tool");

    let response_body = match tool {
        "requirement_complete" => exec_complete(&svc, &args, &claims).await,
        "requirement_update_status" => exec_update_status(&svc, &args, &claims).await,
        unknown => {
            warn!(tool = unknown, "Requirement MCP: unknown tool");
            json!({"error": format!("Unknown tool: {unknown}")})
        }
    };

    finish(response_body)
}

/// Renew a short-lived access credential from the process-local immutable
/// authorization. The bridge never submits user/session/tool/scope fields.
async fn handle_capability_renew(
    State(state): State<ReqMcpState>,
    Json(request): Json<LoopbackCapabilityRenewalRequest>,
) -> axum::response::Response {
    match state
        .issuer
        .renew::<RequirementCapabilityScope>(REQUIREMENT_CAPABILITY_DOMAIN, &request)
    {
        Ok(access) if access.claims.scope.validate(&access.claims.session).is_ok() => {
            Json(access).into_response()
        }
        _ => {
            warn!("Requirement MCP: rejected invalid capability renewal");
            (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"})))
                .into_response()
        }
    }
}

/// Explicit child/runtime teardown. Invalid proofs fail closed; callers treat
/// transport failure as best-effort because process exit also destroys the
/// in-memory registry.
async fn handle_capability_revoke(
    State(state): State<ReqMcpState>,
    Json(request): Json<LoopbackCapabilityRenewalRequest>,
) -> axum::response::Response {
    match state
        .issuer
        .revoke(REQUIREMENT_CAPABILITY_DOMAIN, &request)
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => {
            warn!("Requirement MCP: rejected invalid capability revocation");
            (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"})))
                .into_response()
        }
    }
}

/// Extract an integer id from a JSON value, tolerating both a JSON number and a
/// numeric string (agents occasionally stringify tool arguments). Returns
/// `None` for absent / null / non-numeric — the caller decides whether that is
/// benign.
fn json_to_i64(v: Option<&Value>) -> Option<i64> {
    match v {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// Wrap a JSON body as a response and ask the client to close the connection
/// (the stdio bridge runs with `pool_max_idle_per_host(0)` and does not reuse).
fn finish(body: Value) -> axum::response::Response {
    let mut resp = Json(body).into_response();
    resp.headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    resp
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

async fn exec_complete(
    svc: &RequirementService,
    args: &Value,
    claims: &RequirementCapabilityClaims,
) -> Value {
    let id = match json_to_i64(args.get("id")) {
        Some(id) => id,
        None => return json!({"error": "missing or non-integer required field: id"}),
    };
    let note = args
        .get("completion_note")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if let Err(e) = verify_scope(svc, id, claims).await {
        return json!({"error": e});
    }
    match svc.complete(id, note).await {
        Ok(_) => {
            info!(requirement_id = id, "Requirement MCP: requirement_complete succeeded");
            json!({"result": format!("Requirement {id} marked complete.")})
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

async fn exec_update_status(
    svc: &RequirementService,
    args: &Value,
    claims: &RequirementCapabilityClaims,
) -> Value {
    let id = match json_to_i64(args.get("id")) {
        Some(id) => id,
        None => return json!({"error": "missing or non-integer required field: id"}),
    };
    let status_str = args.get("status").and_then(Value::as_str).unwrap_or("");
    let status = match status_str {
        "in_progress" => RequirementStatus::InProgress,
        "done" => RequirementStatus::Done,
        "failed" => RequirementStatus::Failed,
        other => {
            return json!({
                "error": format!("invalid status '{other}' (expected one of: in_progress, done, failed)")
            });
        }
    };
    let note = args
        .get("note")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if let Err(e) = verify_scope(svc, id, claims).await {
        return json!({"error": e});
    }
    match svc.set_status(id, status, note).await {
        Ok(_) => {
            info!(requirement_id = id, status = status_str, "Requirement MCP: requirement_update_status succeeded");
            json!({"result": format!("Requirement {id} status set to {status_str}.")})
        }
        Err(e) => json!({"error": e.to_string()}),
    }
}

/// A declaration child can mutate only work currently owned by the exact signed
/// session and domain. Unowned work is rejected: the old caller-id-absent and
/// unclaimed fallbacks made an authenticated process-wide token a global write
/// capability.
async fn verify_scope(
    svc: &RequirementService,
    id: i64,
    claims: &RequirementCapabilityClaims,
) -> Result<(), String> {
    let caller_kind = claims.scope.owner_kind.as_str();
    let caller_id = claims.scope.owner_session_id;
    if claims.session.kind != claims.scope.owner_kind
        || claims.session.session_id != caller_id.to_string()
    {
        return Err("signed requirement scope is internally inconsistent".into());
    }
    let req = svc.get(id).await.map_err(|e| e.to_string())?;
    match (req.owner_session_id, req.owner_kind.as_deref()) {
        (Some(owner), Some("conversation")) if caller_kind == "conversation" && owner == caller_id => Ok(()),
        (Some(owner), Some("terminal")) if caller_kind == "terminal" && owner == caller_id => Ok(()),
        _ => Err(format!("requirement {id} is owned by a different session")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::RequirementEventEmitter;
    use nomifun_api_types::{AutoWorkTargetKind, CreateRequirementRequest, RequirementStatus};
    use nomifun_db::{SqliteRequirementRepository, init_database_memory};
    use nomifun_realtime::UserEventSink;

    #[derive(Default)]
    struct NoopBroadcaster;
    impl UserEventSink for NoopBroadcaster {
        fn send_to_user(
            &self,
            _user_id: &str,
            _event: nomifun_api_types::WebSocketMessage<serde_json::Value>,
        ) {
        }
    }

    /// Build a service with one requirement in tag `t`, claimed into `conv_1`
    /// (so it is `in_progress` with `conversation_id = conv_1`). Returns the
    /// service (keep it alive — the server holds only a `Weak`) and the req id.
    async fn service_with_claimed_req() -> (Arc<RequirementService>, i64) {
        let db = init_database_memory().await.unwrap();
        let repo: Arc<dyn nomifun_db::IRequirementRepository> =
            Arc::new(SqliteRequirementRepository::new(db.pool().clone()));
        let emitter = RequirementEventEmitter::new(
            Arc::new(NoopBroadcaster),
            Arc::from("system_default_user"),
        );
        sqlx::query(
            "INSERT INTO conversations (id, user_id, name, type, created_at, updated_at) \
             VALUES (1, 'system_default_user', 'Test Conv', 'acp', 0, 0)",
        )
        .execute(db.pool())
        .await
        .unwrap();
        Box::leak(Box::new(db));
        let svc = Arc::new(RequirementService::new(repo, emitter));
        let req = svc
            .create(CreateRequirementRequest {
                title: "Do X".into(),
                content: "body".into(),
                tag: "t".into(),
                order_key: None,
                status: None,
                created_by: None,
                attachments: vec![],
            })
            .await
            .unwrap();
        let claimed = svc
            .claim_next("t", 1, AutoWorkTargetKind::Conversation, 120_000)
            .await
            .unwrap()
            .expect("a pending requirement should be claimable");
        assert_eq!(claimed.id, req.id);
        (svc, req.id)
    }

    async fn started_server(svc: &Arc<RequirementService>) -> RequirementMcpServer {
        let server = RequirementMcpServer::start().await.expect("start");
        server.set_service(Arc::downgrade(svc)).await;
        server
    }

    fn conversation_child(
        server: &RequirementMcpServer,
        conversation_id: i64,
    ) -> nomifun_api_types::RequirementMcpChildConfig {
        server
            .issuer_config("/bin/nomicore".into())
            .issue_for_conversation("system_default_user", conversation_id)
            .unwrap()
    }

    fn terminal_child(
        server: &RequirementMcpServer,
        terminal_id: i64,
    ) -> nomifun_api_types::RequirementMcpChildConfig {
        server
            .issuer_config("/bin/nomicore".into())
            .issue_for_terminal("system_default_user", terminal_id)
            .unwrap()
    }

    async fn post_tool(
        server: &RequirementMcpServer,
        auth: Option<(&str, &RequirementCapabilityClaims)>,
        mut body: Value,
    ) -> (u16, Value) {
        let client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("test HTTP client should build");
        if let Some((_, claims)) = auth {
            body["session"] = serde_json::to_value(claims).unwrap();
        }
        let mut req = client
            .post(format!("http://127.0.0.1:{}/tool", server.http_port()))
            .json(&body);
        if let Some((token, _)) = auth {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        let resp = req.send().await.unwrap();
        let status = resp.status().as_u16();
        let json: Value = resp.json().await.unwrap_or(Value::Null);
        (status, json)
    }

    async fn post_renew(
        server: &RequirementMcpServer,
        request: &LoopbackCapabilityRenewalRequest,
    ) -> (
        u16,
        Option<nomifun_common::LoopbackCapabilityAccess<RequirementCapabilityClaims>>,
    ) {
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!(
                "http://127.0.0.1:{}{}",
                server.http_port(),
                LOOPBACK_CAPABILITY_RENEW_PATH
            ))
            .json(request)
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        let access = if status == StatusCode::OK.as_u16() {
            Some(response.json().await.unwrap())
        } else {
            None
        };
        (status, access)
    }

    async fn post_revoke(
        server: &RequirementMcpServer,
        request: &LoopbackCapabilityRenewalRequest,
    ) -> u16 {
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!(
                "http://127.0.0.1:{}{}",
                server.http_port(),
                LOOPBACK_CAPABILITY_REVOKE_PATH
            ))
            .json(request)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    #[tokio::test]
    async fn start_returns_positive_port_and_redacted_issuer() {
        let server = RequirementMcpServer::start().await.unwrap();
        assert!(server.http_port() > 0);
        let debug = format!("{:?}", server.issuer_config("/bin/nomicore".into()));
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("root_secret"));
    }

    #[tokio::test]
    async fn each_issued_child_uses_a_fresh_nonce_and_token() {
        let server = RequirementMcpServer::start().await.unwrap();
        let a = conversation_child(&server, 1);
        let b = conversation_child(&server, 1);
        assert_ne!(
            a.bootstrap.access.claims.nonce,
            b.bootstrap.access.claims.nonce
        );
        assert_ne!(a.bootstrap.access.token, b.bootstrap.access.token);
    }

    #[tokio::test]
    async fn renewal_restores_immutable_scope_and_revoke_closes_the_lease() {
        let server = RequirementMcpServer::start().await.unwrap();
        let child = conversation_child(&server, 17);

        let mut forged_proof = child.bootstrap.renewal.clone();
        forged_proof.renewal_proof.push('x');
        assert_eq!(post_renew(&server, &forged_proof).await.0, 401);
        assert_eq!(post_revoke(&server, &forged_proof).await, 401);

        let (status, renewed) = post_renew(&server, &child.bootstrap.renewal).await;
        assert_eq!(status, 200);
        let renewed = renewed.expect("valid proof should renew");
        let original = &child.bootstrap.access.claims;
        assert_eq!(renewed.claims.lease_id, original.lease_id);
        assert_eq!(renewed.claims.user_id, original.user_id);
        assert_eq!(renewed.claims.session, original.session);
        assert_eq!(renewed.claims.allowed_tools, original.allowed_tools);
        assert_eq!(renewed.claims.scope, original.scope);
        assert_ne!(renewed.claims.nonce, original.nonce);

        assert_eq!(post_revoke(&server, &child.bootstrap.renewal).await, 204);
        let (status, _) = post_tool(
            &server,
            Some((&renewed.token, &renewed.claims)),
            json!({"tool": "requirement_complete", "args": {"id": 1}}),
        )
        .await;
        assert_eq!(status, 401, "revoked access must fail before dispatch");
        assert_eq!(post_renew(&server, &child.bootstrap.renewal).await.0, 401);
    }

    #[tokio::test]
    async fn renewal_rejects_registry_authorization_with_invalid_requirement_scope() {
        let server = RequirementMcpServer::start().await.unwrap();
        let claims = RequirementCapabilityClaims::issue(
            "system_default_user",
            nomifun_common::LoopbackSessionBinding::conversation("17"),
            ["requirement_complete"],
            RequirementCapabilityScope {
                owner_kind: nomifun_common::LoopbackSessionKind::Terminal,
                owner_session_id: 17,
            },
        )
        .unwrap();
        let (_, renewal_proof) = server
            .issuer
            .activate(REQUIREMENT_CAPABILITY_DOMAIN, &claims)
            .unwrap();
        let request = LoopbackCapabilityRenewalRequest {
            lease_id: claims.lease_id,
            renewal_proof,
        };
        assert_eq!(post_renew(&server, &request).await.0, 401);
    }

    #[tokio::test]
    async fn tool_call_requires_auth() {
        let (svc, _id) = service_with_claimed_req().await;
        let server = started_server(&svc).await;
        let (status, _) = post_tool(&server, None, json!({"tool": "requirement_complete", "args": {"id": "x"}})).await;
        assert_eq!(status, 401);
    }

    #[tokio::test]
    async fn complete_marks_requirement_done() {
        let (svc, id) = service_with_claimed_req().await;
        let server = started_server(&svc).await;
        let child = conversation_child(&server, 1);
        let (status, body) = post_tool(
            &server,
            Some((
                &child.bootstrap.access.token,
                &child.bootstrap.access.claims,
            )),
            json!({
                "tool": "requirement_complete",
                "args": {"id": id, "completion_note": "did the thing"},
            }),
        )
        .await;
        assert_eq!(status, 200);
        assert!(body.get("result").is_some(), "expected result, got {body}");
        let after = svc.get(id).await.unwrap();
        assert_eq!(after.status, RequirementStatus::Done);
        assert_eq!(after.completion_note.as_deref(), Some("did the thing"));
    }

    #[tokio::test]
    async fn update_status_failed_marks_failed() {
        let (svc, id) = service_with_claimed_req().await;
        let server = started_server(&svc).await;
        let child = conversation_child(&server, 1);
        let (status, body) = post_tool(
            &server,
            Some((
                &child.bootstrap.access.token,
                &child.bootstrap.access.claims,
            )),
            json!({
                "tool": "requirement_update_status",
                "args": {"id": id, "status": "failed", "note": "could not finish"},
            }),
        )
        .await;
        assert_eq!(status, 200);
        assert!(body.get("result").is_some(), "expected result, got {body}");
        let after = svc.get(id).await.unwrap();
        assert_eq!(after.status, RequirementStatus::Failed);
    }

    #[tokio::test]
    async fn update_status_rejects_invalid_status() {
        let (svc, id) = service_with_claimed_req().await;
        let server = started_server(&svc).await;
        let child = conversation_child(&server, 1);
        let (_status, body) = post_tool(
            &server,
            Some((
                &child.bootstrap.access.token,
                &child.bootstrap.access.claims,
            )),
            json!({
                "tool": "requirement_update_status",
                "args": {"id": id, "status": "bogus"},
            }),
        )
        .await;
        assert!(
            body.get("error").and_then(Value::as_str).is_some_and(|e| e.contains("bogus")),
            "expected an invalid-status error, got {body}"
        );
        // The requirement must remain untouched (still in_progress).
        let after = svc.get(id).await.unwrap();
        assert_eq!(after.status, RequirementStatus::InProgress);
    }

    #[tokio::test]
    async fn tool_outside_signed_allowlist_is_forbidden() {
        let (svc, _id) = service_with_claimed_req().await;
        let server = started_server(&svc).await;
        let child = conversation_child(&server, 1);
        let (status, body) = post_tool(
            &server,
            Some((
                &child.bootstrap.access.token,
                &child.bootstrap.access.claims,
            )),
            json!({"tool": "requirement_explode", "args": {}}),
        )
        .await;
        assert_eq!(status, 403);
        assert_eq!(body["error"], "forbidden");
    }

    #[tokio::test]
    async fn missing_service_returns_unavailable() {
        // Server started but set_service never called → Weak upgrades to None.
        let server = RequirementMcpServer::start().await.unwrap();
        let child = conversation_child(&server, 1);
        let (status, body) = post_tool(
            &server,
            Some((
                &child.bootstrap.access.token,
                &child.bootstrap.access.claims,
            )),
            json!({"tool": "requirement_complete", "args": {"id": "x"}}),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body.get("error").and_then(Value::as_str), Some("service_unavailable"));
    }

    #[tokio::test]
    async fn complete_rejects_cross_session() {
        let (svc, id) = service_with_claimed_req().await;
        let server = started_server(&svc).await;
        // Requirement is owned by conv_1; a call from conv_other must be refused.
        let child = conversation_child(&server, 2);
        let (_status, body) = post_tool(
            &server,
            Some((
                &child.bootstrap.access.token,
                &child.bootstrap.access.claims,
            )),
            json!({
                "tool": "requirement_complete",
                "args": {"id": id, "completion_note": "sneaky"},
            }),
        )
        .await;
        assert!(
            body.get("error").and_then(Value::as_str).is_some_and(|e| e.contains("different session")),
            "expected a cross-session refusal, got {body}"
        );
        let after = svc.get(id).await.unwrap();
        assert_eq!(after.status, RequirementStatus::InProgress, "must not be mutated");
    }

    // ── C1 (spec §2.2): cross-domain authz isolation ────────────────────────
    //
    // The requirement MCP caller is ALWAYS a conversation (the MCP is injected
    // into ACP conversation sessions). After integerization `conv#5` and
    // `term#5` share the numeric owner value `5`. A conversation caller must
    // NEVER be allowed to mutate a requirement owned by a TERMINAL that merely
    // shares its number — `verify_scope` pairs the owner with `owner_kind`.

    /// Service with one requirement claimed by TERMINAL #5, plus a conversation
    /// #5 present (same number, different domain). Returns the service + req id.
    async fn service_with_terminal5_claimed_req() -> (Arc<RequirementService>, i64) {
        let db = init_database_memory().await.unwrap();
        let repo: Arc<dyn nomifun_db::IRequirementRepository> =
            Arc::new(SqliteRequirementRepository::new(db.pool().clone()));
        let emitter = RequirementEventEmitter::new(
            Arc::new(NoopBroadcaster),
            Arc::from("system_default_user"),
        );
        sqlx::query(
            "INSERT INTO conversations (id, user_id, name, type, created_at, updated_at) \
             VALUES (5, 'system_default_user', 'Conv Five', 'acp', 0, 0)",
        )
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO terminal_sessions \
                 (id, user_id, name, cwd, command, args, cols, rows, last_status, created_at, updated_at) \
             VALUES (5, 'system_default_user', 'Term Five', '/tmp', 'bash', '[]', 80, 24, 'running', 0, 0)",
        )
        .execute(db.pool())
        .await
        .unwrap();
        Box::leak(Box::new(db));
        let svc = Arc::new(RequirementService::new(repo, emitter));
        let req = svc
            .create(CreateRequirementRequest {
                title: "Term work".into(),
                content: "body".into(),
                tag: "t".into(),
                order_key: None,
                status: None,
                created_by: None,
                attachments: vec![],
            })
            .await
            .unwrap();
        let claimed = svc
            .claim_next("t", 5, AutoWorkTargetKind::Terminal, 120_000)
            .await
            .unwrap()
            .expect("claimable");
        assert_eq!(claimed.owner_kind.as_deref(), Some("terminal"));
        (svc, req.id)
    }

    #[tokio::test]
    async fn c1_terminal_owned_req_unmutated_by_conversation_mcp_call() {
        // End-to-end through the HTTP tool: a conversation #5 caller's
        // requirement_complete on a terminal#5-owned requirement is refused and
        // the requirement is left in_progress.
        let (svc, id) = service_with_terminal5_claimed_req().await;
        let server = started_server(&svc).await;
        let child = conversation_child(&server, 5);
        let (_status, body) = post_tool(
            &server,
            Some((
                &child.bootstrap.access.token,
                &child.bootstrap.access.claims,
            )),
            json!({
                "tool": "requirement_complete",
                "args": {"id": id, "completion_note": "cross-domain"},
            }),
        )
        .await;
        assert!(
            body.get("error").and_then(Value::as_str).is_some_and(|e| e.contains("different session")),
            "expected a cross-domain refusal, got {body}"
        );
        let after = svc.get(id).await.unwrap();
        assert_eq!(after.status, RequirementStatus::InProgress, "terminal#5 work must not be mutated by conv#5");
        assert_eq!(after.owner_kind.as_deref(), Some("terminal"));
    }

    #[tokio::test]
    async fn terminal_child_can_complete_only_its_owned_requirement() {
        let (svc, id) = service_with_terminal5_claimed_req().await;
        let server = started_server(&svc).await;
        let child = terminal_child(&server, 5);
        let (_status, body) = post_tool(
            &server,
            Some((
                &child.bootstrap.access.token,
                &child.bootstrap.access.claims,
            )),
            json!({
                "tool": "requirement_complete",
                "args": {"id": id, "completion_note": "terminal did it"},
            }),
        )
        .await;
        assert!(
            body.get("result").is_some(),
            "terminal #5 should complete its own req, got {body}"
        );
        let after = svc.get(id).await.unwrap();
        assert_eq!(after.status, RequirementStatus::Done);
        assert_eq!(after.completion_note.as_deref(), Some("terminal did it"));
    }

    #[tokio::test]
    async fn different_terminal_caller_denied_via_http() {
        // End-to-end: terminal #99 cannot complete terminal#5-owned req.
        let (svc, id) = service_with_terminal5_claimed_req().await;
        let server = started_server(&svc).await;
        let child = terminal_child(&server, 99);
        let (_status, body) = post_tool(
            &server,
            Some((
                &child.bootstrap.access.token,
                &child.bootstrap.access.claims,
            )),
            json!({
                "tool": "requirement_complete",
                "args": {"id": id, "completion_note": "sneaky terminal"},
            }),
        )
        .await;
        assert!(
            body.get("error").and_then(Value::as_str).is_some_and(|e| e.contains("different session")),
            "terminal #99 must be denied on terminal#5-owned requirement, got {body}"
        );
        let after = svc.get(id).await.unwrap();
        assert_eq!(after.status, RequirementStatus::InProgress, "must not be mutated");
    }

    #[tokio::test]
    async fn tampered_cross_session_and_expired_claims_are_unauthorized() {
        let (svc, id) = service_with_claimed_req().await;
        let server = started_server(&svc).await;
        let child = conversation_child(&server, 1);

        let mut forged = child.bootstrap.access.claims.clone();
        forged.session = nomifun_common::LoopbackSessionBinding::conversation("2");
        forged.scope.owner_session_id = 2;
        let (status, _) = post_tool(
            &server,
            Some((&child.bootstrap.access.token, &forged)),
            json!({"tool": "requirement_complete", "args": {"id": id}}),
        )
        .await;
        assert_eq!(status, 401, "claim tampering must invalidate the token");

        let now = nomifun_common::unix_time_secs();
        let expired = server
            .issuer
            .renew_at::<RequirementCapabilityScope>(
                REQUIREMENT_CAPABILITY_DOMAIN,
                &child.bootstrap.renewal,
                now.saturating_sub(nomifun_common::LOOPBACK_CAPABILITY_TTL_SECS + 1),
            )
            .expect("clock-injected renewal should produce an already-expired access");
        let (status, _) = post_tool(
            &server,
            Some((&expired.token, &expired.claims)),
            json!({"tool": "requirement_complete", "args": {"id": id}}),
        )
        .await;
        assert_eq!(status, 401, "even correctly signed expired claims fail closed");
    }
}
