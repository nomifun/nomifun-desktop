//! Session-bound MCP resource tools for the Nomi runtime.
//!
//! The host supplies managers containing only servers resolved from the
//! current AgentSession's typed `mcp_server` bindings. Model input can select a
//! server from that frozen set, but cannot provide transport, credentials, an
//! owner, or a new server identity.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nomi_mcp::manager::McpManager;
use nomi_protocol::events::ToolCategory;
use nomi_tools::Tool;
use nomi_types::tool::{JsonSchema, ToolResult};
use serde::Deserialize;
use serde_json::{Value, json};

pub const MCP_RESOURCE_LIST_TOOL_NAME: &str = "mcp_resource_list";
pub const MCP_RESOURCE_READ_TOOL_NAME: &str = "mcp_resource_read";
const MCP_RESOURCE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESOURCE_TEXT_BYTES: usize = 512 * 1024;
const MAX_SERVER_NAME_BYTES: usize = 256;
const MAX_RESOURCE_URI_BYTES: usize = 4096;

#[derive(Clone)]
pub struct McpResourceListTool {
    managers: Vec<Arc<McpManager>>,
}

#[derive(Clone)]
pub struct McpResourceReadTool {
    managers: Vec<Arc<McpManager>>,
}

impl McpResourceListTool {
    pub fn new(managers: Vec<Arc<McpManager>>) -> Self {
        Self { managers }
    }
}

impl McpResourceReadTool {
    pub fn new(managers: Vec<Arc<McpManager>>) -> Self {
        Self { managers }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerInput {
    server: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadInput {
    server: String,
    uri: String,
}

fn input_schema(include_uri: bool) -> JsonSchema {
    let mut properties = serde_json::Map::from_iter([(
        "server".to_owned(),
        json!({
            "type": "string",
            "minLength": 1,
            "maxLength": MAX_SERVER_NAME_BYTES,
            "description": "Exact server name from the current AgentSession's bound MCP set."
        }),
    )]);
    let mut required = vec!["server"];
    if include_uri {
        properties.insert(
            "uri".to_owned(),
            json!({
                "type": "string",
                "minLength": 1,
                "maxLength": MAX_RESOURCE_URI_BYTES,
                "description": "Exact resource URI returned by mcp_resource_list."
            }),
        );
        required.push("uri");
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn exact_manager<'a>(
    managers: &'a [Arc<McpManager>],
    server: &str,
) -> Result<&'a Arc<McpManager>, ToolResult> {
    validate_exact_selector("server", server, MAX_SERVER_NAME_BYTES)?;
    let mut matches = managers
        .iter()
        .filter(|manager| manager.server_names().iter().any(|name| name == server));
    let manager = matches.next().ok_or_else(|| {
        error(
            "PRESET_RESOURCE_NOT_BOUND",
            "the requested MCP server is not bound to this AgentSession",
        )
    })?;
    if matches.next().is_some() {
        return Err(error(
            "CAPABILITY_UNAVAILABLE",
            "the bound MCP server identity is ambiguous",
        ));
    }
    if !manager.server_supports_resources(server) {
        return Err(error(
            "CAPABILITY_UNAVAILABLE",
            "the bound MCP server does not advertise resources",
        ));
    }
    Ok(manager)
}

fn validate_exact_selector(
    field: &str,
    value: &str,
    max_bytes: usize,
) -> Result<(), ToolResult> {
    if value.is_empty()
        || value != value.trim()
        || value.len() > max_bytes
        || value.chars().any(char::is_control)
    {
        return Err(error(
            "INVALID_PAYLOAD",
            &format!(
                "{field} must be a trimmed non-empty value of at most {max_bytes} bytes"
            ),
        ));
    }
    Ok(())
}

#[async_trait]
impl Tool for McpResourceListTool {
    fn name(&self) -> &str {
        MCP_RESOURCE_LIST_TOOL_NAME
    }

    fn description(&self) -> &str {
        "List resources from one MCP server already bound to the current AgentSession."
    }

    fn input_schema(&self) -> JsonSchema {
        input_schema(false)
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let input: ServerInput = match serde_json::from_value::<ServerInput>(input) {
            Ok(input) => input,
            Err(_) => return error("INVALID_PAYLOAD", "server is required"),
        };
        let manager = match exact_manager(&self.managers, &input.server) {
            Ok(manager) => manager,
            Err(error) => return error,
        };
        match manager.list_resources(&input.server).await {
            Ok(resources) => ToolResult::text(
                json!({
                    "server": input.server,
                    "resources": resources.into_iter().map(|resource| json!({
                        "uri": resource.uri,
                        "name": resource.name,
                        "description": resource.description,
                        "mime_type": resource.mime_type,
                    })).collect::<Vec<_>>()
                })
                .to_string(),
            ),
            Err(failure) => error("MCP_RESOURCE_FAILED", &failure.to_string()),
        }
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }

    fn execution_timeout(&self, _input: &Value) -> Duration {
        MCP_RESOURCE_TIMEOUT
    }
}

#[async_trait]
impl Tool for McpResourceReadTool {
    fn name(&self) -> &str {
        MCP_RESOURCE_READ_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Read one exact resource URI from an MCP server already bound to the current AgentSession."
    }

    fn input_schema(&self) -> JsonSchema {
        input_schema(true)
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let input: ReadInput = match serde_json::from_value::<ReadInput>(input) {
            Ok(input) => input,
            _ => return error("INVALID_PAYLOAD", "server and uri are required"),
        };
        if let Err(error) = validate_exact_selector("uri", &input.uri, MAX_RESOURCE_URI_BYTES) {
            return error;
        }
        let manager = match exact_manager(&self.managers, &input.server) {
            Ok(manager) => manager,
            Err(error) => return error,
        };
        match manager.read_resource(&input.server, &input.uri).await {
            Ok(text) if text.len() <= MAX_RESOURCE_TEXT_BYTES => ToolResult::text(
                json!({
                    "server": input.server,
                    "uri": input.uri,
                    "text": text,
                })
                .to_string(),
            ),
            Ok(_) => error(
                "MCP_RESOURCE_TOO_LARGE",
                "the MCP resource exceeds the 512 KiB Agent context limit",
            ),
            Err(failure) => error("MCP_RESOURCE_FAILED", &failure.to_string()),
        }
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }

    fn execution_timeout(&self, _input: &Value) -> Duration {
        MCP_RESOURCE_TIMEOUT
    }

    fn max_result_size(&self) -> usize {
        MAX_RESOURCE_TEXT_BYTES + 4096
    }
}

fn error(code: &str, message: &str) -> ToolResult {
    ToolResult {
        content: json!({"code": code, "message": message}).to_string(),
        is_error: true,
        images: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use nomi_mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
    use nomi_mcp::transport::{McpError, McpTransport};

    use super::*;

    struct RecordingTransport {
        responses: Mutex<VecDeque<Value>>,
        requests: Arc<Mutex<Vec<Value>>>,
    }

    #[async_trait]
    impl McpTransport for RecordingTransport {
        async fn request(&self, request: &JsonRpcRequest) -> Result<JsonRpcResponse, McpError> {
            self.requests
                .lock()
                .unwrap()
                .push(serde_json::to_value(request).unwrap());
            let result = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("fixture response");
            Ok(JsonRpcResponse {
                jsonrpc: "2.0".to_owned(),
                id: request.id,
                result: Some(result),
                error: None,
            })
        }

        async fn notify(&self, _request: &JsonRpcRequest) -> Result<(), McpError> {
            Ok(())
        }

        async fn close(&self) -> Result<(), McpError> {
            Ok(())
        }
    }

    fn manager(
        name: &str,
        supports_resources: bool,
        responses: impl IntoIterator<Item = Value>,
    ) -> (Arc<McpManager>, Arc<Mutex<Vec<Value>>>) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let transport = RecordingTransport {
            responses: Mutex::new(responses.into_iter().collect()),
            requests: Arc::clone(&requests),
        };
        (
            Arc::new(McpManager::new_for_test(vec![(
                name,
                supports_resources,
                Box::new(transport),
            )])),
            requests,
        )
    }

    #[tokio::test]
    async fn resource_tools_use_only_the_exact_session_bound_server() {
        let (manager, requests) = manager(
            "bound",
            true,
            [
                json!({
                    "resources": [{
                        "uri": "skill://guide",
                        "name": "Guide",
                        "mimeType": "text/markdown"
                    }]
                }),
                json!({
                    "contents": [{
                        "uri": "skill://guide",
                        "mimeType": "text/markdown",
                        "text": "# Bound guide"
                    }]
                }),
            ],
        );
        let list = McpResourceListTool::new(vec![Arc::clone(&manager)])
            .execute(json!({"server": "bound"}))
            .await;
        assert!(!list.is_error, "{}", list.content);
        let list: Value = serde_json::from_str(&list.content).unwrap();
        assert_eq!(list["resources"][0]["uri"], "skill://guide");

        let read = McpResourceReadTool::new(vec![manager])
            .execute(json!({"server": "bound", "uri": "skill://guide"}))
            .await;
        assert!(!read.is_error, "{}", read.content);
        let read: Value = serde_json::from_str(&read.content).unwrap();
        assert_eq!(read["text"], "# Bound guide");

        let requests = requests.lock().unwrap();
        assert_eq!(requests[0]["method"], "resources/list");
        assert_eq!(requests[1]["method"], "resources/read");
        assert_eq!(requests[1]["params"]["uri"], "skill://guide");
    }

    #[tokio::test]
    async fn resource_tools_reject_unbound_ambiguous_and_inexact_selectors() {
        let (first, _) = manager("bound", true, []);
        let (second, _) = manager("bound", true, []);
        let (without_resources, _) = manager("plain", false, []);

        let unbound = McpResourceListTool::new(vec![Arc::clone(&first)])
            .execute(json!({"server": "other"}))
            .await;
        assert!(unbound.is_error);
        assert!(unbound.content.contains("PRESET_RESOURCE_NOT_BOUND"));

        let ambiguous = McpResourceListTool::new(vec![first, second])
            .execute(json!({"server": "bound"}))
            .await;
        assert!(ambiguous.is_error);
        assert!(ambiguous.content.contains("CAPABILITY_UNAVAILABLE"));

        let unsupported = McpResourceListTool::new(vec![without_resources])
            .execute(json!({"server": "plain"}))
            .await;
        assert!(unsupported.is_error);
        assert!(unsupported.content.contains("CAPABILITY_UNAVAILABLE"));

        let inexact = McpResourceReadTool::new(Vec::new())
            .execute(json!({"server": " bound ", "uri": "skill://guide"}))
            .await;
        assert!(inexact.is_error);
        assert!(inexact.content.contains("INVALID_PAYLOAD"));

        let oversized_uri = format!("custom:{}", "x".repeat(MAX_RESOURCE_URI_BYTES));
        let oversized = McpResourceReadTool::new(Vec::new())
            .execute(json!({"server": "bound", "uri": oversized_uri}))
            .await;
        assert!(oversized.is_error);
        assert!(oversized.content.contains("INVALID_PAYLOAD"));
    }
}
