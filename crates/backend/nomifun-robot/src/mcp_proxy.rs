//! Per-turn MCP transport for authorized device tools. A URL or a capability
//! query string never grants authority; only a live host-issued lease does.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use nomifun_api_types::{McpServerId, SessionMcpServer, SessionMcpTransport};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use crate::mcp_bridge::ToolCallError;
use crate::tool_registry::{RobotToolRegistry, tool_capability};

pub const MCP_PROXY_SERVER_NAME: &str = "robot";

#[async_trait::async_trait]
pub trait RobotToolAuthority: Send + Sync {
    fn robot_id(&self) -> &str;
    fn connection_id(&self) -> &str;
    /// Revalidate binding, connection, permissions and the frozen Agent ceiling.
    async fn validate(&self, capability: &str) -> Result<(), String>;
    async fn validate_tool(&self, device_name: &str) -> Result<(), String> {
        self.validate(tool_capability(device_name).capability_id()).await
    }
}

pub struct RobotMcpLease {
    authority: Arc<dyn RobotToolAuthority>,
    token: String,
    port: u16,
}

impl RobotMcpLease {
    pub fn registration(&self) -> SessionMcpServer {
        SessionMcpServer {
            mcp_server_id: McpServerId::parse(uuid::Uuid::now_v7().to_string()).expect("fresh canonical ID"),
            name: MCP_PROXY_SERVER_NAME.to_owned(),
            transport: SessionMcpTransport::StreamableHttp {
                url: format!("http://127.0.0.1:{}/robot-mcp/{}", self.port, self.authority.robot_id()),
                headers: HashMap::from([("Authorization".to_owned(), format!("Bearer {}", self.token))]),
            },
        }
    }
}

#[derive(Clone)]
struct ProxyState {
    tools: Arc<RobotToolRegistry>,
    leases: Arc<Mutex<HashMap<String, Weak<RobotMcpLease>>>>,
}

pub struct RobotMcpProxyServer {
    pub port: u16,
    leases: Arc<Mutex<HashMap<String, Weak<RobotMcpLease>>>>,
    task: JoinHandle<()>,
}

impl RobotMcpProxyServer {
    pub async fn spawn(tools: Arc<RobotToolRegistry>) -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let leases = Arc::new(Mutex::new(HashMap::new()));
        let app = Router::new().route("/robot-mcp/{robot_id}", post(handle_rpc))
            .with_state(ProxyState { tools, leases: leases.clone() });
        let task = tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, app).await {
                tracing::error!(%error, "robot MCP transport stopped");
            }
        });
        Ok(Self { port, leases, task })
    }

    pub async fn issue(&self, authority: Arc<dyn RobotToolAuthority>) -> anyhow::Result<Arc<RobotMcpLease>> {
        authority.validate("robot.link").await.map_err(anyhow::Error::msg)?;
        use rand::RngCore;
        let mut secret = [0u8; 32];
        rand::rng().fill_bytes(&mut secret);
        let token: String = secret.iter().map(|byte| format!("{byte:02x}")).collect();
        let lease = Arc::new(RobotMcpLease { authority, token: token.clone(), port: self.port });
        let mut leases = self.leases.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        leases.retain(|_, lease| lease.strong_count() > 0);
        if leases.len() >= 256 { anyhow::bail!("too many active robot tool leases"); }
        leases.insert(token, Arc::downgrade(&lease));
        Ok(lease)
    }

    pub fn stop(&self) { self.task.abort(); }
}

impl Drop for RobotMcpProxyServer { fn drop(&mut self) { self.task.abort(); } }

fn rpc_error(id: Value, code: i64, message: impl Into<String>) -> Response {
    Json(json!({"jsonrpc":"2.0", "id":id, "error":{"code":code,"message":message.into()}})).into_response()
}

async fn handle_rpc(
    State(state): State<ProxyState>, Path(robot_id): Path<String>,
    headers: HeaderMap, Json(body): Json<Value>,
) -> Response {
    let token = headers.get("authorization").and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ")).unwrap_or_default();
    let lease = state.leases.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(token).and_then(Weak::upgrade);
    let Some(lease) = lease else { return (StatusCode::UNAUTHORIZED, "expired device turn").into_response(); };
    if lease.authority.robot_id() != robot_id {
        return (StatusCode::FORBIDDEN, "device turn targets another robot").into_response();
    }
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    if let Err(error) = lease.authority.validate("robot.link").await {
        return rpc_error(id, -32000, error);
    }
    let method = body["method"].as_str().unwrap_or_default();
    if id.is_null() || method.starts_with("notifications/") { return StatusCode::ACCEPTED.into_response(); }
    match method {
        "initialize" => Json(json!({"jsonrpc":"2.0", "id":id, "result":{
            "protocolVersion":"2024-11-05", "capabilities":{"tools":{}},
            "serverInfo":{"name":MCP_PROXY_SERVER_NAME,"version":env!("CARGO_PKG_VERSION")},
        }})).into_response(),
        "ping" => Json(json!({"jsonrpc":"2.0","id":id,"result":{}})).into_response(),
        "tools/list" => {
            let mut tools = Vec::new();
            for tool in state.tools.tools(&robot_id).await {
                if lease.authority.validate_tool(&tool.device_name).await.is_ok() {
                    tools.push(json!({"name":tool.exposed_name,"description":tool.description,"inputSchema":tool.input_schema}));
                }
            }
            Json(json!({"jsonrpc":"2.0","id":id,"result":{"tools":tools}})).into_response()
        }
        "tools/call" => {
            let Some(name) = body["params"]["name"].as_str() else { return rpc_error(id, -32602, "missing tool name"); };
            let tools = state.tools.tools(&robot_id).await;
            let Some(tool) = tools.iter().find(|tool| tool.exposed_name == name) else {
                return rpc_error(id, -32601, format!("unknown tool {name}"));
            };
            let capability = tool_capability(&tool.device_name);
            if let Err(error) = lease.authority.validate_tool(&tool.device_name).await {
                return rpc_error(id, -32601, error);
            }
            let args = body["params"].get("arguments").cloned().unwrap_or_else(|| json!({}));
            if !args.is_object() { return rpc_error(id, -32602, "tool arguments must be an object"); }
            match state.tools.call_for_connection(&robot_id, lease.authority.connection_id(), capability, name, args).await {
                Ok(text) => Json(json!({"jsonrpc":"2.0","id":id,"result":{
                    "content":[{"type":"text","text":text}],"isError":false,
                }})).into_response(),
                Err(ToolCallError::Rejected(error)) => rpc_error(id, -32601, error),
                Err(error) => rpc_error(id, -32000, error.to_string()),
            }
        }
        _ => rpc_error(id, -32601, format!("unknown method {method}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use crate::mcp_bridge::{RobotMcpClient, RobotToolDescriptor};
    use crate::link::Frame;

    struct Authority { allowed: BTreeSet<String>, valid: AtomicBool }
    #[async_trait::async_trait]
    impl RobotToolAuthority for Authority {
        fn robot_id(&self) -> &str { "robot-1" }
        fn connection_id(&self) -> &str { "socket-1" }
        async fn validate(&self, capability: &str) -> Result<(), String> {
            if !self.valid.load(Ordering::SeqCst) { return Err("device authority revoked".to_owned()); }
            if !self.allowed.contains(capability) { return Err("outside capability ceiling".to_owned()); }
            Ok(())
        }
    }

    fn tools() -> Vec<RobotToolDescriptor> {
        vec![RobotToolDescriptor { device_name: "self.gimbal.look".to_owned(),
            exposed_name: "robot_gimbal_look".to_owned(), description: "Look left".to_owned(),
            input_schema: json!({"type":"object"}), }]
    }

    async fn fixture(motion: bool) -> (RobotMcpProxyServer, Arc<RobotMcpLease>, Arc<Authority>, Arc<RobotToolRegistry>, Arc<AtomicUsize>) {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let client = Arc::new(RobotMcpClient::new(tx, "socket-1".to_owned()));
        let responder = client.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let call_counter = calls.clone();
        tokio::spawn(async move {
            while let Some(Frame::Text(frame)) = rx.recv().await {
                let value: Value = serde_json::from_str(&frame).unwrap();
                call_counter.fetch_add(1, Ordering::SeqCst);
                responder.handle_incoming(json!({"jsonrpc":"2.0", "id":value["payload"]["id"],
                    "result":{"content":[{"type":"text","text":"moved"}],"isError":false},
                })).await;
            }
        });
        let registry = Arc::new(RobotToolRegistry::default());
        registry.attach("robot-1", client, tools()).await;
        let server = RobotMcpProxyServer::spawn(registry.clone()).await.unwrap();
        let mut allowed = BTreeSet::from(["robot.link".to_owned()]);
        if motion { allowed.insert("robot.motion".to_owned()); }
        let authority = Arc::new(Authority { allowed, valid: AtomicBool::new(true) });
        let lease = server.issue(authority.clone()).await.unwrap();
        (server, lease, authority, registry, calls)
    }

    async fn rpc(server: &RobotMcpProxyServer, token: &str, suffix: &str, body: Value) -> (u16, Value) {
        let response = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{}/robot-mcp/{suffix}", server.port))
            .bearer_auth(token).json(&body).send().await.unwrap();
        let status = response.status().as_u16();
        let body = response.text().await.unwrap();
        (status, serde_json::from_str(&body).unwrap_or(Value::String(body)))
    }

    fn call() -> Value { json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"robot_gimbal_look","arguments":{}}}) }

    #[tokio::test]
    async fn leased_transport_lists_and_executes_the_exact_device_tool() {
        let (server, lease, _, _, calls) = fixture(true).await;
        let (_, initialized) = rpc(&server, &lease.token, "robot-1", json!({"id":1,"method":"initialize"})).await;
        assert_eq!(initialized["result"]["serverInfo"]["name"], "robot");
        let (_, list) = rpc(&server, &lease.token, "robot-1", json!({"id":2,"method":"tools/list"})).await;
        assert_eq!(list["result"]["tools"][0]["name"], "robot_gimbal_look");
        let (_, reply) = rpc(&server, &lease.token, "robot-1", call()).await;
        assert_eq!(reply["result"]["content"][0]["text"], "moved");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn url_queries_cannot_expand_the_companion_capability_ceiling() {
        let (server, lease, _, _, calls) = fixture(false).await;
        let suffix = "robot-1?capabilities=robot.motion";
        let (_, list) = rpc(&server, &lease.token, suffix, json!({"id":1,"method":"tools/list"})).await;
        assert_eq!(list["result"]["tools"], json!([]));
        let (_, reply) = rpc(&server, &lease.token, suffix, call()).await;
        assert_eq!(reply["error"]["code"], -32601);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn expired_or_forged_leases_and_wrong_devices_are_rejected() {
        let (server, lease, _, _, calls) = fixture(true).await;
        assert_eq!(rpc(&server, "forged", "robot-1", call()).await.0, 401);
        assert_eq!(rpc(&server, &lease.token, "robot-2", call()).await.0, 403);
        let token = lease.token.clone();
        drop(lease);
        assert_eq!(rpc(&server, &token, "robot-1", call()).await.0, 401);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn permission_revocation_is_rechecked_after_discovery() {
        let (server, lease, authority, _, calls) = fixture(true).await;
        let (_, list) = rpc(&server, &lease.token, "robot-1", json!({"id":1,"method":"tools/list"})).await;
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 1);
        authority.valid.store(false, Ordering::SeqCst);
        assert!(rpc(&server, &lease.token, "robot-1", call()).await.1.get("error").is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn reconnect_cannot_redirect_an_old_turn_to_a_new_socket() {
        let (server, lease, _, registry, calls) = fixture(true).await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        registry.attach("robot-1", Arc::new(RobotMcpClient::new(tx, "socket-2".to_owned())), tools()).await;
        let (_, reply) = rpc(&server, &lease.token, "robot-1", call()).await;
        assert!(reply.get("error").is_some());
        assert!(rx.try_recv().is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
