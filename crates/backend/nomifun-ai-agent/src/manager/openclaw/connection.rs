use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nomifun_common::AppError;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as RustlsError, SignatureScheme};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{Mutex, broadcast, oneshot};
use tokio_tungstenite::{Connector, connect_async_tls_with_config};
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

struct ConnectAttemptGuard {
    connection: Option<Arc<OpenClawConnection>>,
}

impl ConnectAttemptGuard {
    fn new(connection: Arc<OpenClawConnection>) -> Self {
        Self {
            connection: Some(connection),
        }
    }

    fn disarm(&mut self) {
        self.connection = None;
    }
}

impl Drop for ConnectAttemptGuard {
    fn drop(&mut self) {
        let Some(connection) = self.connection.take() else {
            return;
        };
        tokio::spawn(async move {
            connection.close().await;
        });
    }
}

struct PendingRequestGuard {
    pending: Arc<Mutex<HashMap<String, PendingSender>>>,
    id: Option<String>,
}

impl PendingRequestGuard {
    fn new(pending: Arc<Mutex<HashMap<String, PendingSender>>>, id: String) -> Self {
        Self {
            pending,
            id: Some(id),
        }
    }

    fn disarm(&mut self) {
        self.id = None;
    }
}

impl Drop for PendingRequestGuard {
    fn drop(&mut self) {
        let Some(id) = self.id.take() else {
            return;
        };
        if let Ok(mut pending) = self.pending.try_lock() {
            pending.remove(&id);
            return;
        }
        let pending = Arc::clone(&self.pending);
        tokio::spawn(async move {
            pending.lock().await.remove(&id);
        });
    }
}

#[derive(Debug)]
struct InsecureServerCertVerifier {
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for InsecureServerCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn insecure_tls_config() -> Result<rustls::ClientConfig, AppError> {
    let provider = rustls::crypto::CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::ring::default_provider()));
    let verifier_provider = Arc::clone(&provider);
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| AppError::Internal(format!("OpenClaw TLS config error: {e}")))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(InsecureServerCertVerifier {
            provider: verifier_provider,
        }))
        .with_no_client_auth();
    // WebSocket upgrade is HTTP/1.1-only.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

pub struct AuthConfig {
    pub token: Option<String>,
    pub device_token: Option<String>,
    pub password: Option<String>,
}

pub struct OpenClawConnection {
    ws_sink: Mutex<Option<WsSink>>,
    pending: Arc<Mutex<HashMap<String, PendingSender>>>,
    event_tx: broadcast::Sender<EventFrame>,
    close_tx: broadcast::Sender<()>,
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
        Self::connect_with_options(url, auth, identity, false).await
    }

    /// Connect to an OpenClaw Gateway, optionally accepting an untrusted TLS
    /// certificate. `allow_insecure` only affects `wss://` certificate
    /// verification; the gateway protocol/authentication is always required.
    pub async fn connect_with_options(
        url: &str,
        auth: Option<AuthConfig>,
        identity: &DeviceIdentity,
        allow_insecure: bool,
    ) -> Result<(Arc<Self>, HelloOk), AppError> {
        let connector = if allow_insecure
            && url
                .get(..6)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("wss://"))
        {
            Some(Connector::Rustls(Arc::new(insecure_tls_config()?)))
        } else {
            None
        };
        let (ws_stream, _) = connect_async_tls_with_config(url, None, false, connector)
            .await
            .map_err(|e| AppError::BadGateway(format!("OpenClaw WebSocket connection failed: {e}")))?;

        let (sink, stream) = ws_stream.split();
        let (event_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        let (close_tx, _) = broadcast::channel(1);
        let (challenge_tx, challenge_rx) = oneshot::channel();
        let now = nomifun_common::now_ms();

        let conn = Arc::new(Self {
            ws_sink: Mutex::new(Some(sink)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            event_tx,
            close_tx,
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
        let mut connect_guard = ConnectAttemptGuard::new(Arc::clone(&conn));

        let nonce = match tokio::time::timeout(CHALLENGE_TIMEOUT, challenge_rx).await {
            Ok(Ok(Some(nonce))) if !nonce.trim().is_empty() => nonce,
            Ok(Ok(_)) => {
                conn.close().await;
                return Err(AppError::BadGateway(
                    "OpenClaw connect.challenge did not include a nonce".into(),
                ));
            }
            Ok(Err(_)) => {
                conn.close().await;
                return Err(AppError::BadGateway(
                    "OpenClaw connection closed before connect.challenge".into(),
                ));
            }
            Err(_) => {
                conn.close().await;
                return Err(AppError::Timeout(
                    "Timed out waiting for OpenClaw connect.challenge".into(),
                ));
            }
        };

        let hello = match conn.send_connect(Some(&nonce), auth, identity).await {
            Ok(hello) => hello,
            Err(err) => {
                conn.close().await;
                return Err(err);
            }
        };
        if !(OPENCLAW_MIN_PROTOCOL_VERSION..=OPENCLAW_MAX_PROTOCOL_VERSION).contains(&hello.protocol) {
            conn.close().await;
            return Err(AppError::BadGateway(format!(
                "OpenClaw negotiated unsupported protocol version {:?} (supported {}..={})",
                hello.protocol, OPENCLAW_MIN_PROTOCOL_VERSION, OPENCLAW_MAX_PROTOCOL_VERSION
            )));
        }
        if hello.type_.as_deref() != Some("hello-ok") {
            conn.close().await;
            return Err(AppError::BadGateway(
                "OpenClaw connect response was not a hello-ok payload".into(),
            ));
        }
        if !hello.auth.scopes.iter().any(|scope| scope == "operator.admin") {
            conn.close().await;
            return Err(AppError::Forbidden(
                "OpenClaw connection did not grant the required operator.admin scope".into(),
            ));
        }
        conn.connected.store(true, Ordering::Relaxed);

        conn.tick_interval_ms
            .store(hello.policy.tick_interval_ms, Ordering::Relaxed);

        conn.start_tick_watchdog();

        debug!(
            protocol = hello.protocol,
            server_version = %hello.server.version,
            "OpenClaw handshake complete"
        );

        connect_guard.disarm();
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
                let gap_ms = nomifun_common::now_ms()
                    .saturating_sub(last)
                    .max(0) as u64;
                if gap_ms > interval_ms.saturating_mul(2) {
                    warn!(
                        gap_ms = gap_ms,
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
        let normalized_auth = auth.map(|auth| AuthConfig {
            token: normalize_credential(auth.token),
            device_token: normalize_credential(auth.device_token),
            password: normalize_credential(auth.password),
        });
        let auth_params = match &normalized_auth {
            Some(a) if a.token.is_some() || a.device_token.is_some() || a.password.is_some() => Some(AuthParams {
                token: a.token.clone().or_else(|| a.device_token.clone()),
                device_token: a.device_token.clone(),
                password: a.password.clone(),
            }),
            _ => None,
        };

        // The signed proof binds the configured shared credential. A device
        // token is a separate pairing credential and must not replace the
        // shared token in the v3 proof when both are present.
        let device_params = build_device_auth_params(
            identity,
            nonce,
            device_proof_token(normalized_auth.as_ref()),
            std::env::consts::OS,
            None,
        );

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
        let mut pending_guard = PendingRequestGuard::new(Arc::clone(&self.pending), id.clone());

        let frame = RequestFrame {
            type_: "req",
            id: id.clone(),
            method: method.into(),
            params: Some(params),
        };
        if let Err(error) = self.ws_send_frame(&frame).await {
            self.pending.lock().await.remove(&id);
            pending_guard.disarm();
            return Err(error);
        }

        let result = match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(result) => {
                pending_guard.disarm();
                result.map_err(|_| AppError::Internal(format!("OpenClaw request '{method}' cancelled")))??
            }
            Err(_) => {
                self.pending.lock().await.remove(&id);
                pending_guard.disarm();
                return Err(AppError::Timeout(format!("OpenClaw request '{method}' timed out")));
            }
        };

        serde_json::from_value(result)
            .map_err(|e| AppError::Internal(format!("Failed to parse OpenClaw response for '{method}': {e}")))
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<EventFrame> {
        self.event_tx.subscribe()
    }

    pub fn subscribe_close(&self) -> broadcast::Receiver<()> {
        self.close_tx.subscribe()
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub async fn close(&self) {
        self.connected.store(false, Ordering::Relaxed);
        let _ = self.close_tx.send(());

        if let Some(mut sink) = self.ws_sink.lock().await.take() {
            let _ = sink.close().await;
        }
        if let Some(handle) = self._reader_handle.lock().await.take() {
            handle.abort();
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
        let _ = self.close_tx.send(());

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
                        let error = res.error.map(map_gateway_error).unwrap_or_else(|| {
                            AppError::Internal("Unknown OpenClaw error".into())
                        });
                        let _ = tx.send(Err(error));
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

fn normalize_credential(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

fn device_proof_token(auth: Option<&AuthConfig>) -> Option<&str> {
    auth.and_then(|auth| auth.token.as_deref())
}

fn map_gateway_error(error: super::protocol::ErrorShape) -> AppError {
    let details_text = error
        .details
        .as_ref()
        .and_then(|details| serde_json::to_string(details).ok())
        .map(|details| format!("; details={details}"))
        .unwrap_or_default();
    let message = format!("{}: {}{}", error.code, error.message, details_text);
    let code = error.code.to_ascii_uppercase();
    let details_code = error
        .details
        .as_ref()
        .and_then(|details| details.get("code"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_uppercase();
    if code.contains("AUTH")
        || code.contains("UNAUTHORIZED")
        || code.contains("NOT_PAIRED")
        || code.contains("PAIRING")
        || details_code.contains("AUTH")
        || details_code.contains("PAIRING")
    {
        return AppError::Unauthorized(message);
    }
    if code.contains("RATE") {
        return AppError::RateLimited;
    }
    if error.retryable == Some(true) {
        return AppError::BadGateway(message);
    }
    AppError::BadGateway(message)
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
                    "payload": { "nonce": nonce.unwrap_or_else(|| "test-nonce".into()) }
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
                                "type": "hello-ok",
                                "protocol": 4,
                                "server": { "version": "1.0.0", "connId": "test-conn" },
                                "auth": { "role": "operator", "scopes": ["operator.admin"] },
                                "policy": {
                                    "maxPayload": 26214400,
                                    "tickIntervalMs": 30000
                                },
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

    #[test]
    fn auth_credentials_are_trimmed_and_empty_values_dropped() {
        assert_eq!(
            normalize_credential(Some(" token ".into())).as_deref(),
            Some("token")
        );
        assert_eq!(normalize_credential(Some("   ".into())), None);
        assert_eq!(normalize_credential(None), None);
    }

    #[test]
    fn shared_token_is_preferred_for_device_proof() {
        let auth = AuthConfig {
            token: Some("shared-token".into()),
            device_token: Some("device-token".into()),
            password: None,
        };

        assert_eq!(device_proof_token(Some(&auth)), Some("shared-token"));
    }

    #[test]
    fn device_only_auth_does_not_sign_device_token_as_shared_token() {
        let auth = AuthConfig {
            token: None,
            device_token: Some("device-token".into()),
            password: None,
        };

        assert_eq!(device_proof_token(Some(&auth)), None);
    }

    #[tokio::test]
    async fn dropping_connect_future_closes_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("ws://{addr}");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (_sink, mut stream) = ws.split();
            tokio::time::timeout(Duration::from_secs(1), stream.next())
                .await
                .unwrap()
        });

        let identity = generate_identity();
        let mut connect = Box::pin(OpenClawConnection::connect(&url, None, &identity));
        tokio::select! {
            _ = connect.as_mut() => panic!("connect unexpectedly completed"),
            _ = tokio::time::sleep(Duration::from_millis(25)) => {}
        }
        drop(connect);

        let close = server.await.unwrap();
        assert!(matches!(close, Some(Ok(Message::Close(_))) | None));
    }

    #[tokio::test]
    async fn dropping_request_future_removes_pending_entry() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("ws://{addr}");
        let _server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (mut sink, mut stream) = ws.split();
            let challenge = json!({
                "type": "event",
                "event": "connect.challenge",
                "payload": { "nonce": "test-nonce" }
            });
            sink.send(Message::Text(
                serde_json::to_string(&challenge).unwrap().into(),
            ))
            .await
            .unwrap();

            if let Some(Ok(Message::Text(text))) = stream.next().await {
                let frame: Value = serde_json::from_str(&text).unwrap();
                let res = json!({
                    "type": "res",
                    "id": frame["id"],
                    "ok": true,
                    "payload": {
                        "type": "hello-ok",
                        "protocol": 4,
                        "server": { "version": "1.0.0", "connId": "test-conn" },
                        "auth": { "role": "operator", "scopes": ["operator.admin"] },
                        "policy": { "maxPayload": 26214400, "tickIntervalMs": 30000 }
                    }
                });
                sink.send(Message::Text(serde_json::to_string(&res).unwrap().into()))
                    .await
                    .unwrap();
            }

            let _ = stream.next().await;
            tokio::time::sleep(Duration::from_secs(1)).await;
        });
        let conn = OpenClawConnection::connect(&url, None, &generate_identity())
            .await
            .unwrap()
            .0;

        let request = conn.request::<Value>("never.responds", json!({}));
        tokio::pin!(request);
        tokio::select! {
            result = request.as_mut() => panic!("request unexpectedly completed: {result:?}"),
            _ = tokio::time::sleep(Duration::from_millis(25)) => {}
        }
        drop(request);

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if conn.pending.lock().await.is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        conn.close().await;
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
    async fn connect_with_challenge_nonce() {
        let (url, _server) = spawn_mock_gateway(Some("test-nonce")).await;
        let conn = OpenClawConnection::connect(&url, None, &generate_identity())
            .await
            .unwrap()
            .0;
        assert!(conn.is_connected());
        conn.close().await;
    }

    #[tokio::test]
    async fn request_response_correlation() {
        let (url, _server) = spawn_mock_gateway(Some("test-nonce")).await;
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
                    "payload": { "nonce": "test-nonce" }
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
                        "payload": {
                            "type": "hello-ok",
                            "protocol": 4,
                            "server": { "version": "1.0.0", "connId": "test-conn" },
                            "auth": { "role": "operator", "scopes": ["operator.admin"] },
                            "policy": { "maxPayload": 26214400, "tickIntervalMs": 30000 }
                        }
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
