use std::sync::Arc;
use std::time::Duration;

use axum::extract::WebSocketUpgrade;
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use nomifun_api_types::WebSocketMessage;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use crate::manager::{TokenAuthenticator, WebSocketManager};
use crate::router::MessageRouter;
use crate::types::{ConnectionId, PER_CONNECTION_BUFFER, WebSocketCloseCode, WsOutbound};

/// Extracts a JWT token from WebSocket upgrade request headers.
///
/// Injected by `nomifun-app` — wraps `nomifun_auth::extract_token_from_ws_headers`
/// so that `nomifun-realtime` does not depend on `nomifun-auth` directly.
pub type TokenExtractor = Arc<dyn Fn(&HeaderMap) -> Option<String> + Send + Sync>;

/// Shared state required by the WebSocket upgrade handler.
#[derive(Clone)]
pub struct WsHandlerState {
    pub manager: Arc<WebSocketManager>,
    pub router: Arc<dyn MessageRouter>,
    pub token_authenticator: TokenAuthenticator,
    pub token_extractor: TokenExtractor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OriginDisposition {
    /// Browsers send `Origin` on WebSocket handshakes. Its absence is retained
    /// for CLI, agent, and other non-browser clients that authenticate with an
    /// explicit credential.
    NonBrowser,
    SameOrigin,
    /// The desktop renderer and local Vite renderer run on a different local
    /// origin from the embedded loopback backend.
    LocalWebview,
}

struct UpgradeCredentials {
    token: Option<String>,
    selected_protocol: Option<String>,
}

/// Axum handler for HTTP → WebSocket upgrade.
///
/// Validates an origin-bearing browser handshake before extracting its JWT,
/// then upgrades the connection to WebSocket on success.
/// On authentication failure, sends `auth-expired` and closes with 1008.
///
/// When the token is carried via `Sec-WebSocket-Protocol`, the server
/// echoes the protocol header back so the client handshake succeeds.
pub async fn ws_upgrade_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    axum::extract::State(state): axum::extract::State<WsHandlerState>,
) -> Response {
    // Validate Origin before touching cookie/Bearer credentials. In
    // particular, a hostile page must not be able to use an ambient session
    // cookie for a cross-site WebSocket handshake.
    let credentials = match upgrade_credentials(&headers, &state) {
        Ok(credentials) => credentials,
        Err(()) => {
            debug!("rejected websocket upgrade with an untrusted origin");
            return StatusCode::FORBIDDEN.into_response();
        }
    };

    // Echo the selected Sec-WebSocket-Protocol so clients using it for auth
    // receive a valid subprotocol negotiation response. Select one protocol,
    // rather than echoing a possibly comma-delimited request header.
    let ws = if let Some(protocol) = credentials.selected_protocol.clone() {
        ws.protocols([protocol])
    } else {
        ws
    };

    ws.on_upgrade(move |socket| async move {
        handle_socket(socket, credentials.token, state).await;
    })
    .into_response()
}

/// Resolve the authentication material permitted for this request's origin.
///
/// Same-origin browsers retain the existing extractor priority (Bearer,
/// cookie, then subprotocol). A known local desktop/dev webview is necessarily
/// cross-origin with the loopback backend, so it may connect only when the
/// first requested subprotocol is itself a valid explicit credential. This
/// prevents an ambient cookie from authorizing a local cross-origin page.
fn upgrade_credentials(headers: &HeaderMap, state: &WsHandlerState) -> Result<UpgradeCredentials, ()> {
    match validate_origin(headers)? {
        OriginDisposition::LocalWebview => {
            let token = if let Some(protocol) = first_requested_protocol(headers) {
                let protocol = protocol.to_owned();
                (state.token_authenticator)(&protocol).ok_or(())?;
                protocol
            } else {
                // `--insecure-no-auth` / `dev:web` deliberately has no
                // browser-visible credential and its application-provided
                // extractor returns the synthetic local token. Preserve that
                // legal local development handshake, but remove every ambient
                // credential first so a Cookie or Bearer value can never make
                // an authenticated deployment take this fallback.
                let mut credentialless_headers = headers.clone();
                credentialless_headers.remove(header::AUTHORIZATION);
                credentialless_headers.remove(header::COOKIE);
                credentialless_headers.remove(header::SEC_WEBSOCKET_PROTOCOL);
                let token = (state.token_extractor)(&credentialless_headers).ok_or(())?;
                (state.token_authenticator)(&token).ok_or(())?;
                token
            };
            Ok(UpgradeCredentials {
                token: Some(token),
                selected_protocol: first_requested_protocol(headers).map(str::to_owned),
            })
        }
        OriginDisposition::NonBrowser | OriginDisposition::SameOrigin => Ok(UpgradeCredentials {
            token: (state.token_extractor)(headers),
            selected_protocol: first_requested_protocol(headers).map(str::to_owned),
        }),
    }
}

/// Validate a browser WebSocket Origin against the request Host.
///
/// This follows the application's existing browser-viewer origin model:
/// ordinary WebUI browsers must be same-authority HTTP(S), while the known
/// Tauri and loopback development origins are classified separately and bound
/// to explicit subprotocol authentication by [`upgrade_credentials`]. Missing
/// Origin is allowed only as the non-browser compatibility path. Any present
/// but malformed, opaque, duplicated, or untrusted Origin fails closed.
fn validate_origin(headers: &HeaderMap) -> Result<OriginDisposition, ()> {
    let mut origins = headers.get_all(header::ORIGIN).iter();
    let Some(origin_value) = origins.next() else {
        return Ok(OriginDisposition::NonBrowser);
    };
    if origins.next().is_some() {
        return Err(());
    }
    let origin = origin_value
        .to_str()
        .ok()
        .map(str::trim)
        .filter(|origin| !origin.is_empty() && !origin.contains(','))
        .ok_or(())?;

    let mut hosts = headers.get_all(header::HOST).iter();
    let host = hosts
        .next()
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|host| !host.is_empty() && !host.contains(',') && !host.contains('@'))
        .ok_or(())?;
    if hosts.next().is_some() {
        return Err(());
    }
    let host = host.parse::<axum::http::uri::Authority>().map_err(|_| ())?;

    let uri = origin.parse::<Uri>().map_err(|_| ())?;
    let scheme = uri.scheme_str().ok_or(())?;
    let authority = uri.authority().ok_or(())?;
    if uri
        .path_and_query()
        .is_some_and(|path_and_query| path_and_query.as_str() != "/")
        || authority.as_str().contains('@')
    {
        return Err(());
    }

    if matches!(scheme, "http" | "https") && authority.as_str().eq_ignore_ascii_case(host.as_str()) {
        return Ok(OriginDisposition::SameOrigin);
    }

    let origin_host = authority
        .host()
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or_else(|| authority.host());
    let is_tauri_origin = (scheme == "tauri" && origin_host.eq_ignore_ascii_case("localhost"))
        || (matches!(scheme, "http" | "https") && origin_host.eq_ignore_ascii_case("tauri.localhost"));
    let is_loopback_dev_origin = matches!(scheme, "http" | "https")
        && (origin_host.eq_ignore_ascii_case("localhost") || origin_host == "127.0.0.1" || origin_host == "::1");

    if is_tauri_origin || is_loopback_dev_origin {
        Ok(OriginDisposition::LocalWebview)
    } else {
        Err(())
    }
}

fn first_requested_protocol(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok())
        .and_then(|protocols| protocols.split(',').next())
        .map(str::trim)
        .filter(|protocol| !protocol.is_empty())
}

/// Post-upgrade connection handler.
///
/// Validates the token, registers the client, spawns send/recv loops.
async fn handle_socket(socket: WebSocket, token: Option<String>, state: WsHandlerState) {
    let Some(token) = token else {
        send_close_no_token(socket).await;
        return;
    };

    let Some(user_id) = (state.token_authenticator)(&token) else {
        send_auth_expired_and_close(socket).await;
        return;
    };

    let (tx, rx) = mpsc::channel::<WsOutbound>(PER_CONNECTION_BUFFER);
    let conn_id = state.manager.add_client(user_id, token, tx);
    let cancellation = state
        .manager
        .connection_cancellation(conn_id)
        .expect("newly registered websocket has a cancellation signal");

    info!(%conn_id, "websocket connection established");

    let (ws_sender, ws_receiver) = socket.split();

    let mut send_handle = tokio::spawn(send_loop(
        conn_id,
        rx,
        ws_sender,
        cancellation.clone(),
    ));
    let mut send_loop_finished = false;
    let server_close = tokio::select! {
        _ = recv_loop(conn_id, ws_receiver, &state) => false,
        _ = cancellation.cancelled() => true,
        _ = &mut send_handle => {
            // A heartbeat policy close is queued before its manager entry is
            // removed. Once that queue drains, the send loop ending must also
            // terminate an otherwise-idle receive loop.
            send_loop_finished = true;
            false
        },
    };

    if server_close {
        // Let the send loop put a real close frame on the wire. A bounded wait
        // avoids a stalled socket sink retaining the task indefinitely.
        let _ = tokio::time::timeout(Duration::from_secs(1), &mut send_handle).await;
    }
    if !send_loop_finished {
        send_handle.abort();
    }
    state.manager.remove_client(conn_id);
    info!(%conn_id, "websocket connection closed");
}

/// Send a close frame with 1008 when no token is provided.
async fn send_close_no_token(mut socket: WebSocket) {
    let close = Message::Close(Some(CloseFrame {
        code: WebSocketCloseCode::PolicyViolation.as_u16(),
        reason: "no token provided".into(),
    }));
    let _ = socket.send(close).await;
}

/// Send `auth-expired` event then close with 1008.
async fn send_auth_expired_and_close(mut socket: WebSocket) {
    let auth_expired = WebSocketMessage::new("auth-expired", json!({"message": "Token expired or invalid"}));
    if let Ok(text) = serde_json::to_string(&auth_expired) {
        let _ = socket.send(Message::Text(text.into())).await;
    }
    let close = Message::Close(Some(CloseFrame {
        code: WebSocketCloseCode::PolicyViolation.as_u16(),
        reason: "authentication failed".into(),
    }));
    let _ = socket.send(close).await;
}

// -------------------------------------------------------------------
// Send loop
// -------------------------------------------------------------------

/// Reads `WsOutbound` from the per-connection channel and forwards
/// them to the WebSocket sink.
async fn send_loop(
    conn_id: ConnectionId,
    mut rx: mpsc::Receiver<WsOutbound>,
    mut sender: futures_util::stream::SplitSink<WebSocket, Message>,
    cancellation: CancellationToken,
) {
    loop {
        let outbound = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                let close = Message::Close(Some(CloseFrame {
                    code: WebSocketCloseCode::NormalClosure.as_u16(),
                    reason: "server resync requested".into(),
                }));
                let _ = sender.send(close).await;
                break;
            }
            outbound = rx.recv() => match outbound {
                Some(outbound) => outbound,
                None => break,
            },
        };
        let msg = match outbound {
            WsOutbound::Text(text) => Message::Text(text.into()),
            WsOutbound::Close(code, reason) => Message::Close(Some(CloseFrame {
                code: code.as_u16(),
                reason: reason.into(),
            })),
        };
        if sender.send(msg).await.is_err() {
            debug!(%conn_id, "send loop: socket write failed, exiting");
            break;
        }
    }
}

// -------------------------------------------------------------------
// Receive loop
// -------------------------------------------------------------------

/// Reads messages from the WebSocket stream, parses JSON, routes.
async fn recv_loop(
    conn_id: ConnectionId,
    mut receiver: futures_util::stream::SplitStream<WebSocket>,
    state: &WsHandlerState,
) {
    while let Some(result) = receiver.next().await {
        let msg = match result {
            Ok(m) => m,
            Err(e) => {
                debug!(%conn_id, error = %e, "recv error, closing");
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                handle_text_message(conn_id, &text, state);
            }
            Message::Close(_) => {
                debug!(%conn_id, "received close frame");
                break;
            }
            // Ping/Pong at the WebSocket protocol level are handled
            // automatically by axum/tungstenite. Binary frames are ignored.
            _ => {}
        }
    }
}

/// Process a text message: parse JSON, dispatch to built-in or router.
fn handle_text_message(conn_id: ConnectionId, text: &str, state: &WsHandlerState) {
    let parsed: Result<WebSocketMessage<Value>, _> = serde_json::from_str(text);

    let msg = match parsed {
        Ok(m) => m,
        Err(_) => {
            send_error_response(state, conn_id);
            return;
        }
    };

    match msg.name.as_str() {
        "pong" => {
            state.manager.update_last_ping(conn_id);
        }
        "subscribe-show-open" => {
            handle_subscribe_show_open(state, conn_id, msg.data);
        }
        name => {
            state.router.route(conn_id, name, msg.data);
        }
    }
}

/// Send an error response for invalid message format.
fn send_error_response(state: &WsHandlerState, conn_id: ConnectionId) {
    let error = json!({
        "error": "Invalid message format",
        "expected": r#"{ "name": "event-name", "data": {...} }"#
    });

    if let Ok(text) = serde_json::to_string(&error) {
        state.manager.send_raw_to(conn_id, WsOutbound::Text(text));
    }
}

/// Handle `subscribe-show-open`: reply with `show-open-request`.
///
/// The inbound `data` is the @office-ai/platform bridge envelope
/// `{ id, data: <user-params> }`. The renderer awaits a callback whose event
/// name embeds `id` (`subscribe.callback-show-open<id>`), so we must echo it
/// back; without it, the frontend's `useDirectorySelection` hook builds the
/// wrong callback name and the original `invoke()` Promise never resolves.
///
/// `isFileMode` is `true` when `properties` contains `openFile`
/// but NOT `openDirectory`.
fn handle_subscribe_show_open(state: &WsHandlerState, conn_id: ConnectionId, data: Value) {
    let id = data.get("id").and_then(|v| v.as_str()).unwrap_or("").to_owned();
    let inner = data.get("data").unwrap_or(&Value::Null);

    let properties = inner
        .get("properties")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let has_open_file = properties.iter().any(|v| v.as_str() == Some("openFile"));
    let has_open_directory = properties.iter().any(|v| v.as_str() == Some("openDirectory"));

    let is_file_mode = has_open_file && !has_open_directory;

    let response = WebSocketMessage::new(
        "show-open-request",
        json!({
            "id": id,
            "properties": properties,
            "isFileMode": is_file_mode,
        }),
    );

    state.manager.send_to(conn_id, response);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state(manager: Arc<WebSocketManager>) -> WsHandlerState {
        WsHandlerState {
            manager,
            router: Arc::new(crate::router::NoopMessageRouter),
            token_authenticator: Arc::new(|_| Some("user".to_owned())),
            token_extractor: Arc::new(|_| None),
        }
    }

    fn origin_headers(host: &str, origin: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, host.parse().unwrap());
        if let Some(origin) = origin {
            headers.insert(header::ORIGIN, origin.parse().unwrap());
        }
        headers
    }

    #[test]
    fn origin_validation_accepts_same_authority_and_non_browser_absence() {
        assert_eq!(
            validate_origin(&origin_headers("nomifun.example:8443", None)),
            Ok(OriginDisposition::NonBrowser)
        );
        assert_eq!(
            validate_origin(&origin_headers(
                "nomifun.example:8443",
                Some("https://NOMIFUN.example:8443")
            )),
            Ok(OriginDisposition::SameOrigin)
        );
        assert!(
            validate_origin(&origin_headers(
                "nomifun.example:8443",
                Some("https://nomifun.example")
            ))
            .is_err(),
            "the origin port is part of the WebSocket same-origin boundary"
        );
    }

    #[test]
    fn origin_validation_classifies_known_local_webview_origins() {
        for origin in [
            "tauri://localhost",
            "http://tauri.localhost",
            "http://localhost:5173",
            "http://127.0.0.1:5173",
            "http://[::1]:5173",
        ] {
            assert_eq!(
                validate_origin(&origin_headers("127.0.0.1:25808", Some(origin))),
                Ok(OriginDisposition::LocalWebview),
                "expected {origin} to be a known local webview origin"
            );
        }
    }

    #[test]
    fn origin_validation_fails_closed_for_untrusted_or_ambiguous_values() {
        for origin in [
            "https://attacker.example",
            "null",
            "https://nomifun.example:8443/path",
            "https://user@nomifun.example:8443",
            "https://nomifun.example:8443, https://attacker.example",
        ] {
            assert!(
                validate_origin(&origin_headers("nomifun.example:8443", Some(origin))).is_err(),
                "expected {origin} to be rejected"
            );
        }

        let mut duplicate = origin_headers("nomifun.example:8443", None);
        duplicate.append(header::ORIGIN, "https://nomifun.example:8443".parse().unwrap());
        duplicate.append(header::ORIGIN, "https://attacker.example".parse().unwrap());
        assert!(validate_origin(&duplicate).is_err());

        let mut missing_host = HeaderMap::new();
        missing_host.insert(header::ORIGIN, "https://nomifun.example:8443".parse().unwrap());
        assert!(validate_origin(&missing_host).is_err());
    }

    #[test]
    fn subscribe_show_open_file_mode() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        let state = test_state(manager);

        let data = json!({"id": "abc123", "data": {"properties": ["openFile"]}});
        handle_subscribe_show_open(&state, conn_id, data);

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["name"], "show-open-request");
                assert_eq!(parsed["data"]["id"], "abc123");
                assert_eq!(parsed["data"]["isFileMode"], true);
                assert_eq!(parsed["data"]["properties"], json!(["openFile"]));
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn subscribe_show_open_directory_mode() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        let state = test_state(manager);

        let data = json!({"id": "dir1", "data": {"properties": ["openDirectory"]}});
        handle_subscribe_show_open(&state, conn_id, data);

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["data"]["id"], "dir1");
                assert_eq!(parsed["data"]["isFileMode"], false);
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn subscribe_show_open_mixed_mode() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        let state = test_state(manager);

        let data = json!({"id": "mixed", "data": {"properties": ["openFile", "openDirectory"]}});
        handle_subscribe_show_open(&state, conn_id, data);

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["data"]["id"], "mixed");
                assert_eq!(parsed["data"]["isFileMode"], false);
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn subscribe_show_open_empty_properties() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        let state = test_state(manager);

        let data = json!({"id": "empty", "data": {"properties": []}});
        handle_subscribe_show_open(&state, conn_id, data);

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["data"]["id"], "empty");
                assert_eq!(parsed["data"]["isFileMode"], false);
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn subscribe_show_open_missing_properties() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        let state = test_state(manager);

        handle_subscribe_show_open(&state, conn_id, json!({"id": "noprops", "data": {}}));

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["data"]["id"], "noprops");
                assert_eq!(parsed["data"]["isFileMode"], false);
                assert_eq!(parsed["data"]["properties"], json!([]));
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn subscribe_show_open_missing_id_falls_back_to_empty_string() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        let state = test_state(manager);

        handle_subscribe_show_open(&state, conn_id, json!({}));

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["data"]["id"], "");
                assert_eq!(parsed["data"]["isFileMode"], false);
                assert_eq!(parsed["data"]["properties"], json!([]));
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn text_message_pong_updates_last_ping() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, _rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        let state = test_state(manager);

        std::thread::sleep(std::time::Duration::from_millis(5));

        handle_text_message(conn_id, r#"{"name":"pong","data":{}}"#, &state);
        // No panic = success (update_last_ping was called)
    }

    #[test]
    fn text_message_invalid_json_sends_error() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        let state = test_state(manager);

        handle_text_message(conn_id, "not json", &state);

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["error"], "Invalid message format");
                assert!(parsed["expected"].is_string());
            }
            _ => panic!("expected error text"),
        }
    }

    #[test]
    fn text_message_missing_fields_sends_error() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, mut rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        let state = test_state(manager);

        handle_text_message(conn_id, r#"{"foo":"bar"}"#, &state);

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["error"], "Invalid message format");
            }
            _ => panic!("expected error text"),
        }
    }

    #[test]
    fn text_message_routes_unknown_to_router() {
        use std::sync::atomic::{AtomicBool, Ordering};

        struct TestRouter {
            called: AtomicBool,
        }
        impl MessageRouter for TestRouter {
            fn route(&self, _conn_id: ConnectionId, _name: &str, _data: Value) {
                self.called.store(true, Ordering::Relaxed);
            }
        }

        let manager = Arc::new(WebSocketManager::new());
        let (tx, _rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);

        let router = Arc::new(TestRouter {
            called: AtomicBool::new(false),
        });
        let state = WsHandlerState {
            manager,
            router: router.clone(),
            token_authenticator: Arc::new(|_| Some("user".to_owned())),
            token_extractor: Arc::new(|_| None),
        };

        handle_text_message(
            conn_id,
            r#"{"name":"conversation.send-message","data":{"text":"hi"}}"#,
            &state,
        );

        assert!(router.called.load(Ordering::Relaxed));
    }

    #[test]
    fn error_response_to_disconnected_client_is_noop() {
        let manager = Arc::new(WebSocketManager::new());
        let (tx, rx) = mpsc::channel(PER_CONNECTION_BUFFER);
        let conn_id = manager.add_client("user".into(), "tok".into(), tx);
        drop(rx); // close channel

        let state = test_state(manager.clone());

        // Should not panic — client will be removed
        send_error_response(&state, conn_id);
        assert_eq!(manager.client_count(), 0);
    }
}
