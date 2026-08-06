//! A loopback MCP server fronting the connected robots.
//!
//! Registering this URL in a conversation's `extra.session_mcp_servers` is how
//! the companion's model gets `robot_*` tools with **zero** changes to the agent
//! engine: `SessionMcpTransport::Http` becomes `TransportType::StreamableHttp`
//! and the existing MCP client dials us like any other server.
//!
//! Follows the house pattern for loopback services (`ManagedModelServer`):
//! bind `127.0.0.1:0`, mint a per-boot bearer, keep the `JoinHandle`, abort on
//! `Drop`.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use rand::RngCore;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::mcp_bridge::ToolCallError;
use crate::tool_registry::RobotToolRegistry;

/// The MCP server name the model sees this toolset under.
pub const MCP_PROXY_SERVER_NAME: &str = "robot";
/// MCP protocol revision we claim (same as the firmware's own).
const PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Clone)]
struct ProxyState {
    registry: Arc<RobotToolRegistry>,
    token: String,
}

/// Loopback MCP front for robot tools.
pub struct RobotMcpProxyServer {
    pub port: u16,
    pub token: String,
    task: JoinHandle<()>,
}

impl RobotMcpProxyServer {
    /// Bind and start serving.
    pub async fn spawn(registry: Arc<RobotToolRegistry>) -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let mut secret = [0u8; 32];
        rand::rng().fill_bytes(&mut secret);
        let token: String = secret.iter().map(|b| format!("{b:02x}")).collect();

        let app = Router::new()
            .route("/robot-mcp/{robot_id}", post(handle_rpc))
            .with_state(ProxyState {
                registry,
                token: token.clone(),
            });
        let task = tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, app).await {
                tracing::error!(%error, "robot: MCP proxy server stopped");
            }
        });
        tracing::info!(port, "robot: MCP proxy listening");
        Ok(Self { port, token, task })
    }

    /// The URL to put in a conversation's `session_mcp_servers`.
    pub fn url_for(&self, robot_id: &str) -> String {
        format!("http://127.0.0.1:{}/robot-mcp/{robot_id}", self.port)
    }

    /// Headers for the same registration.
    pub fn headers(&self) -> HashMap<String, String> {
        HashMap::from([(
            "Authorization".to_owned(),
            format!("Bearer {}", self.token),
        )])
    }

    /// Stop serving.
    pub fn stop(&self) {
        self.task.abort();
    }
}

impl Drop for RobotMcpProxyServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn rpc_error(id: Option<Value>, code: i64, message: String) -> Response {
    // `code` is always present: the agent-side JSON-RPC type requires it, while
    // the firmware omits it — normalising here is the whole point of the proxy.
    Json(json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": { "code": code, "message": message },
    }))
    .into_response()
}

async fn handle_rpc(
    State(state): State<ProxyState>,
    Path(robot_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let presented = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    if presented != state.token {
        return (StatusCode::UNAUTHORIZED, "bad token").into_response();
    }

    let id = body.get("id").cloned();
    let method = body
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or_default();

    // Notifications carry no id and expect no body.
    if id.is_none() || method.starts_with("notifications") {
        return StatusCode::ACCEPTED.into_response();
    }

    match method {
        "initialize" => Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": MCP_PROXY_SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
            },
        }))
        .into_response(),
        "tools/list" => {
            let tools: Vec<Value> = state
                .registry
                .tools(&robot_id)
                .await
                .into_iter()
                .map(|t| {
                    json!({
                        "name": t.exposed_name,
                        "description": t.description,
                        "inputSchema": t.input_schema,
                    })
                })
                .collect();
            // One page always: the device's cursor paging was absorbed at attach.
            Json(json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": tools } }))
                .into_response()
        }
        "tools/call" => {
            let params = body.get("params").cloned().unwrap_or(Value::Null);
            let Some(name) = params.get("name").and_then(|n| n.as_str()) else {
                return rpc_error(id, -32602, "tools/call needs a name".to_owned());
            };
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match state.registry.call(&robot_id, name, args).await {
                Ok(text) => Json(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "content": [{ "type": "text", "text": text }], "isError": false },
                }))
                .into_response(),
                Err(ToolCallError::Rejected(message)) => rpc_error(id, -32601, message),
                Err(error @ ToolCallError::Offline) => rpc_error(id, -32000, error.to_string()),
                Err(error) => Json(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "content": [{ "type": "text", "text": error.to_string() }], "isError": true },
                }))
                .into_response(),
            }
        }
        other => rpc_error(id, -32601, format!("method not supported: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::Frame;
    use crate::mcp_bridge::{RobotMcpClient, RobotToolDescriptor};
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    fn descriptor(device_name: &str, exposed: &str) -> RobotToolDescriptor {
        RobotToolDescriptor {
            device_name: device_name.to_owned(),
            exposed_name: exposed.to_owned(),
            description: "turn the head".to_owned(),
            input_schema: json!({ "type": "object", "properties": { "direction": { "type": "string" } } }),
        }
    }

    /// A registry with one attached robot whose device answers `tools/call`.
    async fn fixture() -> (Arc<RobotToolRegistry>, Arc<RobotMcpClient>) {
        let (out_tx, mut out_rx) = mpsc::channel::<Frame>(16);
        let client = Arc::new(RobotMcpClient::new(out_tx, "sess-1".to_owned()));
        let echo = client.clone();
        tokio::spawn(async move {
            while let Some(Frame::Text(raw)) = out_rx.recv().await {
                let envelope: serde_json::Value = serde_json::from_str(&raw).unwrap();
                let payload = &envelope["payload"];
                let id = payload["id"].as_u64().unwrap();
                let name = payload["params"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                echo.handle_incoming(json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": { "content": [{ "type": "text", "text": format!("called {name}") }], "isError": false }
                }))
                .await;
            }
        });
        let registry = Arc::new(RobotToolRegistry::default());
        registry
            .attach(
                "aa:bb",
                client.clone(),
                vec![descriptor("self.gimbal.look", "robot_gimbal_look")],
            )
            .await;
        (registry, client)
    }

    async fn rpc(
        server: &RobotMcpProxyServer,
        robot_id: &str,
        body: serde_json::Value,
    ) -> serde_json::Value {
        let response = reqwest::Client::new()
            .post(server.url_for(robot_id))
            .bearer_auth(&server.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .contains("application/json"),
            "the agent's streamable-http client accepts plain JSON; keep it simple"
        );
        response.json().await.unwrap()
    }

    #[tokio::test]
    async fn initialize_answers_locally_without_touching_the_device() {
        let (registry, _client) = fixture().await;
        let server = RobotMcpProxyServer::spawn(registry).await.unwrap();
        let reply = rpc(
            &server,
            "aa:bb",
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }),
        )
        .await;
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["result"]["capabilities"]["tools"], json!({}));
        assert_eq!(reply["result"]["serverInfo"]["name"], MCP_PROXY_SERVER_NAME);
        server.stop();
    }

    #[tokio::test]
    async fn tools_list_serves_cached_exposed_names_in_one_page() {
        let (registry, _client) = fixture().await;
        let server = RobotMcpProxyServer::spawn(registry).await.unwrap();
        let reply = rpc(
            &server,
            "aa:bb",
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        )
        .await;
        let tools = reply["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "robot_gimbal_look");
        assert_eq!(
            tools[0]["inputSchema"]["properties"]["direction"]["type"],
            "string"
        );
        assert!(
            reply["result"].get("nextCursor").is_none(),
            "the device's 8000-byte paging is absorbed here, not passed on"
        );
        server.stop();
    }

    #[tokio::test]
    async fn tools_call_maps_the_exposed_name_back_to_the_device_name() {
        let (registry, _client) = fixture().await;
        let server = RobotMcpProxyServer::spawn(registry).await.unwrap();
        let reply = rpc(
            &server,
            "aa:bb",
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                    "params": { "name": "robot_gimbal_look", "arguments": { "direction": "left" } } }),
        )
        .await;
        assert_eq!(
            reply["result"]["content"][0]["text"],
            "called self.gimbal.look"
        );
        assert_eq!(reply["result"]["isError"], false);
        server.stop();
    }

    #[tokio::test]
    async fn an_unknown_tool_is_a_method_not_found_error_with_a_code() {
        let (registry, _client) = fixture().await;
        let server = RobotMcpProxyServer::spawn(registry).await.unwrap();
        let reply = rpc(
            &server,
            "aa:bb",
            json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "robot_nope" } }),
        )
        .await;
        assert_eq!(
            reply["error"]["code"], -32601,
            "the agent's client requires `code`; the firmware omits it, so we always supply one"
        );
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap()
                .contains("robot_nope")
        );
        server.stop();
    }

    #[tokio::test]
    async fn an_offline_robot_reports_an_error_instead_of_hanging() {
        let registry = Arc::new(RobotToolRegistry::default());
        let server = RobotMcpProxyServer::spawn(registry).await.unwrap();
        let reply = rpc(
            &server,
            "not-connected",
            json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": { "name": "robot_gimbal_look" } }),
        )
        .await;
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap()
                .contains("offline")
        );
        assert!(reply["error"]["code"].is_i64());
        server.stop();
    }

    #[tokio::test]
    async fn detach_empties_the_toolset() {
        let (registry, _client) = fixture().await;
        assert_eq!(registry.tools("aa:bb").await.len(), 1);
        registry.detach("aa:bb").await;
        assert!(registry.tools("aa:bb").await.is_empty());
        assert!(matches!(
            registry.call("aa:bb", "robot_gimbal_look", json!({})).await,
            Err(crate::mcp_bridge::ToolCallError::Offline)
        ));
    }

    #[tokio::test]
    async fn a_missing_or_wrong_bearer_token_is_rejected() {
        let (registry, _client) = fixture().await;
        let server = RobotMcpProxyServer::spawn(registry).await.unwrap();
        let unauthenticated = reqwest::Client::new()
            .post(server.url_for("aa:bb"))
            .json(&json!({ "jsonrpc": "2.0", "id": 6, "method": "tools/list" }))
            .send()
            .await
            .unwrap();
        assert_eq!(unauthenticated.status(), 401);

        let wrong = reqwest::Client::new()
            .post(server.url_for("aa:bb"))
            .bearer_auth("not-the-token")
            .json(&json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/list" }))
            .send()
            .await
            .unwrap();
        assert_eq!(wrong.status(), 401);
        server.stop();
    }

    #[tokio::test]
    async fn notifications_are_accepted_and_produce_no_body() {
        let (registry, _client) = fixture().await;
        let server = RobotMcpProxyServer::spawn(registry).await.unwrap();
        let response = reqwest::Client::new()
            .post(server.url_for("aa:bb"))
            .bearer_auth(&server.token)
            .json(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        server.stop();
    }

    #[tokio::test]
    async fn the_server_binds_loopback_only() {
        let (registry, _client) = fixture().await;
        let server = RobotMcpProxyServer::spawn(registry).await.unwrap();
        assert!(server.url_for("aa:bb").starts_with("http://127.0.0.1:"));
        assert_ne!(server.port, 0);
        assert_eq!(server.token.len(), 64, "per-boot 256-bit secret");
        assert_eq!(
            server.headers().get("Authorization").unwrap(),
            &format!("Bearer {}", server.token)
        );
        server.stop();
    }
}
