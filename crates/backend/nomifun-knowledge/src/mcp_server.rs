//! In-process HTTP MCP server exposing the single `knowledge_search` tool to
//! ACP agent sessions (claude / codex / gemini CLIs).
//!
//! ## Why this exists
//!
//! AutoWork drives ACP sessions, but ACP CLIs have no in-process tool bus we
//! can register the native `KnowledgeSearchTool` into (only the nomi engine
//! does). To give ACP agents the same knowledge-retrieval surface the nomi
//! engine has natively, this server exposes ONE scoped tool, `knowledge_search`,
//! over authenticated HTTP. Scope resolution has two paths:
//!
//! 1. **Explicit `kb_ids`** — baked at injection time and forwarded by the stdio
//!    bridge in each request body. The model searches only those bases.
//! 2. **Runtime `cwd` resolution** — when no explicit `kb_ids` are supplied, the
//!    server resolves scope from the caller's working directory: workpath-bound
//!    bases if an enabled binding exists, or all mounted bases as fallback.
//!
//! In both paths the security invariant holds: the model supplies only `query`;
//! scope is decided server-side; the model cannot widen the searchable set.
//!
//! ## Shape (mirrors `nomifun-requirement::mcp_server::RequirementMcpServer`)
//!
//! This is the in-process HTTP half. ACP CLIs spawn a SEPARATE stdio process
//! (`nomicore mcp-knowledge-stdio`) that cannot share this process's
//! `KnowledgeService`; it forwards each tool call back here as an authenticated
//! `POST /tool`. The transport is stdio because claude / codex / gemini
//! advertise stdio-only MCP capabilities (HTTP/SSE servers are dropped by the
//! ACP capability filter), so a direct-HTTP injection would never reach them.
//!
//! ## Security
//!
//! A random opaque bearer token gates every request (per-process, like the
//! requirement server). The tool is read-only, so there is no mutation scope to
//! verify beyond the bound base set carried in `kb_ids`.

use std::net::SocketAddr;
use std::sync::{Arc, Weak};

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use nomifun_common::generate_id;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::service::{
    KnowledgeBinding, KnowledgeSearchHit, KnowledgeService, WriteOp, WriteRequest, WriteSurface, WriteTargetSpec,
    decode_doc_handle, encode_doc_handle, resolve_write_policy,
};

/// Late-bound handle to the singleton `KnowledgeService`. Held as a `Weak` so
/// the server never keeps the service alive on its own (matches the requirement
/// server's slot pattern). Wired via [`KnowledgeMcpServer::set_service`].
type ServiceSlot = Arc<RwLock<Weak<KnowledgeService>>>;

#[derive(Clone)]
struct KbMcpState {
    auth_token: String,
    service: ServiceSlot,
}

/// In-process HTTP MCP server for the scoped `knowledge_search` tool.
pub struct KnowledgeMcpServer {
    http_addr: SocketAddr,
    auth_token: String,
    shutdown_handle: Option<tokio::task::JoinHandle<()>>,
    service_slot: ServiceSlot,
}

impl KnowledgeMcpServer {
    /// Bind a fresh `127.0.0.1:0` listener, mint a random bearer token, and
    /// start serving `POST /tool`. The service must be wired separately via
    /// [`set_service`](Self::set_service) before the first tool call arrives.
    pub async fn start() -> Result<Self, String> {
        let auth_token = generate_id();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Failed to bind knowledge MCP HTTP listener: {e}"))?;
        let http_addr = listener
            .local_addr()
            .map_err(|e| format!("Failed to read knowledge MCP local addr: {e}"))?;

        let service_slot: ServiceSlot = Arc::new(RwLock::new(Weak::new()));

        let state = KbMcpState {
            auth_token: auth_token.clone(),
            service: service_slot.clone(),
        };

        let app = axum::Router::new()
            .route("/tool", axum::routing::post(handle_tool_request))
            .with_state(state);

        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                warn!(error = %e, "Knowledge MCP axum server exited with error");
            }
        });

        debug!(http_port = http_addr.port(), "Knowledge MCP Server started (axum)");

        Ok(Self {
            http_addr,
            auth_token,
            shutdown_handle: Some(handle),
            service_slot,
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

    pub fn auth_token(&self) -> &str {
        &self.auth_token
    }

    pub fn stop(&mut self) {
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
    let provided_token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");

    if provided_token != state.auth_token {
        warn!("Knowledge MCP: unauthorized request");
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"}))).into_response();
    }

    let tool = body.get("tool").and_then(Value::as_str).unwrap_or("");

    let Some(service) = state.service.read().await.upgrade() else {
        warn!("Knowledge MCP: service not available");
        return finish(json!({"error": "knowledge service unavailable"}));
    };

    // Back-compat: an old bridge may still bake explicit kb_ids. Otherwise scope
    // is resolved server-side from cwd. Security invariant (all tools): the model
    // supplies only query/handle/content; scope + write policy are decided
    // server-side and cannot be widened by the model.
    let explicit_kb_ids: Vec<String> = body
        .get("kb_ids")
        .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
        .unwrap_or_default();
    let cwd = body.get("cwd").and_then(Value::as_str).unwrap_or("").to_string();
    let args = body.get("args").cloned().unwrap_or(Value::Null);

    match tool {
        "knowledge_search" => {
            let query = args.get("query").and_then(|q| q.as_str()).unwrap_or("").trim().to_string();
            let limit = args
                .get("limit")
                .and_then(|n| n.as_u64())
                .map(|n| n as usize)
                .unwrap_or(8)
                .clamp(1, 20);
            let kb_ids = if !explicit_kb_ids.is_empty() {
                explicit_kb_ids
            } else {
                service.resolve_kb_ids_for_cwd(&cwd).await
            };
            info!(tool, kb_ids = kb_ids.len(), cwd = %cwd, "Knowledge MCP: dispatching tool");
            finish(dispatch_search(&service, &kb_ids, &query, limit).await)
        }
        "knowledge_read" => {
            let handle = args.get("handle").and_then(Value::as_str).unwrap_or("").trim().to_string();
            let kb_ids = if !explicit_kb_ids.is_empty() {
                explicit_kb_ids
            } else {
                service.resolve_kb_ids_for_cwd(&cwd).await
            };
            info!(tool, kb_ids = kb_ids.len(), cwd = %cwd, "Knowledge MCP: dispatching tool");
            finish(dispatch_read(&service, &kb_ids, &handle).await)
        }
        "knowledge_write" => {
            let (bound_kb_ids, binding, wp_key) = service.resolve_write_context_for_cwd(&cwd).await;
            // Staged inbox scope: prefer an explicit conversation id (per-session
            // inbox, matching the nomi engine) when the bridge forwards one;
            // otherwise fall back to the workpath key (per-workspace inbox).
            let scope = body
                .get("conversation_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or(wp_key);
            info!(tool, kb_ids = bound_kb_ids.len(), cwd = %cwd, "Knowledge MCP: dispatching tool");
            finish(dispatch_write(&service, &bound_kb_ids, &binding, &scope, &args).await)
        }
        _ => {
            warn!(tool, "Knowledge MCP: unknown tool");
            finish(json!({"error": format!("unknown tool: {tool}")}))
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
pub(crate) async fn dispatch_search(
    service: &KnowledgeService,
    kb_ids: &[String],
    query: &str,
    limit: usize,
) -> serde_json::Value {
    match service.search_bases(kb_ids, query, limit).await {
        Ok(hits) => serde_json::json!({ "result": render_hits(query, &hits) }),
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    }
}

/// Read a full document by opaque `handle`, scoped to `kb_ids`. A handle whose
/// kb_id is outside the resolved scope is rejected — the model cannot widen it.
pub(crate) async fn dispatch_read(service: &KnowledgeService, kb_ids: &[String], handle: &str) -> Value {
    let Some((kb_id, rel_path)) = decode_doc_handle(handle) else {
        return json!({ "error": format!("invalid document handle: {handle}") });
    };
    if !kb_ids.iter().any(|b| b == &kb_id) {
        return json!({ "error": "handle points to a base not in scope" });
    }
    match service.read_file(&kb_id, &rel_path).await {
        Ok(content) => json!({ "result": content.content }),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

/// Write a document through the canonical `write_document` path. The surface is
/// always `TerminalAcp` (this server serves ACP/terminal CLIs); the placement
/// policy is resolved server-side from the caller's workpath binding — the model
/// supplies only `handle | base+rel_path` + `content`, never the policy/scope.
pub(crate) async fn dispatch_write(
    service: &KnowledgeService,
    bound_kb_ids: &[String],
    binding: &KnowledgeBinding,
    scope: &str,
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
    let policy = resolve_write_policy(WriteSurface::TerminalAcp, binding, scope);
    let req = WriteRequest { spec, content: content.to_owned(), policy, bound_kb_ids: bound_kb_ids.to_vec() };
    match service.write_document(req).await {
        Ok(out) => json!({ "result": {
            "kb_id": out.kb_id,
            "rel_path": out.final_rel_path,
            "staged": out.staged,
            "updated": matches!(out.op, WriteOp::Update),
        }}),
        Err(e) => json!({ "error": e.to_string() }),
    }
}

/// Resolve a model-supplied base NAME to a bound kb_id (create path). When
/// `requested` is omitted and exactly one base is in scope, that base is used.
async fn resolve_base_id(service: &KnowledgeService, bound_kb_ids: &[String], requested: Option<&str>) -> Result<String, String> {
    let bases: Vec<(String, String)> = service
        .list_bases()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|b| bound_kb_ids.contains(&b.id))
        .map(|b| (b.id, b.name))
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
    use crate::events::KnowledgeEventEmitter;

    #[derive(Default)]
    struct NoopBroadcaster;
    impl nomifun_realtime::EventBroadcaster for NoopBroadcaster {
        fn broadcast(&self, _event: nomifun_api_types::WebSocketMessage<serde_json::Value>) {}
    }

    fn hit(kb_name: &str, rel_path: &str, heading: &str, snippet: &str) -> KnowledgeSearchHit {
        KnowledgeSearchHit {
            kb_id: "kb_1".into(),
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
        let repo = Arc::new(nomifun_db::SqliteKnowledgeRepository::new(db.pool().clone()));
        let tmp = tempfile::tempdir().unwrap();
        let emitter = KnowledgeEventEmitter::new(Arc::new(NoopBroadcaster));
        let svc = Arc::new(KnowledgeService::new(repo, tmp.path(), emitter));
        (svc, tmp)
    }

    #[tokio::test]
    async fn dispatch_search_finds_doc_and_wraps_result() {
        let (svc, _tmp) = build_service().await;
        let info = svc.create_base("运维手册", "", None, None).await.unwrap();
        let root = svc.data_dir().join("knowledge").join(&info.id);
        // The self-ignore the mount writes — must NOT blind the search.
        std::fs::write(root.join(".gitignore"), "*\n").unwrap();
        std::fs::write(root.join("rollback.md"), "# 回滚流程\n回滚分三步\n").unwrap();

        let out = dispatch_search(&svc, &[info.id], "回滚", 8).await;
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
        let root = svc.data_dir().join("knowledge").join(&info.id);
        std::fs::write(root.join("a.md"), "# A\nunrelated content\n").unwrap();

        let out = dispatch_search(&svc, &[info.id], "完全不存在的主题词", 8).await;
        let result = out.get("result").and_then(Value::as_str).unwrap_or_else(|| panic!("got {out}"));
        assert!(result.contains("No matches"), "got: {result}");
    }

    // ── cwd-based scope resolution (Task 5) ─────────────────────────────

    /// Helper: start a `KnowledgeMcpServer`, wire a service, and return
    /// (server, service, port, token) for HTTP-level tests.
    async fn start_wired_server() -> (KnowledgeMcpServer, Arc<KnowledgeService>, u16, String, tempfile::TempDir) {
        let (svc, tmp) = build_service().await;
        let server = KnowledgeMcpServer::start().await.expect("bind");
        server.set_service(&svc).await;
        let port = server.http_port();
        let token = server.auth_token().to_owned();
        (server, svc, port, token, tmp)
    }

    /// POST /tool with a JSON body, return the response JSON.
    async fn post_tool(port: u16, token: &str, body: Value) -> Value {
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://127.0.0.1:{port}/tool"))
            .header("Authorization", format!("Bearer {token}"))
            .json(&body)
            .send()
            .await
            .expect("request");
        resp.json::<Value>().await.expect("json")
    }

    #[tokio::test]
    async fn tool_request_with_cwd_resolves_scope_via_service() {
        let (_server, svc, port, token, _tmp) = start_wired_server().await;

        // Create a base and bind it to a workpath.
        let info = svc.create_base("项目库", "", None, None).await.unwrap();
        let root = svc.data_dir().join("knowledge").join(&info.id);
        std::fs::write(root.join("api.md"), "# API\n接口文档内容\n").unwrap();

        let ws = "/Users/test/myproject";
        let key = crate::workpath::workpath_key(ws);
        svc.set_binding(
            crate::workpath::WORKPATH_BINDING_KIND,
            &key,
            crate::service::KnowledgeBinding {
                enabled: true,
                kb_ids: vec![info.id.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // Request with cwd (no kb_ids) → uses cwd-resolved scope.
        let resp = post_tool(port, &token, json!({
            "tool": "knowledge_search",
            "cwd": ws,
            "args": { "query": "接口" }
        }))
        .await;
        let result = resp.get("result").and_then(Value::as_str)
            .unwrap_or_else(|| panic!("expected result, got {resp}"));
        assert!(result.contains("api.md"), "cwd scope should find the doc: {result}");
    }

    #[tokio::test]
    async fn tool_request_with_explicit_kb_ids_uses_them_backcompat() {
        let (_server, svc, port, token, _tmp) = start_wired_server().await;

        let info = svc.create_base("手册", "", None, None).await.unwrap();
        let root = svc.data_dir().join("knowledge").join(&info.id);
        std::fs::write(root.join("ops.md"), "# Ops\n运维流程\n").unwrap();

        // Create another base (not bound to anything).
        let info2 = svc.create_base("无关库", "", None, None).await.unwrap();
        let root2 = svc.data_dir().join("knowledge").join(&info2.id);
        std::fs::write(root2.join("other.md"), "# Other\n别的东西\n").unwrap();

        // Request with explicit kb_ids (old bridge style) → uses those, ignores cwd.
        let resp = post_tool(port, &token, json!({
            "tool": "knowledge_search",
            "kb_ids": [info.id],
            "cwd": "/some/unbound/path",
            "args": { "query": "运维" }
        }))
        .await;
        let result = resp.get("result").and_then(Value::as_str)
            .unwrap_or_else(|| panic!("expected result, got {resp}"));
        assert!(result.contains("ops.md"), "explicit kb_ids should be used: {result}");
        // The other base should NOT be searched (explicit kb_ids narrows scope).
        assert!(!result.contains("other.md"), "should not search unspecified bases: {result}");
    }

    #[tokio::test]
    async fn tool_request_with_empty_cwd_searches_all_bases() {
        let (_server, svc, port, token, _tmp) = start_wired_server().await;

        let info = svc.create_base("全局库", "", None, None).await.unwrap();
        let root = svc.data_dir().join("knowledge").join(&info.id);
        std::fs::write(root.join("global.md"), "# Global\n全局知识\n").unwrap();

        // No kb_ids, empty cwd → fallback to all bases.
        let resp = post_tool(port, &token, json!({
            "tool": "knowledge_search",
            "cwd": "",
            "args": { "query": "全局" }
        }))
        .await;
        let result = resp.get("result").and_then(Value::as_str)
            .unwrap_or_else(|| panic!("expected result, got {resp}"));
        assert!(result.contains("global.md"), "empty cwd should search all: {result}");
    }

    // ── knowledge_read / knowledge_write (P2) ───────────────────────────

    #[tokio::test]
    async fn dispatch_read_returns_content_within_scope_and_denies_outside() {
        let (svc, _tmp) = build_service().await;
        let info = svc.create_base("库", "", None, None).await.unwrap();
        svc.write_file(&info.id, "terms.md", "# T\nBODY-市盈率").await.unwrap();
        let h = encode_doc_handle(&info.id, "terms.md");

        let ok = dispatch_read(&svc, std::slice::from_ref(&info.id), &h).await;
        assert!(ok.get("result").and_then(Value::as_str).unwrap_or("").contains("BODY-市盈率"), "{ok}");
        // Out of scope (empty kb_ids) → denied.
        let denied = dispatch_read(&svc, &[], &h).await;
        assert!(denied.get("error").is_some(), "out-of-scope handle must be denied: {denied}");
        // Malformed handle → error.
        let bad = dispatch_read(&svc, std::slice::from_ref(&info.id), "not-a-handle").await;
        assert!(bad.get("error").is_some(), "{bad}");
    }

    #[tokio::test]
    async fn dispatch_write_staged_lands_in_inbox_and_preserves_original() {
        let (svc, _tmp) = build_service().await;
        let info = svc.create_base("库", "", None, None).await.unwrap();
        svc.write_file(&info.id, "terms.md", "ORIGINAL").await.unwrap();
        let binding = KnowledgeBinding {
            enabled: true,
            writeback: true,
            writeback_mode: "staged".into(),
            kb_ids: vec![info.id.clone()],
            ..Default::default()
        };
        let out = dispatch_write(
            &svc,
            std::slice::from_ref(&info.id),
            &binding,
            "conv-x",
            &json!({ "handle": encode_doc_handle(&info.id, "terms.md"), "content": "PROPOSED" }),
        )
        .await;
        let r = out.get("result").unwrap_or_else(|| panic!("{out}"));
        assert_eq!(r.get("rel_path").and_then(Value::as_str), Some("_inbox/conv-x/terms.md"));
        assert_eq!(r.get("staged").and_then(Value::as_bool), Some(true));
        // Original untouched; proposal staged.
        assert_eq!(svc.read_file(&info.id, "terms.md").await.unwrap().content, "ORIGINAL");
        assert_eq!(svc.read_file(&info.id, "_inbox/conv-x/terms.md").await.unwrap().content, "PROPOSED");
    }

    #[tokio::test]
    async fn dispatch_write_refused_when_writeback_disabled() {
        let (svc, _tmp) = build_service().await;
        let info = svc.create_base("库", "", None, None).await.unwrap();
        svc.write_file(&info.id, "terms.md", "x").await.unwrap();
        // Binding present but writeback off → policy Disabled.
        let binding = KnowledgeBinding { enabled: true, writeback: false, kb_ids: vec![info.id.clone()], ..Default::default() };
        let out = dispatch_write(
            &svc,
            std::slice::from_ref(&info.id),
            &binding,
            "wp",
            &json!({ "handle": encode_doc_handle(&info.id, "terms.md"), "content": "y" }),
        )
        .await;
        assert!(out.get("error").is_some(), "writeback off must refuse: {out}");
    }

    #[tokio::test]
    async fn http_knowledge_write_routes_through_policy_direct() {
        let (_server, svc, port, token, _tmp) = start_wired_server().await;
        let info = svc.create_base("项目库", "", None, None).await.unwrap();
        svc.write_file(&info.id, "notes.md", "OLD").await.unwrap();
        let ws = "/Users/test/wp-write";
        let key = crate::workpath::workpath_key(ws);
        svc.set_binding(
            crate::workpath::WORKPATH_BINDING_KIND,
            &key,
            KnowledgeBinding {
                enabled: true,
                writeback: true,
                writeback_mode: "direct".into(),
                kb_ids: vec![info.id.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let resp = post_tool(port, &token, json!({
            "tool": "knowledge_write",
            "cwd": ws,
            "args": { "handle": encode_doc_handle(&info.id, "notes.md"), "content": "NEW" }
        }))
        .await;
        assert!(resp.get("result").is_some(), "expected result, got {resp}");
        assert_eq!(svc.read_file(&info.id, "notes.md").await.unwrap().content, "NEW");
    }
}
