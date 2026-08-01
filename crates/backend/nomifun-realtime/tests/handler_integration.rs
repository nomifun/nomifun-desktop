use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::http::{HeaderValue, header};
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use nomifun_api_types::WebSocketMessage;
use nomifun_realtime::{ConnectionId, WebSocketManager, WsHandlerState, ws_upgrade_handler};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Start an axum server with the WebSocket handler and return its address.
async fn start_server(state: WsHandlerState) -> SocketAddr {
    let app = Router::new().route("/ws", get(ws_upgrade_handler)).with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    addr
}

fn default_state() -> (WsHandlerState, Arc<WebSocketManager>) {
    let manager = Arc::new(WebSocketManager::new());
    let state = WsHandlerState {
        manager: manager.clone(),
        token_authenticator: Arc::new(|t| (t == "valid-token").then(|| "user".to_owned())),
        token_extractor: Arc::new(|headers| {
            headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.strip_prefix("Bearer "))
                .map(|s| s.to_owned())
        }),
    };
    (state, manager)
}

fn all_auth_sources_state() -> (WsHandlerState, Arc<WebSocketManager>) {
    let manager = Arc::new(WebSocketManager::new());
    let state = WsHandlerState {
        manager: manager.clone(),
        token_authenticator: Arc::new(|token| {
            matches!(token, "valid-token" | "local-trust-token").then(|| "user".to_owned())
        }),
        token_extractor: Arc::new(|headers| {
            headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                .map(str::to_owned)
                .or_else(|| {
                    headers
                        .get(header::COOKIE)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|cookies| {
                            cookies.split(';').find_map(|cookie| {
                                let (name, value) = cookie.trim().split_once('=')?;
                                (name == "nomifun-session" && !value.is_empty()).then(|| value.to_owned())
                            })
                        })
                })
                .or_else(|| {
                    headers
                        .get(header::SEC_WEBSOCKET_PROTOCOL)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|protocols| protocols.split(',').next())
                        .map(str::trim)
                        .filter(|protocol| !protocol.is_empty())
                        .map(str::to_owned)
                })
        }),
    };
    (state, manager)
}

fn no_auth_state() -> (WsHandlerState, Arc<WebSocketManager>) {
    let manager = Arc::new(WebSocketManager::new());
    let state = WsHandlerState {
        manager: manager.clone(),
        token_authenticator: Arc::new(|_| Some("user".to_owned())),
        token_extractor: Arc::new(|_| Some("local".to_owned())),
    };
    (state, manager)
}

fn upgrade_request(addr: SocketAddr) -> tungstenite::http::Request<()> {
    tungstenite::http::Request::builder()
        .uri(format!("ws://{addr}/ws"))
        .header(header::HOST.as_str(), addr.to_string())
        .header(header::CONNECTION.as_str(), "Upgrade")
        .header(header::UPGRADE.as_str(), "websocket")
        .header(header::SEC_WEBSOCKET_VERSION.as_str(), "13")
        .header(
            header::SEC_WEBSOCKET_KEY.as_str(),
            tungstenite::handshake::client::generate_key(),
        )
        .body(())
        .unwrap()
}

async fn rejected_status(request: tungstenite::http::Request<()>) -> u16 {
    match tokio_tungstenite::connect_async(request).await {
        Err(tungstenite::Error::Http(response)) => response.status().as_u16(),
        Ok((mut socket, _)) => {
            let _ = socket.close(None).await;
            panic!("websocket handshake unexpectedly succeeded")
        }
        Err(error) => panic!("expected an HTTP handshake rejection, got {error}"),
    }
}

/// Connect with an Authorization header.
async fn connect_with_token(
    addr: SocketAddr,
    token: &str,
) -> (
    futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
        tungstenite::Message,
    >,
    futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    >,
) {
    let url = format!("ws://{addr}/ws");
    let request = tungstenite::http::Request::builder()
        .uri(&url)
        .header("Host", addr.to_string())
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
        .header("Authorization", format!("Bearer {token}"))
        .body(())
        .unwrap();

    let (ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    ws.split()
}

/// Connect without any auth header.
async fn connect_no_token(
    addr: SocketAddr,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let url = format!("ws://{addr}/ws");
    let (ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    ws
}

/// Read the next text message within a timeout.
async fn read_text<S>(stream: &mut S) -> Value
where
    S: StreamExt<Item = Result<tungstenite::Message, tungstenite::Error>> + Unpin,
{
    let timeout = Duration::from_secs(5);
    tokio::time::timeout(timeout, async {
        loop {
            match stream.next().await {
                Some(Ok(tungstenite::Message::Text(t))) => {
                    return serde_json::from_str::<Value>(&t).unwrap();
                }
                Some(Ok(tungstenite::Message::Close(_))) => {
                    panic!("unexpected close frame");
                }
                Some(Err(e)) => {
                    panic!("read error: {e}");
                }
                None => {
                    panic!("stream ended");
                }
                _ => continue, // skip ping/pong/binary
            }
        }
    })
    .await
    .expect("read timed out")
}

/// Read until a close frame is received, returning the close code.
async fn read_close<S>(stream: &mut S) -> Option<u16>
where
    S: StreamExt<Item = Result<tungstenite::Message, tungstenite::Error>> + Unpin,
{
    let timeout = Duration::from_secs(5);
    tokio::time::timeout(timeout, async {
        loop {
            match stream.next().await {
                Some(Ok(tungstenite::Message::Close(frame))) => {
                    return frame.map(|f| f.code.into());
                }
                Some(Ok(_)) => continue,
                Some(Err(_)) => return None,
                None => return None,
            }
        }
    })
    .await
    .expect("read_close timed out")
}

fn send_json(text: &str) -> tungstenite::Message {
    tungstenite::Message::Text(text.into())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_browser_bearer_without_origin_connects_successfully() {
    let (state, manager) = default_state();
    let addr = start_server(state).await;

    let (_tx, _rx) = connect_with_token(addr, "valid-token").await;

    // Allow connection to register
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(manager.client_count(), 1);
}

#[tokio::test]
async fn same_origin_browser_cookie_connects_successfully() {
    let (state, manager) = all_auth_sources_state();
    let addr = start_server(state).await;
    let mut request = upgrade_request(addr);
    request.headers_mut().insert(
        header::ORIGIN,
        HeaderValue::from_str(&format!("http://{addr}")).unwrap(),
    );
    request.headers_mut().insert(
        header::COOKIE,
        HeaderValue::from_static("nomifun-session=valid-token"),
    );

    let (_socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(response.status().as_u16(), 101);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(manager.client_count(), 1);
}

#[tokio::test]
async fn untrusted_browser_origin_is_rejected_before_cookie_or_bearer_authentication() {
    let manager = Arc::new(WebSocketManager::new());
    let extractor_calls = Arc::new(AtomicUsize::new(0));
    let authenticator_calls = Arc::new(AtomicUsize::new(0));
    let state = WsHandlerState {
        manager: manager.clone(),
        token_authenticator: {
            let calls = authenticator_calls.clone();
            Arc::new(move |_| {
                calls.fetch_add(1, Ordering::Relaxed);
                Some("user".to_owned())
            })
        },
        token_extractor: {
            let calls = extractor_calls.clone();
            Arc::new(move |_| {
                calls.fetch_add(1, Ordering::Relaxed);
                Some("valid-token".to_owned())
            })
        },
    };
    let addr = start_server(state).await;

    for (header_name, header_value) in [
        (header::COOKIE, "nomifun-session=valid-token"),
        (header::AUTHORIZATION, "Bearer valid-token"),
        (header::SEC_WEBSOCKET_PROTOCOL, "valid-token"),
    ] {
        let mut request = upgrade_request(addr);
        request.headers_mut().insert(
            header::ORIGIN,
            HeaderValue::from_static("https://attacker.example"),
        );
        request
            .headers_mut()
            .insert(header_name, HeaderValue::from_static(header_value));
        assert_eq!(rejected_status(request).await, 403);
    }

    assert_eq!(extractor_calls.load(Ordering::Relaxed), 0);
    assert_eq!(authenticator_calls.load(Ordering::Relaxed), 0);
    assert_eq!(manager.client_count(), 0);
}

#[tokio::test]
async fn local_desktop_origin_requires_and_accepts_protocol_bound_authentication() {
    let (state, manager) = all_auth_sources_state();
    let addr = start_server(state).await;

    let mut ambient_cookie_request = upgrade_request(addr);
    ambient_cookie_request.headers_mut().insert(
        header::ORIGIN,
        HeaderValue::from_static("http://tauri.localhost"),
    );
    ambient_cookie_request.headers_mut().insert(
        header::COOKIE,
        HeaderValue::from_static("nomifun-session=valid-token"),
    );
    assert_eq!(rejected_status(ambient_cookie_request).await, 403);

    let mut explicit_request = upgrade_request(addr);
    explicit_request.headers_mut().insert(
        header::ORIGIN,
        HeaderValue::from_static("http://tauri.localhost"),
    );
    explicit_request.headers_mut().insert(
        header::COOKIE,
        HeaderValue::from_static("nomifun-session=valid-token"),
    );
    explicit_request.headers_mut().insert(
        header::SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static("local-trust-token"),
    );
    let (_socket, response) = tokio_tungstenite::connect_async(explicit_request).await.unwrap();
    assert_eq!(response.status().as_u16(), 101);
    assert_eq!(
        response
            .headers()
            .get(header::SEC_WEBSOCKET_PROTOCOL)
            .and_then(|value| value.to_str().ok()),
        Some("local-trust-token")
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(manager.client_count(), 1);
}

#[tokio::test]
async fn local_vite_origin_without_credentials_remains_available_in_no_auth_mode() {
    let (state, manager) = no_auth_state();
    let addr = start_server(state).await;
    let mut request = upgrade_request(addr);
    request.headers_mut().insert(
        header::ORIGIN,
        HeaderValue::from_static("http://localhost:5173"),
    );

    let (_socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(response.status().as_u16(), 101);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(manager.client_count(), 1);
}

#[tokio::test]
async fn local_origin_does_not_fall_back_to_ambient_credentials_when_protocol_is_missing() {
    let (state, _manager) = all_auth_sources_state();
    let addr = start_server(state).await;
    let mut request = upgrade_request(addr);
    request.headers_mut().insert(
        header::ORIGIN,
        HeaderValue::from_static("http://localhost:5173"),
    );
    request.headers_mut().insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("Bearer valid-token"),
    );
    assert_eq!(rejected_status(request).await, 403);
}

#[tokio::test]
async fn no_token_closes_with_1008() {
    let (state, _) = default_state();
    let addr = start_server(state).await;

    let mut ws = connect_no_token(addr).await;

    let code = read_close(&mut ws).await;
    assert_eq!(code, Some(1008));
}

#[tokio::test]
async fn invalid_token_sends_auth_expired_then_closes() {
    let (state, _) = default_state();
    let addr = start_server(state).await;

    let (_, mut rx) = connect_with_token(addr, "bad-token").await;

    let msg = read_text(&mut rx).await;
    assert_eq!(msg["name"], "auth-expired");

    let code = read_close(&mut rx).await;
    assert_eq!(code, Some(1008));
}

#[tokio::test]
async fn invalid_json_message_returns_error() {
    let (state, _) = default_state();
    let addr = start_server(state).await;

    let (mut tx, mut rx) = connect_with_token(addr, "valid-token").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    tx.send(send_json("not valid json")).await.unwrap();

    let msg = read_text(&mut rx).await;
    assert_eq!(msg["error"], "Invalid message format");
    assert!(msg["expected"].is_string());
}

#[tokio::test]
async fn missing_fields_returns_error() {
    let (state, _) = default_state();
    let addr = start_server(state).await;

    let (mut tx, mut rx) = connect_with_token(addr, "valid-token").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    tx.send(send_json(r#"{"foo":"bar"}"#)).await.unwrap();

    let msg = read_text(&mut rx).await;
    assert_eq!(msg["error"], "Invalid message format");
}

#[tokio::test]
async fn subscribe_show_open_replies_with_show_open_request() {
    let (state, _) = default_state();
    let addr = start_server(state).await;

    let (mut tx, mut rx) = connect_with_token(addr, "valid-token").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Mirrors the @office-ai/platform bridge envelope shape produced by
    // `invoke('show-open', { properties: ['openFile'] })`.
    let payload = json!({
        "name": "subscribe-show-open",
        "data": {"id": "abc123", "data": {"properties": ["openFile"]}}
    });
    tx.send(send_json(&payload.to_string())).await.unwrap();

    let msg = read_text(&mut rx).await;
    assert_eq!(msg["name"], "show-open-request");
    assert_eq!(msg["data"]["id"], "abc123");
    assert_eq!(msg["data"]["isFileMode"], true);
    assert_eq!(msg["data"]["properties"], json!(["openFile"]));
}

#[tokio::test]
async fn subscribe_show_open_directory_mode() {
    let (state, _) = default_state();
    let addr = start_server(state).await;

    let (mut tx, mut rx) = connect_with_token(addr, "valid-token").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let payload = json!({
        "name": "subscribe-show-open",
        "data": {"id": "dir1", "data": {"properties": ["openFile", "openDirectory"]}}
    });
    tx.send(send_json(&payload.to_string())).await.unwrap();

    let msg = read_text(&mut rx).await;
    assert_eq!(msg["name"], "show-open-request");
    assert_eq!(msg["data"]["id"], "dir1");
    assert_eq!(msg["data"]["isFileMode"], false);
}

#[tokio::test]
async fn broadcast_reaches_all_connected_clients() {
    let (state, manager) = default_state();
    let addr = start_server(state).await;

    let (_, mut rx1) = connect_with_token(addr, "valid-token").await;
    let (_, mut rx2) = connect_with_token(addr, "valid-token").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(manager.client_count(), 2);

    let event = WebSocketMessage::new("test-broadcast", json!({"seq": 1}));
    manager.broadcast_all(event);

    let msg1 = read_text(&mut rx1).await;
    let msg2 = read_text(&mut rx2).await;

    assert_eq!(msg1["name"], "test-broadcast");
    assert_eq!(msg2["name"], "test-broadcast");
}

#[tokio::test]
async fn authenticated_user_scope_is_enforced_end_to_end() {
    let manager = Arc::new(WebSocketManager::new());
    let state = WsHandlerState {
        manager: manager.clone(),
        token_authenticator: Arc::new(|token| match token {
            "alice-token" => Some("alice".to_owned()),
            "bob-token" => Some("bob".to_owned()),
            _ => None,
        }),
        token_extractor: Arc::new(|headers| {
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                .map(str::to_owned)
        }),
    };
    let addr = start_server(state).await;
    let (_, mut alice_rx) = connect_with_token(addr, "alice-token").await;
    let (_, mut bob_rx) = connect_with_token(addr, "bob-token").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    manager.broadcast_to_user(
        "alice",
        WebSocketMessage::new("agentExecution.leadThinking", json!({"delta": "secret"})),
    );

    let alice = read_text(&mut alice_rx).await;
    assert_eq!(alice["name"], "agentExecution.leadThinking");
    assert_eq!(alice["data"]["delta"], "secret");
    assert!(
        tokio::time::timeout(Duration::from_millis(200), bob_rx.next())
            .await
            .is_err(),
        "another authenticated user must receive no frame",
    );
}

#[tokio::test]
async fn unicast_reaches_only_target() {
    let (state, manager) = default_state();
    let addr = start_server(state).await;

    let (_, mut rx1) = connect_with_token(addr, "valid-token").await;
    let (_, mut rx2) = connect_with_token(addr, "valid-token").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(manager.client_count(), 2);

    // IDs are sequential starting from 1
    let first_conn_id = ConnectionId(1);

    let msg = WebSocketMessage::new("unicast-test", json!({"target": true}));
    manager.send_to(first_conn_id, msg);

    let received = read_text(&mut rx1).await;
    assert_eq!(received["name"], "unicast-test");

    // rx2 should not have received anything — check with short timeout
    let timeout_result = tokio::time::timeout(Duration::from_millis(200), rx2.next()).await;
    assert!(timeout_result.is_err(), "rx2 should not receive the unicast");
}

#[tokio::test]
async fn client_disconnect_removes_from_manager() {
    let (state, manager) = default_state();
    let addr = start_server(state).await;

    let (mut tx, _rx) = connect_with_token(addr, "valid-token").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(manager.client_count(), 1);

    // Send close frame
    tx.send(tungstenite::Message::Close(None)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(manager.client_count(), 0);
}

#[tokio::test]
async fn pong_message_does_not_generate_response() {
    let (state, _) = default_state();
    let addr = start_server(state).await;

    let (mut tx, mut rx) = connect_with_token(addr, "valid-token").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let pong = json!({"name": "pong", "data": {}});
    tx.send(send_json(&pong.to_string())).await.unwrap();

    // pong should not generate any response
    let timeout_result = tokio::time::timeout(Duration::from_millis(200), rx.next()).await;
    assert!(timeout_result.is_err(), "pong should not generate a response");
}

#[tokio::test]
async fn unknown_message_is_discarded_without_response() {
    let (state, _manager) = default_state();

    let addr = start_server(state).await;
    let (mut tx, mut rx) = connect_with_token(addr, "valid-token").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let msg = json!({"name": "custom.business-event", "data": {"key": "val"}});
    tx.send(send_json(&msg.to_string())).await.unwrap();

    // Business requests flow over HTTP; unknown upstream WS messages are
    // silently discarded and must not produce a reply frame.
    let timeout_result = tokio::time::timeout(Duration::from_millis(200), rx.next()).await;
    assert!(timeout_result.is_err(), "unknown message should not generate a response");
}

#[tokio::test]
async fn multiple_concurrent_connections() {
    let (state, manager) = default_state();
    let addr = start_server(state).await;

    let mut handles = Vec::new();
    for _ in 0..10 {
        handles.push(tokio::spawn(
            async move { connect_with_token(addr, "valid-token").await },
        ));
    }

    let mut connections = Vec::new();
    for h in handles {
        connections.push(h.await.unwrap());
    }

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(manager.client_count(), 10);
}
