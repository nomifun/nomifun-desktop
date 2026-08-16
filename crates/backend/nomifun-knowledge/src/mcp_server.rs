//! In-process HTTP MCP server exposing the scoped knowledge tools to in-app
//! terminal CLI sessions (claude / codex / gemini and friends).
//!
//! ## Why this exists
//!
//! Terminal CLIs have no in-process tool bus we can register the native
//! `KnowledgeSearchTool` into (only the nomi engine does). To give those CLIs the
//! same knowledge-retrieval surface the nomi engine has natively, this server
//! exposes a scoped tool set (`knowledge_search` / `knowledge_read` /
//! `knowledge_write`) over authenticated HTTP. The backend resolves workspace +
//! mounted base ids before spawn and signs them into the child's capability. The
//! model supplies only tool arguments; `cwd`, `kb_ids`, user, and session never
//! come from an unsigned request body.
//!
//! ## Shape (mirrors `nomifun-requirement::mcp_server::RequirementMcpServer`)
//!
//! This is the in-process HTTP half. Terminal CLIs spawn a SEPARATE stdio process
//! (`nomicore mcp-knowledge-stdio`) that cannot share this process's
//! `KnowledgeService`; it forwards each tool call back here as an authenticated
//! `POST /tool`. The transport is stdio because claude / codex / gemini advertise
//! stdio-only MCP capabilities (they drop HTTP/SSE servers), so a direct-HTTP
//! injection would never reach them.
//!
//! ## Security
//!
//! The process-local issuer stays in this server. A child receives a renewable
//! lease bootstrap: short-lived access binds user, session, tools, workspace,
//! and mounted base ids, while the renewal proof carries no mutable scope.

use std::net::SocketAddr;
use std::sync::{Arc, Weak};

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use nomifun_api_types::{
    KNOWLEDGE_CAPABILITY_DOMAIN, KnowledgeCapabilityClaims, KnowledgeCapabilityScope,
    KnowledgeMcpConfig,
};
use nomifun_common::{
    LOOPBACK_CAPABILITY_RENEW_PATH, LOOPBACK_CAPABILITY_REVOKE_PATH,
    KnowledgeBaseId, LoopbackCapabilityIssuer, LoopbackCapabilityRenewalRequest,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::service::{
    KnowledgeBinding, KnowledgeSearchHit, KnowledgeService, WriteOp, WriteRequest, WriteSurface, WriteTargetSpec,
    decode_doc_handle, encode_doc_handle, resolve_write_policy,
};
use crate::broker::KnowledgeBroker;

/// Late-bound handle to the singleton `KnowledgeService`. Held as a `Weak` so
/// the server never keeps the service alive on its own (matches the requirement
/// server's slot pattern). Wired via [`KnowledgeMcpServer::set_service`].
type ServiceSlot = Arc<RwLock<Weak<KnowledgeService>>>;

#[derive(Clone)]
struct KbMcpState {
    issuer: Arc<LoopbackCapabilityIssuer>,
    service: ServiceSlot,
}

/// In-process HTTP MCP server for the scoped knowledge tools.
pub struct KnowledgeMcpServer {
    http_addr: SocketAddr,
    issuer: Arc<LoopbackCapabilityIssuer>,
    shutdown_handle: Option<tokio::task::JoinHandle<()>>,
    service_slot: ServiceSlot,
    external_broker: Option<KnowledgeBroker>,
}

impl KnowledgeMcpServer {
    /// Bind a fresh `127.0.0.1:0` listener, create a process-local issuer, and
    /// start serving capability lifecycle routes plus `POST /tool`. The service
    /// must be wired separately via [`set_service`](Self::set_service) before
    /// the first tool call arrives.
    pub async fn start() -> Result<Self, String> {
        let issuer = Arc::new(LoopbackCapabilityIssuer::random()?);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Failed to bind knowledge MCP HTTP listener: {e}"))?;
        let http_addr = listener
            .local_addr()
            .map_err(|e| format!("Failed to read knowledge MCP local addr: {e}"))?;

        let service_slot: ServiceSlot = Arc::new(RwLock::new(Weak::new()));

        let state = KbMcpState {
            issuer: issuer.clone(),
            service: service_slot.clone(),
        };

        let app = axum::Router::new()
            .route("/tool", axum::routing::post(handle_tool_request))
            .route("/context", axum::routing::post(handle_context_request))
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
                warn!(error = %e, "Knowledge MCP axum server exited with error");
            }
        });

        debug!(http_port = http_addr.port(), "Knowledge MCP Server started (axum)");

        Ok(Self {
            http_addr,
            issuer,
            shutdown_handle: Some(handle),
            service_slot,
            external_broker: None,
        })
    }

    /// Wire the singleton `KnowledgeService` after it is constructed. Must be
    /// called once before the first tool request arrives. Takes the `Arc` and
    /// downgrades internally so callers never construct the `Weak` themselves.
    pub async fn set_service(&self, svc: &Arc<KnowledgeService>) {
        // Async setter: the slot is a `tokio::sync::RwLock` (read with
        // `.read().await` in the handler), so we acquire it with `.write().await`.
        // `blocking_write` would PANIC here — `set_service` is called from the
        // async service bootstrap (`AppServices::from_config`), and blocking a
        // tokio runtime thread is forbidden. Runs once at wiring time, before any
        // request can contend the slot.
        *self.service_slot.write().await = Arc::downgrade(svc);
    }

    pub fn http_port(&self) -> u16 {
        self.http_addr.port()
    }

    /// Build the non-serializable issuer configuration used by the main
    /// process to mint one capability per Agent/Terminal child.
    pub fn issuer_config(&self, binary_path: String) -> KnowledgeMcpConfig {
        KnowledgeMcpConfig::from_issuer(
            self.http_addr.port(),
            self.issuer.clone(),
            binary_path,
        )
    }

    /// Start the owner-authenticated local broker used by persistent external
    /// Claude/Gemini/Codex registrations. The broker reuses this server's
    /// private issuer and service slot but never exposes either through config.
    pub async fn start_external_broker(
        &mut self,
        config: KnowledgeMcpConfig,
        installation_owner_id: String,
    ) -> Result<(), String> {
        let service = self.service_slot.read().await.clone();
        if service.upgrade().is_none() {
            return Err("knowledge service must be wired before broker start".into());
        }
        let broker = KnowledgeBroker::start(config, service, installation_owner_id).await?;
        self.external_broker = Some(broker);
        Ok(())
    }

    pub fn stop(&mut self) {
        // Revoke all external-process leases before stopping their HTTP renewal
        // endpoint. This makes restart invalidation immediate and deterministic.
        self.external_broker.take();
        if let Some(handle) = self.shutdown_handle.take() {
            handle.abort();
            debug!(http_port = self.http_addr.port(), "Knowledge MCP Server stop requested");
        }
    }
}

impl Drop for KnowledgeMcpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// Axum handler
// ---------------------------------------------------------------------------

async fn handle_tool_request(
    State(state): State<KbMcpState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let presented_token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");

    let claims: KnowledgeCapabilityClaims = match body
        .get("session")
        .cloned()
        .and_then(|value| serde_json::from_value::<KnowledgeCapabilityClaims>(value).ok())
    {
        Some(claims)
            if state
                .issuer
                .verify_access(
                    KNOWLEDGE_CAPABILITY_DOMAIN,
                    &claims,
                    presented_token,
                )
                .is_ok()
                && claims.scope.validate().is_ok() => claims,
        _ => {
            warn!("Knowledge MCP: rejected invalid, expired, or missing scoped capability");
            return (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"})))
                .into_response();
        }
    };

    let tool = body.get("tool").and_then(Value::as_str).unwrap_or("");
    if !claims.allows(tool) {
        warn!(tool, "Knowledge MCP: tool is outside signed capability scope");
        return (StatusCode::FORBIDDEN, Json(json!({"error": "forbidden"})))
            .into_response();
    }

    let Some(service) = state.service.read().await.upgrade() else {
        warn!("Knowledge MCP: service not available");
        return finish(json!({"error": "knowledge service unavailable"}));
    };

    let workspace_path = &claims.scope.workspace_path;
    let args = body.get("args").cloned().unwrap_or(Value::Null);

    // Terminal sessions resolve their kb scope LIVE from the workspace's
    // workpath binding at every dispatch: the signed kb_ids are only the
    // spawn-time snapshot, and a binding edited while the PTY runs (the
    // session header's KnowledgeControl) must take effect without a relaunch.
    // This stays inside the signed trust boundary — the capability still pins
    // user + terminal + workspace_path, and the binding row is server-side
    // state of that same workspace. Conversation and external-process
    // sessions keep their issuance-time scope (conversations are re-issued on
    // every binding change by the runtime recycler; external broker sessions
    // deliberately keep their connect-time scope).
    let is_terminal_session =
        claims.session.kind == nomifun_common::LoopbackSessionKind::Terminal;
    let (kb_ids, live_terminal_binding): (Vec<KnowledgeBaseId>, Option<KnowledgeBinding>) =
        if is_terminal_session {
            let (live_ids, binding, _) =
                service.resolve_terminal_scope_for_cwd(workspace_path).await;
            (live_ids, Some(binding))
        } else {
            (claims.scope.kb_ids.clone(), None)
        };

    match tool {
        "knowledge_search" => {
            let query = args.get("query").and_then(|q| q.as_str()).unwrap_or("").trim().to_string();
            let limit = args
                .get("limit")
                .and_then(|n| n.as_u64())
                .map(|n| n as usize)
                .unwrap_or(8)
                .clamp(1, 20);
            info!(tool, kb_ids = kb_ids.len(), workspace = %workspace_path, "Knowledge MCP: dispatching tool");
            if kb_ids.is_empty() {
                return finish(no_bases_bound_error());
            }
            finish(dispatch_search(&service, &kb_ids, &query, limit).await)
        }
        "knowledge_read" => {
            let handle = args.get("handle").and_then(Value::as_str).unwrap_or("").trim().to_string();
            info!(tool, kb_ids = kb_ids.len(), workspace = %workspace_path, "Knowledge MCP: dispatching tool");
            if kb_ids.is_empty() {
                return finish(no_bases_bound_error());
            }
            finish(dispatch_read(&service, &kb_ids, &handle).await)
        }
        "knowledge_write" => {
            let (bound_kb_ids, binding) = match live_terminal_binding {
                // Terminal: the live binding IS the write scope — no
                // intersection with the spawn-time snapshot, so bases bound
                // after launch become writable immediately (and unbinding
                // revokes immediately). resolve_write_policy fails closed
                // when the live binding has write-back off.
                Some(binding) => (kb_ids.clone(), binding),
                None => {
                    let (resolved_kb_ids, binding, _) = service
                        .resolve_write_context_for_cwd(workspace_path)
                        .await;
                    let signed = &claims.scope.kb_ids;
                    let bound: Vec<KnowledgeBaseId> = resolved_kb_ids
                        .into_iter()
                        .filter(|id| signed.contains(id))
                        .collect();
                    (bound, binding)
                }
            };
            info!(tool, kb_ids = bound_kb_ids.len(), workspace = %workspace_path, "Knowledge MCP: dispatching tool");
            if bound_kb_ids.is_empty() {
                return finish(no_bases_bound_error());
            }
            finish(dispatch_write(&service, &bound_kb_ids, &binding, &args).await)
        }
        _ => {
            warn!(tool, "Knowledge MCP: unknown tool");
            finish(json!({"error": format!("unknown tool: {tool}")}))
        }
    }
}

/// Honest tool result for a session whose workspace currently has no enabled
/// knowledge binding (live resolution). Guides the model instead of failing
/// with an opaque empty search.
fn no_bases_bound_error() -> Value {
    json!({
        "error": "no knowledge bases are currently bound to this workspace; \
                  ask the user to mount one via the session's knowledge control"
    })
}

/// Compact mount metadata for the stdio bridge's MCP `initialize`
/// instructions (RC-5: the on-disk README was never surfaced to the model, so
/// the bridge asks for the CURRENT scope at startup and renders it into the
/// one channel every MCP host shows the model). Same authentication as
/// `/tool`; the response carries only base names/descriptions — no paths, no
/// ids, no secrets. Terminal sessions resolve the binding LIVE (identical to
/// dispatch); conversation/external sessions describe their signed scope.
async fn handle_context_request(
    State(state): State<KbMcpState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let presented_token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let claims: KnowledgeCapabilityClaims = match body
        .get("session")
        .cloned()
        .and_then(|value| serde_json::from_value::<KnowledgeCapabilityClaims>(value).ok())
    {
        Some(claims)
            if state
                .issuer
                .verify_access(KNOWLEDGE_CAPABILITY_DOMAIN, &claims, presented_token)
                .is_ok()
                && claims.scope.validate().is_ok() => claims,
        _ => {
            warn!("Knowledge MCP: rejected invalid context request");
            return (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"})))
                .into_response();
        }
    };
    let Some(service) = state.service.read().await.upgrade() else {
        return finish(json!({"error": "knowledge service unavailable"}));
    };

    let is_terminal_session =
        claims.session.kind == nomifun_common::LoopbackSessionKind::Terminal;
    let (kb_ids, write_enabled) = if is_terminal_session {
        let (ids, binding, _) = service
            .resolve_terminal_scope_for_cwd(&claims.scope.workspace_path)
            .await;
        (ids, binding.writeback)
    } else {
        (
            claims.scope.kb_ids.clone(),
            claims.allows(nomifun_api_types::KNOWLEDGE_WRITE_TOOL),
        )
    };

    let mut mounted = Vec::with_capacity(kb_ids.len());
    for id in &kb_ids {
        match service.get_base_info(id.as_str()).await {
            Ok(info) => mounted.push(json!({
                "name": info.name,
                "description": info.description,
            })),
            // A bound base that no longer exists is skipped, mirroring the
            // mount engine — the instructions must not advertise it.
            Err(_) => continue,
        }
    }
    finish(json!({"result": {"mounted": mounted, "write_enabled": write_enabled}}))
}

/// Renew access from the issuer's immutable authorization registry. The
/// request intentionally contains no user, session, tools, workspace, or base
/// ids that a child could widen.
async fn handle_capability_renew(
    State(state): State<KbMcpState>,
    Json(request): Json<LoopbackCapabilityRenewalRequest>,
) -> axum::response::Response {
    match state
        .issuer
        .renew::<KnowledgeCapabilityScope>(KNOWLEDGE_CAPABILITY_DOMAIN, &request)
    {
        Ok(access) if access.claims.scope.validate().is_ok() => Json(access).into_response(),
        _ => {
            warn!("Knowledge MCP: rejected invalid capability renewal");
            (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"})))
                .into_response()
        }
    }
}

/// Explicit child/runtime teardown. Invalid proofs fail closed; transport
/// failure is best-effort because the issuer registry is process-local.
async fn handle_capability_revoke(
    State(state): State<KbMcpState>,
    Json(request): Json<LoopbackCapabilityRenewalRequest>,
) -> axum::response::Response {
    match state
        .issuer
        .revoke(KNOWLEDGE_CAPABILITY_DOMAIN, &request)
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => {
            warn!("Knowledge MCP: rejected invalid capability revocation");
            (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"})))
                .into_response()
        }
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
// Tool implementation
// ---------------------------------------------------------------------------

/// Testable dispatch core: run `search_bases` and render the result envelope.
/// Returns `{"result": …}` on success / `{"error": …}` on failure, matching the
/// requirement server's envelope.
pub(crate) async fn dispatch_search<I: AsRef<str>>(
    service: &KnowledgeService,
    kb_ids: &[I],
    query: &str,
    limit: usize,
) -> serde_json::Value {
    let kb_ids = match kb_ids
        .iter()
        .map(|id| KnowledgeBaseId::parse(id.as_ref()))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(ids) => ids,
        Err(error) => return serde_json::json!({ "error": format!("invalid knowledge base id: {error}") }),
    };
    match service.search_bases(&kb_ids, query, limit).await {
        Ok(hits) => serde_json::json!({ "result": render_hits(query, &hits) }),
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    }
}

/// Read a full document by opaque `handle`, scoped to `kb_ids`. A handle whose
/// kb_id is outside the resolved scope is rejected — the model cannot widen it.
pub(crate) async fn dispatch_read<I: AsRef<str>>(service: &KnowledgeService, kb_ids: &[I], handle: &str) -> Value {
    let Some((kb_id, rel_path)) = decode_doc_handle(handle) else {
        return json!({ "error": format!("invalid document handle: {handle}") });
    };
    if !kb_ids.iter().any(|b| b.as_ref() == kb_id.as_str()) {
        return json!({ "error": "handle points to a base not in scope" });
    }
    match service.read_file(kb_id.as_str(), &rel_path).await {
        Ok(content) => json!({ "result": content.content }),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

/// Write a document through the canonical `write_document` path. The surface is
/// always `Terminal` (this server serves the in-app terminal CLIs); the placement
/// policy is resolved server-side from the caller's workpath binding — the model
/// supplies only `handle | base+rel_path` + `content`, never the policy.
pub(crate) async fn dispatch_write<I: AsRef<str>>(
    service: &KnowledgeService,
    bound_kb_ids: &[I],
    binding: &KnowledgeBinding,
    args: &Value,
) -> Value {
    let Some(content) = args.get("content").and_then(Value::as_str) else {
        return json!({ "error": "missing required field: content" });
    };
    if content.trim().is_empty() {
        return json!({ "error": "content is empty" });
    }
    let spec = if let Some(handle) = args.get("handle").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        WriteTargetSpec::Handle(handle.to_owned())
    } else {
        let Some(rel_path) = args.get("rel_path").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) else {
            return json!({ "error": "pass either `handle` (to update) or `rel_path` (to create a new document)" });
        };
        let kb_id = match resolve_base_id(service, bound_kb_ids, args.get("base").and_then(Value::as_str)).await {
            Ok(id) => id,
            Err(e) => return json!({ "error": e }),
        };
        WriteTargetSpec::Path { kb_id, rel_path: rel_path.to_owned() }
    };
    let policy = resolve_write_policy(WriteSurface::Terminal, binding);
    let bound_kb_ids = match bound_kb_ids
        .iter()
        .map(|id| KnowledgeBaseId::parse(id.as_ref()))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(ids) => ids,
        Err(error) => return json!({ "error": format!("invalid knowledge base id: {error}") }),
    };
    let req = WriteRequest {
        spec,
        content: content.to_owned(),
        policy,
        bound_kb_ids,
    };
    match service.write_document(req).await {
        Ok(out) => json!({ "result": {
            "kb_id": out.kb_id,
            "rel_path": out.final_rel_path,
            "updated": matches!(out.op, WriteOp::Update),
        }}),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

/// Resolve a model-supplied base NAME to a bound kb_id (create path). When
/// `requested` is omitted and exactly one base is in scope, that base is used.
async fn resolve_base_id<I: AsRef<str>>(
    service: &KnowledgeService,
    bound_kb_ids: &[I],
    requested: Option<&str>,
) -> Result<KnowledgeBaseId, String> {
    let bases: Vec<(KnowledgeBaseId, String)> = service
        .list_bases()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|b| bound_kb_ids.iter().any(|id| id.as_ref() == b.knowledge_base_id.as_str()))
        .map(|b| (b.knowledge_base_id, b.name))
        .collect();
    if bases.is_empty() {
        return Err("no knowledge bases are in scope to write to".to_owned());
    }
    match requested.map(str::trim).filter(|s| !s.is_empty()) {
        Some(name) => bases
            .iter()
            .find(|(_, n)| n.trim().eq_ignore_ascii_case(name))
            .map(|(id, _)| id.clone())
            .ok_or_else(|| {
                let names = bases.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>().join(", ");
                format!("unknown base \"{name}\"; in scope: {names}")
            }),
        None => {
            if bases.len() == 1 {
                Ok(bases[0].0.clone())
            } else {
                let names = bases.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>().join(", ");
                Err(format!("multiple bases in scope ({names}); specify `base`"))
            }
        }
    }
}

/// Render hits into the agent-facing plain-text block the tool returns.
fn render_hits(query: &str, hits: &[KnowledgeSearchHit]) -> String {
    if hits.is_empty() {
        return format!("No matches for \"{query}\" in the mounted knowledge bases. Try different terms.");
    }
    let mut out = format!("{} result(s) for \"{}\":\n", hits.len(), query);
    for (i, h) in hits.iter().enumerate() {
        out.push_str(&format!(
            "{}. [{}] {} — {}\n   {}\n   handle: {}\n",
            i + 1,
            h.kb_name,
            h.rel_path,
            if h.heading.is_empty() { "(no heading)" } else { &h.heading },
            h.snippet,
            encode_doc_handle(&h.kb_id, &h.rel_path),
        ));
    }
    out.push_str(
        "\nTo read a full document, call knowledge_read with its `handle`. To update one, call \
         knowledge_write with that same `handle` (do NOT rebuild the path).",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_OWNER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const TEST_CONVERSATION_ID: &str = "0190f5fe-7c00-7a00-8000-000000000017";
    const TEST_KB_ID_A: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const TEST_KB_ID_B: &str = "0190f5fe-7c00-7a00-8000-000000000002";
    use crate::events::KnowledgeEventEmitter;

    #[derive(Default)]
    struct NoopBroadcaster;
    impl nomifun_realtime::UserEventSink for NoopBroadcaster {
        fn send_to_user(
            &self,
            _user_id: &str,
            _event: nomifun_api_types::WebSocketMessage<serde_json::Value>,
        ) {
        }
    }

    fn hit(kb_name: &str, rel_path: &str, heading: &str, snippet: &str) -> KnowledgeSearchHit {
        KnowledgeSearchHit {
            kb_id: nomifun_common::KnowledgeBaseId::new(),
            kb_name: kb_name.into(),
            rel_path: rel_path.into(),
            heading: heading.into(),
            snippet: snippet.into(),
            score: 1,
        }
    }

    #[test]
    fn render_hits_empty_reports_no_matches() {
        let out = render_hits("回滚", &[]);
        assert!(out.contains("No matches"), "got: {out}");
        assert!(out.contains("回滚"), "echoes the query: {out}");
    }

    #[test]
    fn render_hits_non_empty_lists_path_heading_and_handle() {
        let hits = vec![hit("运维手册", "rollback.md", "回滚流程", "回滚分三步")];
        let out = render_hits("回滚", &hits);
        assert!(out.contains("rollback.md"), "path: {out}");
        assert!(out.contains("回滚流程"), "heading: {out}");
        assert!(out.contains("运维手册"), "kb name: {out}");
        assert!(out.contains("handle: kdoc_"), "handle: {out}");
        assert!(out.contains("knowledge_read") || out.contains("knowledge_write"), "tool hint: {out}");
    }

    #[test]
    fn render_hits_blank_heading_falls_back() {
        let hits = vec![hit("库", "a.md", "", "some snippet")];
        let out = render_hits("topic", &hits);
        assert!(out.contains("(no heading)"), "got: {out}");
    }

    /// Build a real `KnowledgeService` over an in-memory DB + temp data dir
    /// (recipe from nomifun-ai-agent's `knowledge_search_e2e`). Returns the
    /// service and the `TempDir` (keep it alive for the test's duration).
    async fn build_service() -> (Arc<KnowledgeService>, tempfile::TempDir) {
        let db = nomifun_db::init_database_memory().await.expect("in-memory db");
        let installation_owner = nomifun_db::installation_owner_id(db.pool())
            .await
            .expect("installation owner");
        let repo = Arc::new(nomifun_db::SqliteKnowledgeRepository::new(db.pool().clone()));
        let tmp = tempfile::tempdir().unwrap();
        let emitter = KnowledgeEventEmitter::new(
            Arc::new(NoopBroadcaster),
            Arc::from(installation_owner),
        );
        let svc = Arc::new(KnowledgeService::new(repo, tmp.path(), emitter));
        (svc, tmp)
    }

    #[tokio::test]
    async fn dispatch_search_finds_doc_and_wraps_result() {
        let (svc, _tmp) = build_service().await;
        let info = svc.create_base("运维手册", "", None, None).await.unwrap();
        let root = svc.data_dir().join("knowledge").join(info.knowledge_base_id.as_str());
        // The self-ignore the mount writes — must NOT blind the search.
        std::fs::write(root.join(".gitignore"), "*\n").unwrap();
        std::fs::write(root.join("rollback.md"), "# 回滚流程\n回滚分三步\n").unwrap();

        let out = dispatch_search(&svc, &[info.knowledge_base_id], "回滚", 8).await;
        let result = out
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("expected a result envelope, got {out}"));
        assert!(result.contains("rollback.md"), "must surface the doc:\n{result}");
        assert!(result.contains("回滚流程"), "must include heading:\n{result}");
    }

    #[tokio::test]
    async fn dispatch_search_no_match_reports_cleanly() {
        let (svc, _tmp) = build_service().await;
        let info = svc.create_base("库", "", None, None).await.unwrap();
        let root = svc.data_dir().join("knowledge").join(info.knowledge_base_id.as_str());
        std::fs::write(root.join("a.md"), "# A\nunrelated content\n").unwrap();

        let out = dispatch_search(&svc, &[info.knowledge_base_id], "完全不存在的主题词", 8).await;
        let result = out.get("result").and_then(Value::as_str).unwrap_or_else(|| panic!("got {out}"));
        assert!(result.contains("No matches"), "got: {result}");
    }

    // ── Signed scope HTTP boundary ───────────────────────────────────────

    /// Helper: start a `KnowledgeMcpServer`, wire a service, and return
    /// (server, service, temp dir) for HTTP-level tests.
    async fn start_wired_server() -> (KnowledgeMcpServer, Arc<KnowledgeService>, tempfile::TempDir) {
        let (svc, tmp) = build_service().await;
        let server = KnowledgeMcpServer::start().await.expect("bind");
        server.set_service(&svc).await;
        (server, svc, tmp)
    }

    fn conversation_child<I: AsRef<str>>(
        server: &KnowledgeMcpServer,
        conversation_id: &str,
        workspace: &str,
        kb_ids: &[I],
        allow_write: bool,
    ) -> nomifun_api_types::KnowledgeMcpChildConfig {
        let kb_ids = kb_ids
            .iter()
            .map(|id| KnowledgeBaseId::parse(id.as_ref()).expect("canonical knowledge-base test ID"))
            .collect::<Vec<_>>();
        server
            .issuer_config("/bin/nomicore".into())
            .issue_for_conversation(
                TEST_OWNER_ID,
                conversation_id,
                workspace,
                &kb_ids,
                allow_write,
            )
            .unwrap()
    }

    /// POST /tool with signed claims, returning status + JSON.
    async fn post_tool(
        server: &KnowledgeMcpServer,
        token: &str,
        claims: &KnowledgeCapabilityClaims,
        mut body: Value,
    ) -> (u16, Value) {
        body["session"] = serde_json::to_value(claims).unwrap();
        let client = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("test http client");
        let resp = client
            .post(format!("http://127.0.0.1:{}/tool", server.http_port()))
            .header("Authorization", format!("Bearer {token}"))
            .json(&body)
            .send()
            .await
            .expect("request");
        let status = resp.status().as_u16();
        (status, resp.json::<Value>().await.expect("json"))
    }

    async fn post_renew(
        server: &KnowledgeMcpServer,
        request: &LoopbackCapabilityRenewalRequest,
    ) -> (
        u16,
        Option<nomifun_common::LoopbackCapabilityAccess<KnowledgeCapabilityClaims>>,
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
        server: &KnowledgeMcpServer,
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
    async fn renewal_restores_immutable_scope_and_revoke_closes_the_lease() {
        let server = KnowledgeMcpServer::start().await.unwrap();
        let child = conversation_child(
            &server,
            TEST_CONVERSATION_ID,
            "/workspace",
            &[TEST_KB_ID_A, TEST_KB_ID_B],
            true,
        );

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
            &renewed.token,
            &renewed.claims,
            json!({"tool": "knowledge_search", "args": {"query": "x"}}),
        )
        .await;
        assert_eq!(status, 401, "revoked access must fail before dispatch");
        assert_eq!(post_renew(&server, &child.bootstrap.renewal).await.0, 401);
    }

    #[tokio::test]
    async fn renewal_rejects_registry_authorization_with_invalid_knowledge_scope() {
        let server = KnowledgeMcpServer::start().await.unwrap();
        let claims = KnowledgeCapabilityClaims::issue(
            TEST_OWNER_ID,
            nomifun_common::LoopbackSessionBinding::conversation(
                "0190f5fe-7c00-7a00-8000-000000000017",
            ),
            ["knowledge_search"],
            KnowledgeCapabilityScope {
                workspace_path: " /not-canonical".to_owned(),
                kb_ids: vec![KnowledgeBaseId::parse(
                    "0190f5fe-7c00-7a00-8000-000000000001",
                )
                .unwrap()],
            },
        )
        .unwrap();
        let (_, renewal_proof) = server
            .issuer
            .activate(KNOWLEDGE_CAPABILITY_DOMAIN, &claims)
            .unwrap();
        let request = LoopbackCapabilityRenewalRequest {
            lease_id: claims.lease_id,
            renewal_proof,
        };
        assert_eq!(post_renew(&server, &request).await.0, 401);
    }

    #[tokio::test]
    async fn signed_scope_selects_bases_and_ignores_forged_body_scope() {
        let (server, svc, _tmp) = start_wired_server().await;

        let info = svc.create_base("项目库", "", None, None).await.unwrap();
        let root = svc.data_dir().join("knowledge").join(info.knowledge_base_id.as_str());
        std::fs::write(root.join("api.md"), "# API\n接口文档内容\n").unwrap();
        let other = svc.create_base("无关库", "", None, None).await.unwrap();
        let other_root = svc.data_dir().join("knowledge").join(other.knowledge_base_id.as_str());
        std::fs::write(other_root.join("secret.md"), "# Secret\n接口隐藏内容\n").unwrap();

        let child = conversation_child(
            &server,
            TEST_CONVERSATION_ID,
            "/Users/test/myproject",
            std::slice::from_ref(&info.knowledge_base_id),
            false,
        );
        let (status, resp) = post_tool(&server, &child.bootstrap.access.token, &child.bootstrap.access.claims, json!({
            "tool": "knowledge_search",
            "cwd": "/forged",
            "kb_ids": [other.knowledge_base_id],
            "args": { "query": "接口" }
        }))
        .await;
        assert_eq!(status, 200);
        let result = resp.get("result").and_then(Value::as_str)
            .unwrap_or_else(|| panic!("expected result, got {resp}"));
        assert!(result.contains("api.md"), "signed scope should find the doc: {result}");
        assert!(!result.contains("secret.md"), "unsigned body scope must be ignored: {result}");
    }

    #[tokio::test]
    async fn tampered_cross_session_expired_and_write_escalation_fail_closed() {
        let (server, svc, _tmp) = start_wired_server().await;
        let info = svc.create_base("库", "", None, None).await.unwrap();
        let child = conversation_child(
            &server,
            TEST_CONVERSATION_ID,
            "/workspace",
            std::slice::from_ref(&info.knowledge_base_id),
            false,
        );

        let mut forged = child.bootstrap.access.claims.clone();
        forged.session = nomifun_common::LoopbackSessionBinding::conversation(
            "0190f5fe-7c00-7a00-8000-000000000099",
        );
        let (status, _) = post_tool(
            &server,
            &child.bootstrap.access.token,
            &forged,
            json!({"tool": "knowledge_search", "args": {"query": "x"}}),
        )
        .await;
        assert_eq!(status, 401);

        let (status, _) = post_tool(
            &server,
            &child.bootstrap.access.token,
            &child.bootstrap.access.claims,
            json!({"tool": "knowledge_write", "args": {"content": "x"}}),
        )
        .await;
        assert_eq!(status, 403, "read-only child cannot self-enable writes");

        let now = nomifun_common::unix_time_secs();
        let expired = server
            .issuer
            .renew_at::<KnowledgeCapabilityScope>(
                KNOWLEDGE_CAPABILITY_DOMAIN,
                &child.bootstrap.renewal,
                now.saturating_sub(nomifun_common::LOOPBACK_CAPABILITY_TTL_SECS + 1),
            )
            .expect("clock-injected renewal should produce an already-expired access");
        let (status, _) = post_tool(
            &server,
            &expired.token,
            &expired.claims,
            json!({"tool": "knowledge_search", "args": {"query": "x"}}),
        )
        .await;
        assert_eq!(status, 401);
    }

    // ── knowledge_read / knowledge_write (P2) ───────────────────────────

    #[tokio::test]
    async fn dispatch_read_returns_content_within_scope_and_denies_outside() {
        let (svc, _tmp) = build_service().await;
        let info = svc.create_base("库", "", None, None).await.unwrap();
        svc.write_file(&info.knowledge_base_id, "terms.md", "# T\nBODY-市盈率").await.unwrap();
        let h = encode_doc_handle(&info.knowledge_base_id, "terms.md");

        let ok = dispatch_read(&svc, std::slice::from_ref(&info.knowledge_base_id), &h).await;
        assert!(ok.get("result").and_then(Value::as_str).unwrap_or("").contains("BODY-市盈率"), "{ok}");
        // Out of scope (empty kb_ids) → denied.
        let denied = dispatch_read(&svc, &[] as &[String], &h).await;
        assert!(denied.get("error").is_some(), "out-of-scope handle must be denied: {denied}");
        // Malformed handle → error.
        let bad = dispatch_read(&svc, std::slice::from_ref(&info.knowledge_base_id), "not-a-handle").await;
        assert!(bad.get("error").is_some(), "{bad}");
    }

    #[tokio::test]
    async fn dispatch_write_lands_in_the_target_document_and_keeps_curated_text() {
        let (svc, _tmp) = build_service().await;
        let info = svc.create_base("库", "", None, None).await.unwrap();
        svc.write_file(&info.knowledge_base_id, "terms.md", "ORIGINAL").await.unwrap();
        let binding = KnowledgeBinding {
            enabled: true,
            writeback: true,
            kb_ids: vec![info.knowledge_base_id.clone()],
            ..Default::default()
        };
        let out = dispatch_write(
            &svc,
            std::slice::from_ref(&info.knowledge_base_id),
            &binding,
            &json!({ "handle": encode_doc_handle(&info.knowledge_base_id, "terms.md"), "content": "PROPOSED" }),
        )
        .await;
        let r = out.get("result").unwrap_or_else(|| panic!("{out}"));
        assert_eq!(r.get("rel_path").and_then(Value::as_str), Some("terms.md"));
        assert_eq!(r.get("updated").and_then(Value::as_bool), Some(true));
        // The model never sees the whole document, so the write appends instead
        // of overwriting — hand-curated material must survive the turn.
        assert_eq!(
            svc.read_file(&info.knowledge_base_id, "terms.md").await.unwrap().content,
            "ORIGINAL\n\nPROPOSED\n"
        );
    }

    #[tokio::test]
    async fn dispatch_write_refused_when_writeback_disabled() {
        let (svc, _tmp) = build_service().await;
        let info = svc.create_base("库", "", None, None).await.unwrap();
        svc.write_file(&info.knowledge_base_id, "terms.md", "x").await.unwrap();
        // Binding present but writeback off → policy Disabled.
        let binding = KnowledgeBinding { enabled: true, writeback: false, kb_ids: vec![info.knowledge_base_id.clone()], ..Default::default() };
        let out = dispatch_write(
            &svc,
            std::slice::from_ref(&info.knowledge_base_id),
            &binding,
            &json!({ "handle": encode_doc_handle(&info.knowledge_base_id, "terms.md"), "content": "y" }),
        )
        .await;
        assert!(out.get("error").is_some(), "writeback off must refuse: {out}");
    }

    #[tokio::test]
    async fn http_knowledge_write_routes_through_policy_direct() {
        let (server, svc, _tmp) = start_wired_server().await;
        let info = svc.create_base("项目库", "", None, None).await.unwrap();
        svc.write_file(&info.knowledge_base_id, "notes.md", "OLD").await.unwrap();
        let ws = "/Users/test/wp-write";
        let key = crate::workpath::workpath_key(ws);
        svc.set_binding(
            crate::workpath::WORKPATH_BINDING_KIND,
            &key,
            KnowledgeBinding {
                enabled: true,
                writeback: true,
                kb_ids: vec![info.knowledge_base_id.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let child = conversation_child(
            &server,
            TEST_CONVERSATION_ID,
            ws,
            std::slice::from_ref(&info.knowledge_base_id),
            true,
        );
        let (status, resp) = post_tool(&server, &child.bootstrap.access.token, &child.bootstrap.access.claims, json!({
            "tool": "knowledge_write",
            "args": { "handle": encode_doc_handle(&info.knowledge_base_id, "notes.md"), "content": "NEW" }
        }))
        .await;
        assert_eq!(status, 200);
        assert!(resp.get("result").is_some(), "expected result, got {resp}");
        // The HTTP tool route reaches the same append-only write path as every
        // other surface: the curated text stays, the new material joins it.
        let after = svc.read_file(&info.knowledge_base_id, "notes.md").await.unwrap().content;
        assert!(after.contains("OLD"), "the existing document must survive: {after}");
        assert!(after.contains("NEW"), "the new material must be recorded: {after}");
    }

    // ── Terminal sessions: live binding resolution (no relaunch needed) ──

    const TEST_TERMINAL_ID: &str = "0190f5fe-7c00-7a00-8000-0000000000a1";

    fn terminal_child<I: AsRef<str>>(
        server: &KnowledgeMcpServer,
        workspace: &str,
        kb_ids: &[I],
    ) -> nomifun_api_types::KnowledgeMcpChildConfig {
        let kb_ids = kb_ids
            .iter()
            .map(|id| KnowledgeBaseId::parse(id.as_ref()).expect("canonical knowledge-base test ID"))
            .collect::<Vec<_>>();
        server
            .issuer_config("/bin/nomicore".into())
            .issue_for_terminal(TEST_OWNER_ID, TEST_TERMINAL_ID, workspace, &kb_ids)
            .unwrap()
    }

    /// The incident shape: a terminal spawned BEFORE any base was bound (its
    /// signed kb snapshot is empty), then the user mounts a base via the
    /// session header. Search must pick up the live binding on the SAME
    /// capability — no relaunch.
    #[tokio::test]
    async fn terminal_search_resolves_live_binding_after_mount() {
        let (server, svc, _tmp) = start_wired_server().await;
        let info = svc.create_base("python基础", "", None, None).await.unwrap();
        let root = svc.data_dir().join("knowledge").join(info.knowledge_base_id.as_str());
        std::fs::write(root.join("list.md"), "# 列表\npython 列表推导式\n").unwrap();
        let ws = "/Users/test/wp-terminal-live";
        let child = terminal_child(&server, ws, &[] as &[String]);

        // Before any binding exists: honest "nothing bound" error.
        let (status, resp) = post_tool(&server, &child.bootstrap.access.token, &child.bootstrap.access.claims, json!({
            "tool": "knowledge_search", "args": { "query": "列表推导式" }
        }))
        .await;
        assert_eq!(status, 200);
        assert!(
            resp.get("error").and_then(Value::as_str).unwrap_or("").contains("no knowledge bases"),
            "unbound workspace must say so: {resp}"
        );

        // Bind while the "PTY" (capability) is live — the 13:27 POST.
        svc.set_binding(
            crate::workpath::WORKPATH_BINDING_KIND,
            &crate::workpath::workpath_key(ws),
            KnowledgeBinding {
                enabled: true,
                kb_ids: vec![info.knowledge_base_id.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // Same capability, no relaunch: search now hits the mounted base.
        let (status, resp) = post_tool(&server, &child.bootstrap.access.token, &child.bootstrap.access.claims, json!({
            "tool": "knowledge_search", "args": { "query": "列表推导式" }
        }))
        .await;
        assert_eq!(status, 200);
        let result = resp.get("result").and_then(Value::as_str).unwrap_or_else(|| panic!("{resp}"));
        assert!(result.contains("list.md"), "live binding must be searchable: {result}");
    }

    /// Write-back toggled ON while the terminal runs: the same capability
    /// (whose spawn-time binding had writeback off) must start accepting
    /// knowledge_write — and toggling OFF must refuse again immediately.
    #[tokio::test]
    async fn terminal_write_follows_live_writeback_policy_both_directions() {
        let (server, svc, _tmp) = start_wired_server().await;
        let info = svc.create_base("库", "", None, None).await.unwrap();
        svc.write_file(&info.knowledge_base_id, "terms.md", "ORIGINAL").await.unwrap();
        let ws = "/Users/test/wp-terminal-writeback";
        let key = crate::workpath::workpath_key(ws);
        // Spawn-time state: bound but read-only (the incident README said
        // "Write-back is DISABLED").
        svc.set_binding(
            crate::workpath::WORKPATH_BINDING_KIND,
            &key,
            KnowledgeBinding {
                enabled: true,
                writeback: false,
                kb_ids: vec![info.knowledge_base_id.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let child = terminal_child(&server, ws, std::slice::from_ref(&info.knowledge_base_id));
        let write_body = || json!({
            "tool": "knowledge_write",
            "args": { "handle": encode_doc_handle(&info.knowledge_base_id, "terms.md"), "content": "PROPOSED" }
        });

        // Terminal claims always carry knowledge_write (no 403); the live
        // policy refuses while write-back is off.
        let (status, resp) = post_tool(&server, &child.bootstrap.access.token, &child.bootstrap.access.claims, write_body()).await;
        assert_eq!(status, 200, "write tool must be in scope for terminals: {resp}");
        assert!(
            resp.get("error").and_then(Value::as_str).unwrap_or("").contains("write-back is disabled"),
            "live policy must refuse while writeback is off: {resp}"
        );

        // The 13:27 change: writeback on. Same capability now writes.
        svc.set_binding(
            crate::workpath::WORKPATH_BINDING_KIND,
            &key,
            KnowledgeBinding {
                enabled: true,
                writeback: true,
                kb_ids: vec![info.knowledge_base_id.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (status, resp) = post_tool(&server, &child.bootstrap.access.token, &child.bootstrap.access.claims, write_body()).await;
        assert_eq!(status, 200);
        let r = resp.get("result").unwrap_or_else(|| panic!("write must succeed live: {resp}"));
        assert_eq!(r.get("rel_path").and_then(Value::as_str), Some("terms.md"));
        assert_eq!(
            svc.read_file(&info.knowledge_base_id, "terms.md").await.unwrap().content,
            "ORIGINAL\n\nPROPOSED\n"
        );

        // Revoke: writeback back off — refusal is immediate too.
        svc.set_binding(
            crate::workpath::WORKPATH_BINDING_KIND,
            &key,
            KnowledgeBinding {
                enabled: true,
                writeback: false,
                kb_ids: vec![info.knowledge_base_id.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (status, resp) = post_tool(&server, &child.bootstrap.access.token, &child.bootstrap.access.claims, write_body()).await;
        assert_eq!(status, 200);
        assert!(resp.get("error").is_some(), "revocation must be immediate: {resp}");
    }

    async fn post_context(
        server: &KnowledgeMcpServer,
        token: &str,
        claims: &KnowledgeCapabilityClaims,
    ) -> (u16, Value) {
        let mut body = json!({});
        body["session"] = serde_json::to_value(claims).unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let resp = client
            .post(format!("http://127.0.0.1:{}/context", server.http_port()))
            .header("Authorization", format!("Bearer {token}"))
            .json(&body)
            .send()
            .await
            .expect("request");
        let status = resp.status().as_u16();
        (status, resp.json::<Value>().await.expect("json"))
    }

    /// `/context` (the bridge's initialize-instructions source) resolves a
    /// TERMINAL session's mounts LIVE: base names + write state track the
    /// binding row, an unbound workspace reports an empty set, and a missing
    /// or forged capability is rejected.
    #[tokio::test]
    async fn context_route_reports_live_terminal_mounts() {
        let (server, svc, _tmp) = start_wired_server().await;
        let info = svc.create_base("python基础", "Python 入门", None, None).await.unwrap();
        let ws = "/Users/test/wp-context";
        let child = terminal_child(&server, ws, &[] as &[String]);

        // Unbound → empty mounted set (bridge renders no instructions).
        let (status, resp) =
            post_context(&server, &child.bootstrap.access.token, &child.bootstrap.access.claims).await;
        assert_eq!(status, 200);
        assert_eq!(resp["result"]["mounted"].as_array().map(Vec::len), Some(0), "{resp}");

        // Bind + enable write-back → live names and write state, same capability.
        svc.set_binding(
            crate::workpath::WORKPATH_BINDING_KIND,
            &crate::workpath::workpath_key(ws),
            KnowledgeBinding {
                enabled: true,
                writeback: true,
                kb_ids: vec![info.knowledge_base_id.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (status, resp) =
            post_context(&server, &child.bootstrap.access.token, &child.bootstrap.access.claims).await;
        assert_eq!(status, 200);
        assert_eq!(resp["result"]["mounted"][0]["name"], "python基础", "{resp}");
        assert_eq!(resp["result"]["mounted"][0]["description"], "Python 入门", "{resp}");
        assert_eq!(resp["result"]["write_enabled"], true, "{resp}");
        assert!(
            resp["result"]["mounted"][0].get("root_path").is_none()
                && resp["result"]["mounted"][0].get("kb_id").is_none(),
            "context must not leak paths or ids: {resp}"
        );

        // Forged bearer token fails closed.
        let (status, _) =
            post_context(&server, "forged", &child.bootstrap.access.claims).await;
        assert_eq!(status, 401);
    }

    /// Conversation sessions keep issuance-time semantics: a read-only
    /// conversation capability still gets 403 for knowledge_write even after
    /// the workpath binding enables write-back (its runtime is recycled and
    /// re-issued on binding changes instead).
    #[tokio::test]
    async fn conversation_capability_write_gate_stays_frozen() {
        let (server, svc, _tmp) = start_wired_server().await;
        let info = svc.create_base("库", "", None, None).await.unwrap();
        svc.write_file(&info.knowledge_base_id, "terms.md", "x").await.unwrap();
        let ws = "/Users/test/wp-conv-frozen";
        let child = conversation_child(
            &server,
            TEST_CONVERSATION_ID,
            ws,
            std::slice::from_ref(&info.knowledge_base_id),
            false,
        );
        svc.set_binding(
            crate::workpath::WORKPATH_BINDING_KIND,
            &crate::workpath::workpath_key(ws),
            KnowledgeBinding {
                enabled: true,
                writeback: true,
                kb_ids: vec![info.knowledge_base_id.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (status, _) = post_tool(&server, &child.bootstrap.access.token, &child.bootstrap.access.claims, json!({
            "tool": "knowledge_write",
            "args": { "handle": encode_doc_handle(&info.knowledge_base_id, "terms.md"), "content": "y" }
        }))
        .await;
        assert_eq!(status, 403, "read-only conversation capability must stay frozen");
    }
}
