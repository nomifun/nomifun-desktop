//! `nomicore mcp-knowledge-stdio` subcommand: MCP stdio server for the
//! per-session knowledge-search tool (`knowledge_search`).
//!
//! Spawned by ACP agent CLIs (claude / codex / gemini) when the knowledge MCP
//! is injected into a session that has knowledge bases mounted. Uses the `rmcp`
//! crate (Rust MCP SDK) for protocol handling so it is byte-compatible with each
//! CLI's MCP client.
//!
//! Tool calls are forwarded as authenticated HTTP POSTs to the in-process
//! `KnowledgeMcpServer` running in the main backend process at
//! `http://127.0.0.1:{port}/tool`. This stdio→HTTP hop exists because the
//! spawned process cannot share the main process's knowledge services, and
//! because claude / codex / gemini advertise stdio-only MCP capabilities (a
//! direct HTTP MCP server would be dropped by the ACP capability filter).
//!
//! Managed Agent/Terminal children receive a renewable capability in the
//! environment. Persistently registered external CLIs receive no credential in
//! config; they authenticate to the owner-only local broker, submit only their
//! cwd, and keep that control connection open until this bridge exits.

// Pre-existing layout convention (mirrors requirement_stdio / team_guide): the
// `forward_tool` impl block lives after the test module.
#![allow(clippy::items_after_test_module)]

use std::process::ExitCode;
use std::sync::Arc;

use nomifun_api_types::{
    KNOWLEDGE_CAPABILITY_DOMAIN, KnowledgeCapabilityScope,
    KnowledgeMcpConfig,
};
use nomifun_common::{LoopbackCapabilityClaims, LoopbackCapabilityError};
use nomifun_common::unix_time_secs;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{schemars, service::ServiceExt, tool, tool_router, transport};
use serde::Deserialize;

pub async fn run_knowledge_stdio() -> ExitCode {
    let managed_capability_present =
        select_managed_mode(std::env::var_os(KnowledgeMcpConfig::ENV_CAPABILITY));
    let mut broker_control = None;
    let client_result = if managed_capability_present {
        // Presence always selects managed mode, including an empty/malformed
        // value. Never fall through to broader broker scope on bad auth.
        super::stdio_common::ScopedBridgeClient::from_env(
            KnowledgeMcpConfig::ENV_CAPABILITY,
            KNOWLEDGE_CAPABILITY_DOMAIN,
            "mcp-knowledge-stdio",
            validate_knowledge_claims,
        )
        .await
    } else {
        let result = async {
            let cwd = std::env::current_dir()
                .map_err(|error| format!("could not determine current cwd: {error}"))?;
            let control = nomifun_knowledge::connect_external_knowledge_broker(&cwd).await?;
            let client = super::stdio_common::ScopedBridgeClient::from_bootstrap(
                control.bootstrap().clone(),
                KNOWLEDGE_CAPABILITY_DOMAIN,
                "mcp-knowledge-stdio",
                validate_knowledge_claims,
                Arc::new(unix_time_secs),
            )
            .await?;
            Ok::<_, String>((client, control))
        }
        .await;
        match result {
            Ok((client, control)) => {
                broker_control = Some(control);
                Ok(client)
            }
            Err(error) => Err(error),
        }
    };
    let client = match client_result {
        Ok(client) => client,
        Err(error) => {
            eprintln!("[mcp-knowledge-stdio] ERROR: {error}");
            return ExitCode::from(1);
        }
    };
    let claims = client.access().await.expect("startup renewal succeeded").claims;

    eprintln!(
        "[mcp-knowledge-stdio] Started OK. PORT={}, SESSION={}:{}, EXP={}",
        client.port(),
        claims.session.kind.as_str(),
        claims.session.session_id,
        claims.expires_at_unix_secs,
    );

    let lifecycle = client.clone();
    let server = KnowledgeStdioServer { client };

    let transport = transport::io::stdio();
    let exit = match server.serve(transport).await {
        Ok(peer) => {
            eprintln!("[mcp-knowledge-stdio] MCP session started, waiting for completion...");
            if let Err(e) = peer.waiting().await {
                eprintln!("[mcp-knowledge-stdio] Session ended with error: {e}");
            } else {
                eprintln!("[mcp-knowledge-stdio] Session ended normally");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[mcp-knowledge-stdio] Failed to start MCP server: {e}");
            ExitCode::from(1)
        }
    };
    lifecycle.revoke().await;
    drop(broker_control);
    exit
}

#[derive(Clone)]
struct KnowledgeStdioServer {
    client: super::stdio_common::ScopedBridgeClient<KnowledgeCapabilityScope>,
}

fn validate_knowledge_claims(
    claims: &LoopbackCapabilityClaims<KnowledgeCapabilityScope>,
) -> Result<(), LoopbackCapabilityError> {
    claims.validate_renewable_shape()?;
    claims.scope.validate()
}

#[cfg(test)]
mod startup_mode_tests {
    #[test]
    fn managed_environment_presence_always_wins() {
        assert!(super::select_managed_mode(Some(std::ffi::OsString::new())));
        assert!(super::select_managed_mode(Some(std::ffi::OsString::from("malformed"))));
        assert!(!super::select_managed_mode(None));
    }
}

fn select_managed_mode(value: Option<std::ffi::OsString>) -> bool {
    value.is_some()
}

#[derive(Deserialize, schemars::JsonSchema)]
struct SearchParams {
    /// The search query. Phrase it as the topic or question you need information
    /// about; the bases are ranked by relevance to this text.
    query: String,
    /// Optional maximum number of ranked results to return.
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct ReadParams {
    /// The opaque `handle` from a knowledge_search result.
    handle: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub(crate) struct WriteParams {
    /// To UPDATE an existing document: the opaque `handle` from a knowledge_search result.
    #[serde(default)]
    handle: Option<String>,
    /// For a NEW document: which knowledge base to write to (its name).
    #[serde(default)]
    base: Option<String>,
    /// For a NEW document: relative markdown path within the base (e.g. "terms.md"). Must end with .md.
    #[serde(default)]
    rel_path: Option<String>,
    /// The FULL markdown content to store.
    content: String,
}

#[tool_router]
impl KnowledgeStdioServer {
    #[tool(
        name = "knowledge_search",
        description = "Search the knowledge bases mounted into THIS session for relevant documents. Call this FIRST, before answering from memory, when the task touches any topic the bases may cover. Returns ranked results with an opaque `handle`; read a full result by calling knowledge_read with that exact handle. Copy the handle unchanged and do not rebuild it from the path."
    )]
    async fn knowledge_search(&self, Parameters(params): Parameters<SearchParams>) -> String {
        eprintln!("[mcp-knowledge-stdio] tools/call: knowledge_search");
        self.forward_tool(params.query, params.limit).await
    }

    #[tool(
        name = "knowledge_read",
        description = "Read the FULL markdown of a knowledge document by the `handle` returned by knowledge_search. Use this before updating a document so you can merge into its current content, then write it back with knowledge_write passing the same `handle`."
    )]
    async fn knowledge_read(&self, Parameters(params): Parameters<ReadParams>) -> String {
        eprintln!("[mcp-knowledge-stdio] tools/call: knowledge_read");
        self.forward_read(params.handle).await
    }

    #[tool(
        name = "knowledge_write",
        description = "Persist reusable knowledge INTO a mounted knowledge base. To UPDATE an existing document, pass its `handle` from a knowledge_search result; to CREATE a new one, pass `base` + a descriptive `.md` `rel_path`. Always include the full markdown `content`. Whether the write lands directly or is staged for review is decided by the workspace's write-back setting — you do not manage placement."
    )]
    async fn knowledge_write(&self, Parameters(params): Parameters<WriteParams>) -> String {
        eprintln!("[mcp-knowledge-stdio] tools/call: knowledge_write");
        self.forward_write(params).await
    }
}

#[rmcp::tool_handler(router = Self::tool_router())]
impl rmcp::ServerHandler for KnowledgeStdioServer {
    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, rmcp::ErrorData> {
        let claims = self
            .client
            .access()
            .await
            .map_err(capability_request_error)?
            .claims;
        let tools = Self::tool_router()
            .list_all()
            .into_iter()
            .filter(|tool| claims.allows(&tool.name))
            .collect();
        Ok(rmcp::model::ListToolsResult {
            tools,
            meta: None,
            next_cursor: None,
        })
    }

    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
        self.client
            .access_for(&request.name)
            .await
            .map_err(capability_request_error)?;
        let call = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        Self::tool_router().call(call).await
    }
}

fn capability_request_error(error: String) -> rmcp::ErrorData {
    rmcp::ErrorData::invalid_request(
        format!("knowledge capability is no longer valid: {error}"),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tool_schemas_have_properties_field() {
        let router = KnowledgeStdioServer::tool_router();
        let tools = router.list_all();
        assert!(!tools.is_empty(), "knowledge bridge must register at least one tool");
        for tool in &tools {
            assert!(
                tool.input_schema.contains_key("properties"),
                "Tool '{}' schema missing 'properties' field: {:?}. OpenAI API rejects schemas without it.",
                tool.name,
                tool.input_schema,
            );
        }
    }

    #[test]
    fn registers_knowledge_search_tool() {
        let router = KnowledgeStdioServer::tool_router();
        let names: Vec<String> = router.list_all().iter().map(|t| t.name.to_string()).collect();
        assert!(names.contains(&"knowledge_search".to_string()), "got {names:?}");
    }

    #[test]
    fn build_search_body_contains_only_tool_arguments() {
        let body = build_search_body("how to deploy", Some(5));
        assert_eq!(body["tool"], "knowledge_search");
        assert_eq!(body["args"]["query"], "how to deploy");
        assert_eq!(body["args"]["limit"], 5);
        assert!(body.get("cwd").is_none(), "workspace must come from signed claims");
        assert!(body.get("kb_ids").is_none(), "body must not contain kb_ids, got: {body}");
    }

    #[test]
    fn build_search_body_with_none_limit() {
        let body = build_search_body("query", None);
        assert_eq!(body["args"]["limit"], serde_json::Value::Null);
    }

    #[test]
    fn registers_read_and_write_tools() {
        let router = KnowledgeStdioServer::tool_router();
        let names: Vec<String> = router.list_all().iter().map(|t| t.name.to_string()).collect();
        assert!(names.contains(&"knowledge_read".to_string()), "got {names:?}");
        assert!(names.contains(&"knowledge_write".to_string()), "got {names:?}");
        let tools = router.list_all();
        let search = tools
            .iter()
            .find(|tool| tool.name.as_ref() == "knowledge_search")
            .expect("knowledge_search registered");
        let description = search.description.as_deref().unwrap_or_default();
        assert!(description.contains("knowledge_read") && description.contains("handle"));
        assert!(!description.contains("Read tool"));
    }

    #[test]
    fn build_read_body_shape() {
        let body = build_read_body("kdoc_abc");
        assert_eq!(body["tool"], "knowledge_read");
        assert_eq!(body["args"]["handle"], "kdoc_abc");
        assert!(body.get("cwd").is_none());
        assert!(body.get("kb_ids").is_none(), "scope is server-resolved: {body}");
    }

    #[test]
    fn build_write_body_handle_and_path_variants() {
        let upd = WriteParams { handle: Some("kdoc_x".into()), base: None, rel_path: None, content: "merged".into() };
        let b = build_write_body(&upd);
        assert_eq!(b["tool"], "knowledge_write");
        assert_eq!(b["args"]["handle"], "kdoc_x");
        assert_eq!(b["args"]["content"], "merged");
        assert!(b.get("cwd").is_none());
        assert!(b.get("kb_ids").is_none(), "no kb_ids/policy in body: {b}");

        let create = WriteParams { handle: None, base: Some("Finance".into()), rel_path: Some("terms.md".into()), content: "c".into() };
        let b2 = build_write_body(&create);
        assert_eq!(b2["args"]["base"], "Finance");
        assert_eq!(b2["args"]["rel_path"], "terms.md");
        assert_eq!(b2["args"]["handle"], serde_json::Value::Null);
    }
}

/// Build the HTTP body for forwarding a knowledge_search tool call to the
/// in-process `KnowledgeMcpServer`. Pure function — easily testable.
///
/// Shape: `{"tool":"knowledge_search","args":{"query":..,"limit":..}}`.
/// Authoritative workspace and base ids live only in the signed claims.
pub(crate) fn build_search_body(query: &str, limit: Option<u32>) -> serde_json::Value {
    serde_json::json!({
        "tool": "knowledge_search",
        "args": { "query": query, "limit": limit },
    })
}

/// Body for `knowledge_read`; the server checks the handle against signed base ids.
pub(crate) fn build_read_body(handle: &str) -> serde_json::Value {
    serde_json::json!({
        "tool": "knowledge_read",
        "args": { "handle": handle },
    })
}

/// Body for `knowledge_write`. The signed tool allowlist gates write access and
/// the server re-resolves the persisted workpath binding before committing.
pub(crate) fn build_write_body(p: &WriteParams) -> serde_json::Value {
    serde_json::json!({
        "tool": "knowledge_write",
        "args": { "handle": p.handle, "base": p.base, "rel_path": p.rel_path, "content": p.content },
    })
}

impl KnowledgeStdioServer {
    async fn forward_tool(&self, query: String, limit: Option<u32>) -> String {
        let body = build_search_body(&query, limit);
        self.client
            .forward_tool("knowledge_search", body, false)
            .await
    }

    async fn forward_read(&self, handle: String) -> String {
        let body = build_read_body(&handle);
        self.client
            .forward_tool("knowledge_read", body, false)
            .await
    }

    async fn forward_write(&self, params: WriteParams) -> String {
        let body = build_write_body(&params);
        self.client
            .forward_tool("knowledge_write", body, false)
            .await
    }
}
