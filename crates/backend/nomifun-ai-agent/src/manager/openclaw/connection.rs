use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nomifun_common::AppError;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{Mutex, broadcast, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, warn};

use super::device_identity::{DeviceIdentity, build_device_auth_params};
use super::protocol::{
    AuthParams, CLIENT_DISPLAY_NAME, CLIENT_ID, CLIENT_MODE, CLIENT_VERSION, ClientInfo, ConnectParams, EventFrame,
    HelloOk, IncomingFrame, OPENCLAW_MAX_PROTOCOL_VERSION, OPENCLAW_MIN_PROTOCOL_VERSION, RequestFrame,
};

type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

type WsStream = futures_util::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

const EVENT_CHANNEL_CAPACITY: usize = 256;
const CHALLENGE_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_TICK_INTERVAL_MS: u64 = 30_000;

type PendingSender = oneshot::Sender<Result<Value, AppError>>;

pub struct AuthConfig {
    pub token: Option<String>,
    pub password: Option<String>,
}

pub struct OpenClawConnection {
    ws_sink: Mutex<Option<WsSink>>,
    pending: Mutex<HashMap<String, PendingSender>>,
    event_tx: broadcast::Sender<EventFrame>,
    connected: AtomicBool,
    challenge_tx: Mutex<Option<oneshot::Sender<Option<String>>>>,
    _reader_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    last_tick: AtomicI64,
    tick_interval_ms: AtomicU64,
}

impl OpenClawConnection {
    pub async fn connect(
        url: &str,
        auth: Option<AuthConfig>,
        identity: &DeviceIdentity,
    ) -> Result<(Arc<Self>, HelloOk), AppError> {
        let (ws_stream, _) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|e| AppError::Internal(format!("OpenClaw WebSocket connection failed: {e}")))?;

        let (sink, stream) = ws_stream.split();
        let (event_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        let (challenge_tx, challenge_rx) = oneshot::channel();
        let now = nomifun_common::now_ms();

        let conn = Arc::new(Self {
            ws_sink: Mutex::new(Some(sink)),
            pending: Mutex::new(HashMap::new()),
            event_tx,
            connected: AtomicBool::new(false),
            challenge_tx: Mutex::new(Some(challenge_tx)),
            _reader_handle: Mutex::new(None),
            last_tick: AtomicI64::new(now),
            tick_interval_ms: AtomicU64::new(DEFAULT_TICK_INTERVAL_MS),
        });

        let reader_conn = Arc::clone(&conn);
        let reader_handle = tokio::spawn(async move {
            reader_conn.run_reader(stream).await;
        });
        *conn._reader_handle.lock().await = Some(reader_handle);

        let nonce = match tokio::time::timeout(CHALLENGE_TIMEOUT, challenge_rx).await {
            Ok(Ok(nonce)) => nonce,
            _ => None,
        };

        let hello = conn.send_connect(nonce.as_deref(), auth, identity).await?;
        conn.connected.store(true, Ordering::Relaxed);

        if let Some(ref policy) = hello.policy
            && let Some(interval) = policy.tick_interval_ms
        {
            conn.tick_interval_ms.store(interval, Ordering::Relaxed);
        }

        conn.start_tick_watchdog();

        debug!(
            protocol = ?hello.protocol,
            server_version = ?hello.server.as_ref().and_then(|s| s.version.as_deref()),
            "OpenClaw handshake complete"
        );

        Ok((conn, hello))
    }

    fn start_tick_watchdog(self: &Arc<Self>) {
        let conn = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                let interval_ms = conn.tick_interval_ms.load(Ordering::Relaxed).max(1000);
                tokio::time::sleep(Duration::from_millis(interval_ms)).await;

                if !conn.connected.load(Ordering::Relaxed) {
                    break;
                }

                let last = conn.last_tick.load(Ordering::Relaxed);
                let gap = nomifun_common::now_ms() - last;
                if gap > (interval_ms as i64) * 2 {
                    warn!(
                        gap_ms = gap,
                        interval_ms = interval_ms,
                        "OpenClaw tick timeout, closing connection"
                    );
                    conn.close().await;
                    break;
                }
            }
        });
    }

    async fn send_connect(
        &self,
        nonce: Option<&str>,
        auth: Option<AuthConfig>,
        identity: &DeviceIdentity,
    ) -> Result<HelloOk, AppError> {
        let auth_params = match &auth {
            Some(a) if a.token.is_some() || a.password.is_some() => Some(AuthParams {
                token: a.token.clone(),
                password: a.password.clone(),
            }),
            _ => None,
        };

        let device_params = build_device_auth_params(identity, nonce, auth.as_ref().and_then(|a| a.token.as_deref()));

        let params = ConnectParams {
            min_protocol: OPENCLAW_MIN_PROTOCOL_VERSION,
            max_protocol: OPENCLAW_MAX_PROTOCOL_VERSION,
            client: ClientInfo {
                id: CLIENT_ID,
                display_name: CLIENT_DISPLAY_NAME,
                version: CLIENT_VERSION,
                platform: std::env::consts::OS,
                mode: CLIENT_MODE,
            },
            caps: vec!["tool-events"],
            role: Some("operator".into()),
            scopes: Some(vec!["operator.admin".into()]),
            auth: auth_params,
            device: Some(device_params),
        };

        self.request::<HelloOk>("connect", serde_json::to_value(params).unwrap_or_default())
            .await
    }

    pub async fn request<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T, AppError> {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();

        {
            let mut pending = self.pending.lock().await;
            pending.insert(id.clone(), tx);
        }

        let frame = RequestFrame {
            type_: "req",
            id: id.clone(),
            method: method.into(),
            params: Some(params),
        };
        self.ws_send_frame(&frame).await?;

        let result = tokio::time::timeout(REQUEST_TIMEOUT, rx)
            .await
            .map_err(|_| AppError::Internal(format!("OpenClaw request '{method}' timed out")))?
            .map_err(|_| AppError::Internal(format!("OpenClaw request '{method}' cancelled")))??;

        serde_json::from_value(result)
            .map_err(|e| AppError::Internal(format!("Failed to parse OpenClaw response for '{method}': {e}")))
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<EventFrame> {
        self.event_tx.subscribe()
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub async fn close(&self) {
        self.connected.store(false, Ordering::Relaxed);

        if let Some(mut sink) = self.ws_sink.lock().await.take() {
            let _ = sink.close().await;
        }

        // Fail all pending requests
        let mut pending = self.pending.lock().await;
        for (_, tx) in pending.drain() {
            let _ = tx.send(Err(AppError::Internal("Connection closed".into())));
        }
    }

    async fn run_reader(self: Arc<Self>, mut stream: WsStream) {
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    self.handle_incoming_text(&text).await;
                }
                Ok(Message::Close(_)) => {
                    debug!("OpenClaw WebSocket closed by server");
                    break;
                }
                Err(e) => {
                    warn!(error = %e, "OpenClaw WebSocket read error");
                    break;
                }
                _ => {}
            }
        }

        self.connected.store(false, Ordering::Relaxed);

        // Fail all pending requests
        let mut pending = self.pending.lock().await;
        for (_, tx) in pending.drain() {
            let _ = tx.send(Err(AppError::Internal("OpenClaw connection closed".into())));
        }
    }

    async fn handle_incoming_text(&self, text: &str) {
        let frame: IncomingFrame = match serde_json::from_str(text) {
            Ok(f) => f,
            Err(_) => {
                debug!("Unrecognized OpenClaw message, skipping");
                return;
            }
        };

        match frame {
            IncomingFrame::Res(res) => {
                let mut pending = self.pending.lock().await;
                if let Some(tx) = pending.remove(&res.id) {
                    if res.ok {
                        let _ = tx.send(Ok(res.payload.unwrap_or(Value::Null)));
                    } else {
                        let msg = res
                            .error
                            .map(|e| format!("{}: {}", e.code, e.message))
                            .unwrap_or_else(|| "Unknown error".into());
                        let _ = tx.send(Err(AppError::Internal(msg)));
                    }
                }
            }
            IncomingFrame::Event(evt) => {
                if evt.event == "connect.challenge" {
                    let nonce = evt
                        .payload
                        .as_ref()
                        .and_then(|p| p.get("nonce"))
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    if let Some(tx) = self.challenge_tx.lock().await.take() {
                        let _ = tx.send(nonce);
                    }
                    return;
                }

                if evt.event == "tick" {
                    self.last_tick.store(nomifun_common::now_ms(), Ordering::Relaxed);
                    return;
                }

                let _ = self.event_tx.send(evt);
            }
        }
    }

    async fn ws_send_frame(&self, frame: &RequestFrame) -> Result<(), AppError> {
        let text = serde_json::to_string(frame)
            .map_err(|e| AppError::Internal(format!("Failed to serialize request frame: {e}")))?;

        let mut guard = self.ws_sink.lock().await;
        let sink = guard
            .as_mut()
            .ok_or_else(|| AppError::Internal("OpenClaw WebSocket not connected".into()))?;

        sink.send(Message::Text(text.into())).await.map_err(|e| {
            error!(error = %e, "Failed to send OpenClaw WebSocket message");
            AppError::Internal(format!("OpenClaw WebSocket send failed: {e}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::device_identity::generate_identity;
    use super::*;
    use serde_json::json;
    use tokio::net::TcpListener;

    async fn spawn_mock_gateway(challenge_nonce: Option<&str>) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("ws://{addr}");
        let nonce = challenge_nonce.map(String::from);

        let handle = tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                let (mut sink, mut stream) = ws.split();

                // Send challenge
                let challenge = json!({
                    "type": "event",
                    "event": "connect.challenge",
                    "payload": { "nonce": nonce }
                });
                let _ = sink
                    .send(Message::Text(serde_json::to_string(&challenge).unwrap().into()))
                    .await;

                // Wait for connect request
                while let Some(Ok(Message::Text(text))) = stream.next().await {
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    if frame["method"] == "connect" {
                        // Send hello-ok response
                        let res = json!({
                            "type": "res",
                            "id": frame["id"],
                            "ok": true,
                            "payload": {
                                "protocol": 3,
                                "server": { "version": "1.0.0", "connId": "test-conn" },
                                "policy": { "tickIntervalMs": 30000 },
                            }
                        });
                        let _ = sink
                            .send(Message::Text(serde_json::to_string(&res).unwrap().into()))
                            .await;
                        break;
                    }
                }

                // Keep connection alive for subsequent requests
                while let Some(Ok(Message::Text(text))) = stream.next().await {
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    if frame["type"] == "req" {
                        let method = frame["method"].as_str().unwrap_or("");
                        let res = match method {
                            "sessions.reset" => json!({
                                "type": "res",
                                "id": frame["id"],
                                "ok": true,
                                "payload": {
                                    "key": "conv-1",
                                    "sessionId": "sess-1"
                                }
                            }),
                            _ => json!({
                                "type": "res",
                                "id": frame["id"],
                                "ok": true,
                                "payload": {}
                            }),
                        };
                        let _ = sink
                            .send(Message::Text(serde_json::to_string(&res).unwrap().into()))
                            .await;
                    }
                }
            }
        });

        (url, handle)
    }

    #[tokio::test]
    async fn connect_and_handshake() {
        let (url, _server) = spawn_mock_gateway(Some("test-nonce")).await;
        let conn = OpenClawConnection::connect(&url, None, &generate_identity())
            .await
            .unwrap()
            .0;
        assert!(conn.is_connected());
        conn.close().await;
    }

    #[tokio::test]
    async fn connect_without_challenge_nonce() {
        let (url, _server) = spawn_mock_gateway(None).await;
        let conn = OpenClawConnection::connect(&url, None, &generate_identity())
            .await
            .unwrap()
            .0;
        assert!(conn.is_connected());
        conn.close().await;
    }

    #[tokio::test]
    async fn request_response_correlation() {
        let (url, _server) = spawn_mock_gateway(None).await;
        let conn = OpenClawConnection::connect(&url, None, &generate_identity())
            .await
            .unwrap()
            .0;

        let result: super::super::protocol::SessionsResetResponse = conn
            .request("sessions.reset", json!({ "key": "conv-1", "reason": "new" }))
            .await
            .unwrap();

        assert_eq!(result.key.as_deref(), Some("conv-1"));
        assert_eq!(result.session_id.as_deref(), Some("sess-1"));
        conn.close().await;
    }

    #[tokio::test]
    async fn event_broadcast() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("ws://{addr}");

        let server = tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                let (mut sink, mut stream) = ws.split();

                // Send challenge
                let challenge = json!({
                    "type": "event",
                    "event": "connect.challenge",
                    "payload": {}
                });
                let _ = sink
                    .send(Message::Text(serde_json::to_string(&challenge).unwrap().into()))
                    .await;

                // Wait for connect, respond
                if let Some(Ok(Message::Text(text))) = stream.next().await {
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    let res = json!({
                        "type": "res",
                        "id": frame["id"],
                        "ok": true,
                        "payload": { "protocol": 3 }
                    });
                    let _ = sink
                        .send(Message::Text(serde_json::to_string(&res).unwrap().into()))
                        .await;
                }

                // Brief delay so client has time to subscribe before event
                tokio::time::sleep(Duration::from_millis(50)).await;

                // Send a chat event
                let chat_event = json!({
                    "type": "event",
                    "event": "chat",
                    "payload": { "state": "delta", "message": { "content": "hello" } }
                });
                let _ = sink
                    .send(Message::Text(serde_json::to_string(&chat_event).unwrap().into()))
                    .await;

                // Keep alive briefly
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });

        let conn = OpenClawConnection::connect(&url, None, &generate_identity())
            .await
            .unwrap()
            .0;
        let mut event_rx = conn.subscribe_events();

        let event = tokio::time::timeout(Duration::from_secs(2), event_rx.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(event.event, "chat");
        assert_eq!(event.payload.as_ref().unwrap()["state"].as_str(), Some("delta"));

        conn.close().await;
        server.abort();
    }

    #[tokio::test]
    async fn connection_failure_returns_error() {
        let result = OpenClawConnection::connect("ws://127.0.0.1:1", None, &generate_identity())
            .await
            .map(|(c, _)| c);
        assert!(result.is_err());
    }
}
