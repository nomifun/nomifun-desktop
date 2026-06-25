use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::routing::get;
use futures_util::{SinkExt, StreamExt};
use nomifun_api_types::WebSocketMessage;
use nomifun_realtime::{
    ConnectionId, MessageRouter, NoopMessageRouter, WebSocketManager, WsHandlerState, ws_upgrade_handler,
};
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
        router: Arc::new(NoopMessageRouter),
        token_validator: Arc::new(|t| t == "valid-token"),
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
async fn valid_token_connects_successfully() {
    let (state, manager) = default_state();
    let addr = start_server(state).await;

    let (_tx, _rx) = connect_with_token(addr, "valid-token").await;

    // Allow connection to register
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(manager.client_count(), 1);
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
async fn unknown_message_routed_to_message_router() {
    use std::sync::atomic::{AtomicBool, Ordering};

    struct TrackingRouter {
        called: AtomicBool,
    }
    impl MessageRouter for TrackingRouter {
        fn route(&self, _conn_id: ConnectionId, _name: &str, _data: Value) {
            self.called.store(true, Ordering::Relaxed);
        }
    }

    let manager = Arc::new(WebSocketManager::new());
    let router = Arc::new(TrackingRouter {
        called: AtomicBool::new(false),
    });
    let state = WsHandlerState {
        manager: manager.clone(),
        router: router.clone(),
        token_validator: Arc::new(|t| t == "valid-token"),
        token_extractor: Arc::new(|headers| {
            headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.strip_prefix("Bearer "))
                .map(|s| s.to_owned())
        }),
    };

    let addr = start_server(state).await;
    let (mut tx, _rx) = connect_with_token(addr, "valid-token").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let msg = json!({"name": "custom.business-event", "data": {"key": "val"}});
    tx.send(send_json(&msg.to_string())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(router.called.load(Ordering::Relaxed));
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
