use std::sync::Arc;
use std::time::Duration;

use nomifun_api_types::WebSocketMessage;
use nomifun_realtime::{
    ConnectionId, PER_CONNECTION_BUFFER, TokenAuthenticator, WebSocketCloseCode, WebSocketManager, WsOutbound,
};
use serde_json::json;
use tokio::sync::mpsc;

fn always_valid() -> TokenAuthenticator {
    Arc::new(|_| Some("user".to_owned()))
}

fn new_client_tx() -> (mpsc::Sender<WsOutbound>, mpsc::Receiver<WsOutbound>) {
    mpsc::channel(PER_CONNECTION_BUFFER)
}

// --- Connection lifecycle ---

#[test]
fn register_and_remove_multiple_clients() {
    let mgr = WebSocketManager::new();
    let mut ids = Vec::new();

    for i in 0..10 {
        let (tx, _rx) = new_client_tx();
        let id = mgr.add_client("user".into(), format!("token-{i}"), tx);
        ids.push(id);
    }

    assert_eq!(mgr.client_count(), 10);

    // Remove every other client
    for id in ids.iter().step_by(2) {
        mgr.remove_client(*id);
    }
    assert_eq!(mgr.client_count(), 5);

    // Remove remaining
    for id in ids.iter().skip(1).step_by(2) {
        mgr.remove_client(*id);
    }
    assert_eq!(mgr.client_count(), 0);
}

#[test]
fn connection_ids_are_unique_and_monotonic() {
    let mgr = WebSocketManager::new();
    let mut ids = Vec::new();

    for _ in 0..100 {
        let (tx, _rx) = new_client_tx();
        ids.push(mgr.add_client("user".into(), "tok".into(), tx));
    }

    // Check uniqueness
    let mut sorted = ids.clone();
    sorted.sort_by_key(|id| id.0);
    sorted.dedup();
    assert_eq!(sorted.len(), 100);

    // Check monotonic
    for window in ids.windows(2) {
        assert!(window[0].0 < window[1].0);
    }
}

// --- Broadcast ---

#[test]
fn broadcast_all_delivers_identical_content_to_every_client() {
    let mgr = WebSocketManager::new();
    let mut receivers = Vec::new();

    for i in 0..5 {
        let (tx, rx) = new_client_tx();
        mgr.add_client("user".into(), format!("token-{i}"), tx);
        receivers.push(rx);
    }

    let event = WebSocketMessage::new("notification", json!({"level": "info", "text": "hello"}));
    mgr.broadcast_all(event);

    let mut texts = Vec::new();
    for rx in &mut receivers {
        match rx.try_recv().unwrap() {
            WsOutbound::Text(t) => texts.push(t),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    // All received identical content
    assert!(texts.windows(2).all(|w| w[0] == w[1]));
    assert!(texts[0].contains("notification"));
}

#[test]
fn user_scoped_broadcast_never_crosses_authenticated_identity() {
    let mgr = WebSocketManager::new();
    let (alice_tx, mut alice_rx) = new_client_tx();
    let (bob_tx, mut bob_rx) = new_client_tx();
    mgr.add_client("alice".into(), "alice-token".into(), alice_tx);
    mgr.add_client("bob".into(), "bob-token".into(), bob_tx);

    mgr.broadcast_to_user(
        "alice",
        WebSocketMessage::new("agentExecution.leadThinking", json!({"delta": "private"})),
    );

    let alice = alice_rx.try_recv().expect("owner connection receives its event");
    assert!(matches!(alice, WsOutbound::Text(text) if text.contains("private")));
    assert!(bob_rx.try_recv().is_err(), "another user must receive no frame");
}

#[test]
fn broadcast_cleans_up_disconnected_clients_transparently() {
    let mgr = WebSocketManager::new();

    // 3 live clients
    let (tx1, _rx1) = new_client_tx();
    let (tx2, _rx2) = new_client_tx();
    let (tx3, _rx3) = new_client_tx();
    mgr.add_client("user".into(), "a".into(), tx1);
    mgr.add_client("user".into(), "b".into(), tx2);
    mgr.add_client("user".into(), "c".into(), tx3);

    // 2 dead clients (receivers dropped)
    let (tx4, rx4) = new_client_tx();
    let (tx5, rx5) = new_client_tx();
    mgr.add_client("user".into(), "dead-1".into(), tx4);
    mgr.add_client("user".into(), "dead-2".into(), tx5);
    drop(rx4);
    drop(rx5);

    assert_eq!(mgr.client_count(), 5);

    mgr.broadcast_all(WebSocketMessage::new("check", json!(null)));

    // Dead clients should be removed
    assert_eq!(mgr.client_count(), 3);
}

// --- Unicast ---

#[test]
fn send_to_reaches_only_target_connection() {
    let mgr = WebSocketManager::new();
    let mut pairs: Vec<(ConnectionId, mpsc::Receiver<WsOutbound>)> = Vec::new();

    for i in 0..5 {
        let (tx, rx) = new_client_tx();
        let id = mgr.add_client("user".into(), format!("token-{i}"), tx);
        pairs.push((id, rx));
    }

    let target_id = pairs[2].0;
    mgr.send_to(target_id, WebSocketMessage::new("private", json!({"secret": true})));

    for (id, rx) in &mut pairs {
        if *id == target_id {
            let msg = rx.try_recv().unwrap();
            match msg {
                WsOutbound::Text(t) => assert!(t.contains("private")),
                other => panic!("expected Text, got {other:?}"),
            }
        } else {
            assert!(rx.try_recv().is_err(), "non-target {id} should not receive message");
        }
    }
}

// --- Heartbeat integration ---

#[tokio::test]
async fn heartbeat_sends_ping_and_keeps_healthy_connections() {
    let mgr = WebSocketManager::new();
    let (tx, mut rx) = new_client_tx();
    mgr.add_client("user".into(), "valid-token".into(), tx);

    let handle = mgr.start_heartbeat(always_valid());

    // Wait for first heartbeat tick (interval is 30s, but first tick fires immediately)
    let msg = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout waiting for ping")
        .expect("channel closed");

    match msg {
        WsOutbound::Text(text) => {
            let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(parsed["name"], "ping");
            assert!(parsed["data"]["timestamp"].is_u64());
        }
        other => panic!("expected ping Text, got {other:?}"),
    }

    // Connection should still be alive
    assert_eq!(mgr.client_count(), 1);

    handle.abort();
}

#[tokio::test]
async fn heartbeat_closes_expired_token_with_auth_expired_event() {
    let mgr = WebSocketManager::new();
    let (tx, mut rx) = new_client_tx();
    mgr.add_client("user".into(), "bad-token".into(), tx);

    let expired_authenticator: TokenAuthenticator = Arc::new(|_| None);
    let handle = mgr.start_heartbeat(expired_authenticator);

    // Expect auth-expired event
    let msg1 = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout")
        .expect("closed");

    match msg1 {
        WsOutbound::Text(text) => {
            let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(parsed["name"], "auth-expired");
            assert!(parsed["data"]["message"].is_string());
        }
        other => panic!("expected auth-expired, got {other:?}"),
    }

    // Expect close frame
    let msg2 = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timeout")
        .expect("closed");

    assert_eq!(
        msg2,
        WsOutbound::Close(WebSocketCloseCode::PolicyViolation, "token expired".into())
    );

    // Connection should be removed
    assert_eq!(mgr.client_count(), 0);

    handle.abort();
}

// --- Concurrent access ---

#[test]
fn concurrent_add_remove_does_not_panic() {
    let mgr = Arc::new(WebSocketManager::new());
    let mut handles = Vec::new();

    // Spawn threads that add clients
    for i in 0..10 {
        let mgr = Arc::clone(&mgr);
        handles.push(std::thread::spawn(move || {
            let (tx, _rx) = new_client_tx();
            mgr.add_client("user".into(), format!("thread-{i}"), tx)
        }));
    }

    let ids: Vec<ConnectionId> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    assert_eq!(mgr.client_count(), 10);

    // All IDs should be unique
    let mut unique = ids.clone();
    unique.sort_by_key(|id| id.0);
    unique.dedup();
    assert_eq!(unique.len(), 10);

    // Remove all concurrently
    let mut handles = Vec::new();
    for id in ids {
        let mgr = Arc::clone(&mgr);
        handles.push(std::thread::spawn(move || {
            mgr.remove_client(id);
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(mgr.client_count(), 0);
}

#[test]
fn concurrent_broadcast_does_not_panic() {
    let mgr = Arc::new(WebSocketManager::new());
    let mut _receivers = Vec::new();

    for i in 0..5 {
        let (tx, rx) = new_client_tx();
        mgr.add_client("user".into(), format!("tok-{i}"), tx);
        _receivers.push(rx);
    }

    let mut handles = Vec::new();
    for i in 0..10 {
        let mgr = Arc::clone(&mgr);
        handles.push(std::thread::spawn(move || {
            mgr.broadcast_all(WebSocketMessage::new(format!("event-{i}"), json!(null)));
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    // All clients should still be connected
    assert_eq!(mgr.client_count(), 5);
}
