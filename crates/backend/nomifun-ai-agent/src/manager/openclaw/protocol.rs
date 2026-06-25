use serde::{Deserialize, Serialize};
use serde_json::Value;

// Negotiated protocol range. Gateway 2026.5.12+ requires v4 (chat events become a
// discriminated union with required `deltaText` on delta frames); older Gateways still
// speak v3. Advertising `min=3, max=4` lets the same client connect to both.
pub const OPENCLAW_MIN_PROTOCOL_VERSION: u32 = 3;
pub const OPENCLAW_MAX_PROTOCOL_VERSION: u32 = 4;

pub const CLIENT_ID: &str = "gateway-client";
pub const CLIENT_DISPLAY_NAME: &str = "Nomi-Backend";
pub const CLIENT_MODE: &str = "backend";
pub const CLIENT_VERSION: &str = "1.0.0";

// ── WebSocket Frame Types ───────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct RequestFrame {
    #[serde(rename = "type")]
    pub type_: &'static str,
    pub id: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseFrame {
    pub id: String,
    pub ok: bool,
    #[serde(default)]
    pub payload: Option<Value>,
    #[serde(default)]
    pub error: Option<ErrorShape>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EventFrame {
    pub event: String,
    #[serde(default)]
    pub payload: Option<Value>,
    #[serde(default)]
    pub seq: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorShape {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub details: Option<Value>,
    #[serde(default)]
    pub retryable: Option<bool>,
    #[serde(default, rename = "retryAfterMs")]
    pub retry_after_ms: Option<u64>,
}

/// Discriminator for incoming WebSocket messages.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum IncomingFrame {
    Res(ResponseFrame),
    Event(EventFrame),
}

// ── Connect Handshake ───────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectParams {
    pub min_protocol: u32,
    pub max_protocol: u32,
    pub client: ClientInfo,
    pub caps: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthParams>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<DeviceAuthParams>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub id: &'static str,
    pub display_name: &'static str,
    pub version: &'static str,
    pub platform: &'static str,
    pub mode: &'static str,
}

#[derive(Debug, Serialize)]
pub struct AuthParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAuthParams {
    pub id: String,
    pub public_key: String,
    pub signature: String,
    pub signed_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct HelloOk {
    #[serde(default)]
    pub protocol: Option<u32>,
    #[serde(default)]
    pub server: Option<ServerInfo>,
    #[serde(default)]
    pub policy: Option<PolicyInfo>,
    #[serde(default)]
    pub auth: Option<HelloAuthInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub conn_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyInfo {
    #[serde(default)]
    pub max_payload: Option<u64>,
    #[serde(default)]
    pub tick_interval_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloAuthInfo {
    #[serde(default)]
    pub device_token: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
}

// ── Session Management ──────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SessionsResolveParams {
    pub key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionsResolveResponse {
    pub key: String,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SessionsResetParams {
    pub key: String,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionsResetResponse {
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

// ── Chat Operations ─────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSendParams {
    pub session_key: String,
    pub message: String,
    pub idempotency_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<Value>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatAbortParams {
    pub session_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

// ── Gateway Events ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatEvent {
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub session_key: Option<String>,
    #[serde(default)]
    pub seq: Option<u64>,
    pub state: ChatEventState,
    #[serde(default)]
    pub message: Option<Value>,
    /// v4-only: incremental delta text on `state == "delta"` frames. Required by the
    /// v4 schema (`ChatDeltaEventSchema`), absent on v3 Gateways. When present it is
    /// the authoritative delta — `message` may be missing or carry only metadata.
    #[serde(default)]
    pub delta_text: Option<String>,
    /// v4-only: when true the delta replaces the accumulated text instead of appending.
    #[serde(default)]
    pub replace: Option<bool>,
    #[serde(default)]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatEventState {
    Delta,
    Final,
    Aborted,
    Error,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvent {
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub session_key: Option<String>,
    #[serde(default)]
    pub seq: Option<u64>,
    pub stream: String,
    #[serde(default)]
    pub data: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequestEvent {
    pub request_id: String,
    #[serde(default)]
    pub tool_call: Option<ApprovalToolCall>,
    #[serde(default)]
    pub options: Option<Vec<ApprovalOption>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalToolCall {
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub raw_input: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalOption {
    pub option_id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRespondParams {
    pub request_id: String,
    pub option_id: String,
}

// ── Challenge Event ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ChallengePayload {
    #[serde(default)]
    pub nonce: Option<String>,
}

// ── URL Normalization ───────────────────────────────────────────────────

pub fn normalize_ws_url(host: &str, port: u16) -> String {
    let raw = if host.contains("://") {
        format!("{host}:{port}")
    } else {
        format!("ws://{host}:{port}")
    };

    raw.replace("https://", "wss://").replace("http://", "ws://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_ws_url_bare_host() {
        assert_eq!(normalize_ws_url("127.0.0.1", 18789), "ws://127.0.0.1:18789");
        assert_eq!(normalize_ws_url("localhost", 9999), "ws://localhost:9999");
    }

    #[test]
    fn normalize_ws_url_with_scheme() {
        assert_eq!(normalize_ws_url("https://remote.host", 443), "wss://remote.host:443");
        assert_eq!(normalize_ws_url("http://local.host", 8080), "ws://local.host:8080");
        assert_eq!(normalize_ws_url("ws://already.ws", 18789), "ws://already.ws:18789");
    }

    #[test]
    fn request_frame_serializes() {
        let frame = RequestFrame {
            type_: "req",
            id: "abc-123".into(),
            method: "connect".into(),
            params: Some(serde_json::json!({"key": "value"})),
        };
        let json = serde_json::to_value(&frame).unwrap();
        assert_eq!(json["type"], "req");
        assert_eq!(json["id"], "abc-123");
        assert_eq!(json["method"], "connect");
    }

    #[test]
    fn response_frame_deserializes_ok() {
        let json = serde_json::json!({
            "id": "abc-123",
            "ok": true,
            "payload": { "protocol": 3 }
        });
        let frame: ResponseFrame = serde_json::from_value(json).unwrap();
        assert!(frame.ok);
        assert_eq!(frame.id, "abc-123");
        assert!(frame.payload.is_some());
        assert!(frame.error.is_none());
    }

    #[test]
    fn response_frame_deserializes_error() {
        let json = serde_json::json!({
            "id": "abc-123",
            "ok": false,
            "error": { "code": "AUTH_FAILED", "message": "bad token" }
        });
        let frame: ResponseFrame = serde_json::from_value(json).unwrap();
        assert!(!frame.ok);
        let err = frame.error.unwrap();
        assert_eq!(err.code, "AUTH_FAILED");
    }

    #[test]
    fn incoming_frame_dispatch() {
        let res_json = serde_json::json!({
            "type": "res",
            "id": "x",
            "ok": true,
        });
        let parsed: IncomingFrame = serde_json::from_value(res_json).unwrap();
        assert!(matches!(parsed, IncomingFrame::Res(_)));

        let evt_json = serde_json::json!({
            "type": "event",
            "event": "chat",
            "payload": {},
        });
        let parsed: IncomingFrame = serde_json::from_value(evt_json).unwrap();
        assert!(matches!(parsed, IncomingFrame::Event(_)));
    }

    #[test]
    fn chat_event_state_deserializes() {
        let json = serde_json::json!({
            "state": "delta",
            "message": { "content": "hello" },
        });
        let event: ChatEvent = serde_json::from_value(json).unwrap();
        assert_eq!(event.state, ChatEventState::Delta);
        assert!(event.delta_text.is_none());
        assert!(event.replace.is_none());
    }

    #[test]
    fn chat_event_v4_delta_with_delta_text() {
        let json = serde_json::json!({
            "runId": "run-1",
            "sessionKey": "sk-1",
            "seq": 0,
            "state": "delta",
            "deltaText": "Hello",
            "replace": false,
        });
        let event: ChatEvent = serde_json::from_value(json).unwrap();
        assert_eq!(event.state, ChatEventState::Delta);
        assert_eq!(event.delta_text.as_deref(), Some("Hello"));
        assert_eq!(event.replace, Some(false));
        assert!(event.message.is_none());
    }

    #[test]
    fn connect_params_serializes() {
        let params = ConnectParams {
            min_protocol: OPENCLAW_MIN_PROTOCOL_VERSION,
            max_protocol: OPENCLAW_MAX_PROTOCOL_VERSION,
            client: ClientInfo {
                id: CLIENT_ID,
                display_name: CLIENT_DISPLAY_NAME,
                version: CLIENT_VERSION,
                platform: "darwin",
                mode: CLIENT_MODE,
            },
            caps: vec!["tool-events"],
            role: Some("operator".into()),
            scopes: Some(vec!["operator.admin".into()]),
            auth: None,
            device: None,
        };
        let json = serde_json::to_value(&params).unwrap();
        assert_eq!(json["minProtocol"], 3);
        assert_eq!(json["maxProtocol"], 4);
        assert_eq!(json["client"]["id"], "gateway-client");
        assert_eq!(json["caps"][0], "tool-events");
    }

    #[test]
    fn hello_ok_deserializes_minimal() {
        let json = serde_json::json!({});
        let hello: HelloOk = serde_json::from_value(json).unwrap();
        assert!(hello.protocol.is_none());
        assert!(hello.policy.is_none());
    }

    #[test]
    fn hello_ok_deserializes_full() {
        let json = serde_json::json!({
            "type": "hello-ok",
            "protocol": 3,
            "server": { "version": "1.2.0", "connId": "conn-1" },
            "policy": { "tickIntervalMs": 30000 },
            "auth": { "deviceToken": "tok123", "role": "operator" },
        });
        let hello: HelloOk = serde_json::from_value(json).unwrap();
        assert_eq!(hello.protocol, Some(3));
        assert_eq!(hello.policy.as_ref().unwrap().tick_interval_ms, Some(30000));
        assert_eq!(hello.auth.as_ref().unwrap().device_token.as_deref(), Some("tok123"));
    }

    #[test]
    fn sessions_resolve_serializes() {
        let params = SessionsResolveParams { key: "sk-prev".into() };
        let json = serde_json::to_value(&params).unwrap();
        assert_eq!(json["key"], "sk-prev");
    }

    #[test]
    fn sessions_resolve_response_deserializes() {
        let json = serde_json::json!({
            "key": "sk-resolved",
            "sessionId": "sess-42"
        });
        let resp: SessionsResolveResponse = serde_json::from_value(json).unwrap();
        assert_eq!(resp.key, "sk-resolved");
        assert_eq!(resp.session_id.unwrap(), "sess-42");
    }

    #[test]
    fn sessions_reset_serializes() {
        let params = SessionsResetParams {
            key: "conv-1".into(),
            reason: "new".into(),
        };
        let json = serde_json::to_value(&params).unwrap();
        assert_eq!(json["key"], "conv-1");
        assert_eq!(json["reason"], "new");
    }

    #[test]
    fn chat_send_params_serializes() {
        let params = ChatSendParams {
            session_key: "sk-1".into(),
            message: "hello".into(),
            idempotency_key: "idem-1".into(),
            attachments: None,
        };
        let json = serde_json::to_value(&params).unwrap();
        assert_eq!(json["sessionKey"], "sk-1");
        assert_eq!(json["message"], "hello");
        assert_eq!(json["idempotencyKey"], "idem-1");
        assert!(json.get("attachments").is_none());
    }

    #[test]
    fn approval_request_deserializes() {
        let json = serde_json::json!({
            "requestId": "req-1",
            "toolCall": {
                "toolCallId": "tc-1",
                "title": "bash",
                "kind": "execute"
            },
            "options": [
                { "optionId": "allow_once", "name": "Allow", "kind": "allow_once" }
            ]
        });
        let event: ApprovalRequestEvent = serde_json::from_value(json).unwrap();
        assert_eq!(event.request_id, "req-1");
        assert_eq!(event.tool_call.unwrap().title.unwrap(), "bash");
        assert_eq!(event.options.unwrap().len(), 1);
    }
}
