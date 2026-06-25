//! `nomicore mcp-requirement-stdio` subcommand: MCP stdio server for the
//! requirement *declaration* tools (`requirement_complete` /
//! `requirement_update_status`).
//!
//! Spawned by ACP agent CLIs (claude / codex / gemini) when the requirement MCP
//! is injected into an AutoWork session. Uses the `rmcp` crate (Rust MCP SDK)
//! for protocol handling so it is byte-compatible with each CLI's MCP client.
//!
//! Tool calls are forwarded as authenticated HTTP POSTs to the in-process
//! `RequirementMcpServer` running in the main backend process at
//! `http://127.0.0.1:{NOMI_REQ_MCP_PORT}/tool`. This stdio→HTTP hop exists
//! because the spawned process cannot share the main process's
//! `RequirementService`, and because claude / codex / gemini advertise
//! stdio-only MCP capabilities (a direct HTTP MCP server would be dropped by
//! the ACP capability filter).

// Pre-existing layout convention (mirrors team_guide): the `forward_tool` impl
// block lives after the test module.
#![allow(clippy::items_after_test_module)]

use std::process::ExitCode;

use nomifun_api_types::RequirementMcpConfig;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{schemars, service::ServiceExt, tool, tool_router, transport};
use serde::Deserialize;

pub async fn run_requirement_stdio() -> ExitCode {
    let port = match std::env::var(RequirementMcpConfig::ENV_PORT) {
        Ok(p) => p,
        Err(_) => {
            eprintln!("[mcp-requirement-stdio] ERROR: missing {}", RequirementMcpConfig::ENV_PORT);
            return ExitCode::from(1);
        }
    };
    let token = match std::env::var(RequirementMcpConfig::ENV_TOKEN) {
        Ok(t) => t,
        Err(_) => {
            eprintln!("[mcp-requirement-stdio] ERROR: missing {}", RequirementMcpConfig::ENV_TOKEN);
            return ExitCode::from(1);
        }
    };
    let conversation_id = std::env::var(RequirementMcpConfig::ENV_CONVERSATION_ID).unwrap_or_default();
    // owner_kind: "conversation" (default, back-compat) or "terminal". Controls
    // verify_scope's cross-domain check on the server side.
    let owner_kind = std::env::var(RequirementMcpConfig::ENV_OWNER_KIND)
        .unwrap_or_else(|_| "conversation".to_owned());

    eprintln!("[mcp-requirement-stdio] Started OK. PORT={port}, CONV_ID={conversation_id}, KIND={owner_kind}");

    let http_client = super::stdio_common::build_bridge_http_client();

    let server = RequirementStdioServer {
        port: port.parse().unwrap_or(0),
        token,
        conversation_id,
        owner_kind,
        http_client,
    };

    let transport = transport::io::stdio();
    match server.serve(transport).await {
        Ok(peer) => {
            eprintln!("[mcp-requirement-stdio] MCP session started, waiting for completion...");
            if let Err(e) = peer.waiting().await {
                eprintln!("[mcp-requirement-stdio] Session ended with error: {e}");
            } else {
                eprintln!("[mcp-requirement-stdio] Session ended normally");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("[mcp-requirement-stdio] Failed to start MCP server: {e}");
            ExitCode::from(1)
        }
    }
}

#[derive(Clone)]
struct RequirementStdioServer {
    port: u16,
    token: String,
    conversation_id: String,
    /// "conversation" or "terminal" — forwarded to verify_scope on the server.
    owner_kind: String,
    http_client: reqwest::Client,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct CompleteParams {
    /// The id of the requirement you are completing. It is given to you verbatim
    /// in the AutoWork prompt ("id: ...").
    id: i64,
    /// A concise note describing what you did to complete the requirement.
    #[serde(default)]
    completion_note: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct UpdateStatusParams {
    /// The id of the requirement to update. It is given to you verbatim in the
    /// AutoWork prompt ("id: ...").
    id: i64,
    /// New status. One of: "in_progress", "done", "failed".
    status: String,
    /// Optional note or failure reason (recommended when status is "failed").
    #[serde(default)]
    note: Option<String>,
}

#[tool_router(server_handler)]
impl RequirementStdioServer {
    #[tool(
        name = "requirement_complete",
        description = "Mark the current AutoWork requirement as successfully completed. Call this exactly once, with the requirement id from the prompt, when the work is fully done."
    )]
    async fn requirement_complete(&self, Parameters(params): Parameters<CompleteParams>) -> String {
        eprintln!("[mcp-requirement-stdio] tools/call: requirement_complete");
        self.forward_tool(
            "requirement_complete",
            &serde_json::json!({
                "id": params.id,
                "completion_note": params.completion_note,
            }),
        )
        .await
    }

    #[tool(
        name = "requirement_update_status",
        description = "Update the status of the current AutoWork requirement. Use status=\"failed\" with a reason if you cannot complete it; status=\"done\" is equivalent to requirement_complete."
    )]
    async fn requirement_update_status(&self, Parameters(params): Parameters<UpdateStatusParams>) -> String {
        eprintln!("[mcp-requirement-stdio] tools/call: requirement_update_status");
        self.forward_tool(
            "requirement_update_status",
            &serde_json::json!({
                "id": params.id,
                "status": params.status,
                "note": params.note,
            }),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tool_schemas_have_properties_field() {
        let router = RequirementStdioServer::tool_router();
        let tools = router.list_all();
        assert!(!tools.is_empty(), "requirement bridge must register at least one tool");
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
    fn registers_both_requirement_tools() {
        let router = RequirementStdioServer::tool_router();
        let names: Vec<String> = router.list_all().iter().map(|t| t.name.to_string()).collect();
        assert!(names.contains(&"requirement_complete".to_string()), "got {names:?}");
        assert!(names.contains(&"requirement_update_status".to_string()), "got {names:?}");
    }
}

impl RequirementStdioServer {
    async fn forward_tool(&self, tool_name: &str, args: &serde_json::Value) -> String {
        let body = serde_json::json!({
            "tool": tool_name,
            "args": args,
            "conversation_id": self.conversation_id,
            "owner_kind": self.owner_kind,
        });
        super::stdio_common::forward_tool_http(
            &self.http_client,
            self.port,
            &self.token,
            "mcp-requirement-stdio",
            &body,
            false,
        )
        .await
    }
}
