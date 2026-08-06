//! The device is an MCP **server**; we are its client.
//!
//! Three firmware quirks shape this file, and none of them are optional:
//!
//! 1. Request `id` **must be a number**. A string id makes the firmware drop the
//!    message with no reply at all, so every call would hang.
//! 2. Methods starting with `notifications` are ignored outright — nothing may
//!    depend on a notification arriving.
//! 3. `tools/list` truncates at 8000 bytes and pages via `nextCursor = <tool
//!    name>`, so one request is not enough to see the whole toolset.
//!
//! Its error objects also omit JSON-RPC's mandatory `code`, which is why this
//! module carries its own tolerant response type instead of reusing
//! `nomi_mcp::protocol::JsonRpcResponse` (whose `code` is required and would
//! fail to deserialize).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::time::{Duration, timeout};

use crate::link::Frame;
use crate::protocol::{ServerMessage, serialize_server_message};

/// How long to wait for a tool result. Matches the firmware's own 30 s HTTP
/// ceiling for `take_photo`, the slowest tool it has.
pub const TOOL_CALL_TIMEOUT_SECS: u64 = 30;
/// Default tool-thread stack the firmware allocates when we say nothing.
const DEFAULT_STACK: u64 = 6_144;
/// Stack for tools that encode JPEG and open TLS.
const CAMERA_STACK: u64 = 32_768;

/// A device tool, with both its on-device name and the name models see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RobotToolDescriptor {
    pub device_name: String,
    pub exposed_name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Why a tool call did not produce a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallError {
    /// The device refused the request (bad or out-of-range arguments). Arrives
    /// as a JSON-RPC `error`, before the tool ever runs.
    Rejected(String),
    /// The tool ran and failed (`isError: true`).
    Failed(String),
    /// No reply within [`TOOL_CALL_TIMEOUT_SECS`].
    Timeout,
    /// The link is gone.
    Offline,
}

impl std::fmt::Display for ToolCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(m) => write!(f, "device rejected the call: {m}"),
            Self::Failed(m) => write!(f, "tool failed: {m}"),
            Self::Timeout => write!(f, "device did not answer in {TOOL_CALL_TIMEOUT_SECS}s"),
            Self::Offline => write!(f, "robot is offline"),
        }
    }
}

/// Turn `self.gimbal.look` into `robot_gimbal_look`: dots and spaces are not
/// valid in tool names for most providers, and the `self.` prefix carries no
/// meaning once the tool is namespaced to this robot.
pub fn exposed_tool_name(device_name: &str) -> String {
    let trimmed = device_name.strip_prefix("self.").unwrap_or(device_name);
    let sanitized: String = trimmed
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("robot_{sanitized}")
}

/// Tolerant JSON-RPC response: `code` is optional because the firmware omits it.
#[derive(Debug, Deserialize)]
struct DeviceResponse {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<DeviceError>,
}

#[derive(Debug, Deserialize)]
struct DeviceError {
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize)]
struct ToolsListPage {
    #[serde(default)]
    tools: Vec<ToolsListEntry>,
    #[serde(default, rename = "nextCursor")]
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ToolsListEntry {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default, rename = "inputSchema")]
    input_schema: Value,
}

/// Speaks JSON-RPC to the device over the `type:"mcp"` envelope.
pub struct RobotMcpClient {
    out: mpsc::Sender<Frame>,
    session_id: String,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<DeviceResponse>>>,
}

impl RobotMcpClient {
    pub fn new(out: mpsc::Sender<Frame>, session_id: String) -> Self {
        Self {
            out,
            session_id,
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Feed a `type:"mcp"` payload received from the device.
    pub async fn handle_incoming(&self, payload: Value) {
        let Ok(response) = serde_json::from_value::<DeviceResponse>(payload) else {
            return;
        };
        let Some(id) = response.id else {
            // A notification or a malformed frame: nothing is waiting on it.
            return;
        };
        if let Some(waiter) = self.pending.lock().await.remove(&id) {
            let _ = waiter.send(response);
        }
    }

    /// Send one request and await its reply.
    async fn request(&self, method: &str, params: Value) -> Result<Value, ToolCallError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        // `id` is a number on purpose: the firmware silently drops string ids.
        let payload = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let frame = Frame::Text(serialize_server_message(&ServerMessage::Mcp {
            session_id: self.session_id.clone(),
            payload,
        }));
        if self.out.send(frame).await.is_err() {
            self.pending.lock().await.remove(&id);
            return Err(ToolCallError::Offline);
        }

        let response = match timeout(Duration::from_secs(TOOL_CALL_TIMEOUT_SECS), rx).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => return Err(ToolCallError::Offline),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                return Err(ToolCallError::Timeout);
            }
        };
        if let Some(error) = response.error {
            return Err(ToolCallError::Rejected(error.message));
        }
        Ok(response.result.unwrap_or(Value::Null))
    }

    /// MCP handshake. `vision_url` is the **only** way to configure the
    /// firmware's photo-explain endpoint; when the transport has no reachable
    /// HTTP base we simply omit the capability rather than send a dead URL.
    pub async fn initialize(
        &self,
        vision_url: Option<&str>,
        vision_token: &str,
    ) -> anyhow::Result<()> {
        let mut capabilities = json!({});
        if let Some(url) = vision_url {
            capabilities["vision"] = json!({ "url": url, "token": vision_token });
        }
        self.request("initialize", json!({ "capabilities": capabilities }))
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(())
    }

    /// Full toolset, following `nextCursor` until the device stops paging.
    pub async fn list_tools(&self) -> anyhow::Result<Vec<RobotToolDescriptor>> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        // A cursor loop needs a bound: a firmware bug repeating one cursor
        // forever must not spin this task.
        for _ in 0..16 {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let result = self
                .request("tools/list", params)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let page: ToolsListPage = serde_json::from_value(result)?;
            for entry in page.tools {
                out.push(RobotToolDescriptor {
                    exposed_name: exposed_tool_name(&entry.name),
                    device_name: entry.name,
                    description: entry.description,
                    input_schema: entry.input_schema,
                });
            }
            match page.next_cursor {
                Some(next) if Some(&next) != cursor.as_ref() => cursor = Some(next),
                _ => return Ok(out),
            }
        }
        tracing::warn!("robot: tools/list paging did not terminate; using what we have");
        Ok(out)
    }

    /// Invoke a device tool by its on-device name.
    pub async fn call_tool(&self, device_name: &str, args: Value) -> Result<String, ToolCallError> {
        let stack = if device_name.contains("camera") || device_name.contains("photo") {
            CAMERA_STACK
        } else {
            DEFAULT_STACK
        };
        let result = self
            .request(
                "tools/call",
                json!({ "name": device_name, "arguments": args, "stackSize": stack }),
            )
            .await?;
        let text = result
            .get("content")
            .and_then(|c| c.as_array())
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if result
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Err(ToolCallError::Failed(text));
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    /// Spawn a client plus a fake device that answers requests with `responder`.
    fn device<F>(responder: F) -> Arc<RobotMcpClient>
    where
        F: Fn(&str, u64, &serde_json::Value) -> Option<serde_json::Value> + Send + 'static,
    {
        let (out_tx, mut out_rx) = mpsc::channel::<Frame>(32);
        let client = Arc::new(RobotMcpClient::new(out_tx, "sess-1".to_owned()));
        let echo = client.clone();
        tokio::spawn(async move {
            while let Some(frame) = out_rx.recv().await {
                let Frame::Text(raw) = frame else { continue };
                let envelope: serde_json::Value = serde_json::from_str(&raw).unwrap();
                assert_eq!(
                    envelope["type"], "mcp",
                    "device MCP travels in an mcp envelope"
                );
                assert_eq!(envelope["session_id"], "sess-1");
                let payload = &envelope["payload"];
                let method = payload["method"].as_str().unwrap_or_default();
                let id = payload["id"]
                    .as_u64()
                    .expect("the firmware drops non-numeric ids");
                if let Some(reply) = responder(method, id, &payload["params"]) {
                    echo.handle_incoming(reply).await;
                }
            }
        });
        client
    }

    #[test]
    fn exposed_names_are_model_friendly() {
        assert_eq!(exposed_tool_name("self.gimbal.look"), "robot_gimbal_look");
        assert_eq!(
            exposed_tool_name("self.audio_speaker.set_volume"),
            "robot_audio_speaker_set_volume"
        );
        assert_eq!(
            exposed_tool_name("self.camera.take_photo"),
            "robot_camera_take_photo"
        );
        assert_eq!(exposed_tool_name("weird name"), "robot_weird_name");
    }

    #[tokio::test]
    async fn initialize_delivers_the_vision_url_and_token() {
        let seen = Arc::new(std::sync::Mutex::new(None));
        let recorder = seen.clone();
        let client = device(move |method, id, params| {
            if method == "initialize" {
                *recorder.lock().unwrap() = Some(params.clone());
                return Some(json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": { "tools": {} },
                        "serverInfo": { "name": "ESP32-S3N16R8-EMOJI", "version": "1.9.0" }
                    }
                }));
            }
            None
        });

        client
            .initialize(
                Some("http://192.168.1.20:25808/robot/vision/explain"),
                "tok-1",
            )
            .await
            .unwrap();
        let params = seen.lock().unwrap().clone().expect("initialize was sent");
        assert_eq!(
            params["capabilities"]["vision"]["url"],
            "http://192.168.1.20:25808/robot/vision/explain",
            "MCP initialize is the ONLY channel that configures the vision URL"
        );
        assert_eq!(params["capabilities"]["vision"]["token"], "tok-1");
    }

    #[tokio::test]
    async fn initialize_omits_vision_when_there_is_no_reachable_url() {
        let seen = Arc::new(std::sync::Mutex::new(None));
        let recorder = seen.clone();
        let client = device(move |method, id, params| {
            if method == "initialize" {
                *recorder.lock().unwrap() = Some(params.clone());
                return Some(json!({ "jsonrpc": "2.0", "id": id, "result": {} }));
            }
            None
        });
        client.initialize(None, "tok-1").await.unwrap();
        let params = seen.lock().unwrap().clone().unwrap();
        assert!(params["capabilities"].get("vision").is_none());
    }

    #[tokio::test]
    async fn list_tools_follows_next_cursor_to_the_end() {
        let client = device(|method, id, params| {
            if method != "tools/list" {
                return None;
            }
            let cursor = params
                .get("cursor")
                .and_then(|c| c.as_str())
                .unwrap_or_default();
            let page = if cursor.is_empty() {
                json!({
                    "tools": [
                        { "name": "self.get_device_status", "description": "status", "inputSchema": { "type": "object", "properties": {} } },
                        { "name": "self.audio_speaker.set_volume", "description": "volume", "inputSchema": { "type": "object", "properties": { "volume": { "type": "integer" } } } }
                    ],
                    "nextCursor": "self.gimbal.look"
                })
            } else {
                json!({
                    "tools": [
                        { "name": "self.gimbal.look", "description": "turn the head", "inputSchema": { "type": "object", "properties": { "direction": { "type": "string" } } } }
                    ]
                })
            };
            Some(json!({ "jsonrpc": "2.0", "id": id, "result": page }))
        });

        let tools = client.list_tools().await.unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t.exposed_name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "robot_get_device_status",
                "robot_audio_speaker_set_volume",
                "robot_gimbal_look"
            ],
            "the 8000-byte page limit must not truncate the toolset"
        );
        let gimbal = tools
            .iter()
            .find(|t| t.exposed_name == "robot_gimbal_look")
            .unwrap();
        assert_eq!(gimbal.device_name, "self.gimbal.look");
        assert_eq!(gimbal.input_schema["properties"]["direction"]["type"], "string");
    }

    #[tokio::test]
    async fn call_tool_returns_the_text_content() {
        let client = device(|method, id, params| {
            if method != "tools/call" {
                return None;
            }
            assert_eq!(params["name"], "self.gimbal.set");
            assert_eq!(params["arguments"]["pan"], 100);
            assert!(
                params["stackSize"].as_u64().unwrap() >= 6144,
                "give the tool thread room"
            );
            Some(json!({
                "jsonrpc": "2.0", "id": id,
                "result": { "content": [{ "type": "text", "text": "{\"pan\":100,\"tilt\":90}" }], "isError": false }
            }))
        });

        let text = client
            .call_tool("self.gimbal.set", json!({ "pan": 100, "tilt": 90 }))
            .await
            .unwrap();
        assert_eq!(text, "{\"pan\":100,\"tilt\":90}");
    }

    #[tokio::test]
    async fn a_firmware_error_without_a_code_field_is_still_understood() {
        // The firmware's error objects have NO `code` — a strict JSON-RPC
        // deserializer would fail here, so ours must tolerate it.
        let client = device(|method, id, _params| {
            if method != "tools/call" {
                return None;
            }
            Some(json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "message": "Value exceeds maximum allowed: 130" }
            }))
        });

        let error = client
            .call_tool("self.gimbal.set", json!({ "pan": 999 }))
            .await
            .unwrap_err();
        match error {
            ToolCallError::Rejected(message) => assert!(message.contains("exceeds maximum")),
            other => panic!("out-of-range parameters are a rejection, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_is_error_result_is_a_failure_not_a_rejection() {
        let client = device(|method, id, _params| {
            if method != "tools/call" {
                return None;
            }
            Some(json!({
                "jsonrpc": "2.0", "id": id,
                "result": { "content": [{ "type": "text", "text": "Failed to capture photo" }], "isError": true }
            }))
        });

        let error = client
            .call_tool("self.camera.take_photo", json!({ "question": "?" }))
            .await
            .unwrap_err();
        match error {
            ToolCallError::Failed(message) => assert!(message.contains("Failed to capture")),
            other => panic!("a runtime tool failure is Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn camera_tools_get_a_much_larger_stack() {
        let client = device(|method, id, params| {
            if method != "tools/call" {
                return None;
            }
            assert!(
                params["stackSize"].as_u64().unwrap() >= 32_768,
                "JPEG encode plus TLS does not fit the 6144-byte default"
            );
            Some(json!({ "jsonrpc": "2.0", "id": id, "result": { "content": [], "isError": false } }))
        });
        let _ = client
            .call_tool("self.camera.take_photo", json!({ "question": "?" }))
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_device_times_out_rather_than_hanging_forever() {
        let client = device(|_method, _id, _params| None);
        let call = tokio::spawn({
            let client = client.clone();
            async move { client.call_tool("self.gimbal.center", json!({})).await }
        });
        // Paused time auto-advances once every task is parked on a timer, so the
        // 30 s ceiling is reached without the test sleeping for real.
        assert!(matches!(call.await.unwrap(), Err(ToolCallError::Timeout)));
    }

    #[tokio::test]
    async fn a_closed_link_reports_offline() {
        let (out_tx, out_rx) = mpsc::channel::<Frame>(1);
        drop(out_rx);
        let client = RobotMcpClient::new(out_tx, "sess-1".to_owned());
        assert!(matches!(
            client.call_tool("self.gimbal.center", json!({})).await,
            Err(ToolCallError::Offline)
        ));
    }

    #[tokio::test]
    async fn a_notification_from_the_device_is_ignored_safely() {
        let client = device(|_method, _id, _params| None);
        // No id, and a notifications/* method: must not panic or poison state.
        client
            .handle_incoming(json!({ "jsonrpc": "2.0", "method": "notifications/ready" }))
            .await;
        client
            .handle_incoming(json!({ "jsonrpc": "2.0", "id": 9999, "result": {} }))
            .await;
    }
}
