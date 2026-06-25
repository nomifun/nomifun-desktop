use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use nomifun_api_types::WebSocketMessage;
use serde_json::json;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::broadcaster::EventBroadcaster;
use crate::types::{ClientInfo, ConnectionId, HEARTBEAT_INTERVAL, HEARTBEAT_TIMEOUT, WebSocketCloseCode, WsOutbound};

/// Validates whether a JWT token is still valid.
/// Returns `true` if the token is valid, `false` if expired or revoked.
pub type TokenValidator = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// Manages active WebSocket connections, heartbeat detection,
/// and provides broadcast/unicast messaging.
pub struct WebSocketManager {
    connections: Arc<DashMap<ConnectionId, ClientInfo>>,
    next_id: AtomicU64,
}

impl WebSocketManager {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(DashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Register a new client connection and return its assigned ID.
    pub fn add_client(&self, token: String, tx: mpsc::Sender<WsOutbound>) -> ConnectionId {
        let id = ConnectionId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let info = ClientInfo {
            token,
            last_ping: Instant::now(),
            tx,
        };
        self.connections.insert(id, info);
        debug!(%id, "client added");
        id
    }

    /// Remove a client connection by ID.
    pub fn remove_client(&self, conn_id: ConnectionId) {
        if self.connections.remove(&conn_id).is_some() {
            debug!(%conn_id, "client removed");
        }
    }

    /// Update the last heartbeat timestamp for a connection.
    pub fn update_last_ping(&self, conn_id: ConnectionId) {
        if let Some(mut client) = self.connections.get_mut(&conn_id) {
            client.last_ping = Instant::now();
        }
    }

    /// Returns the number of active connections.
    pub fn client_count(&self) -> usize {
        self.connections.len()
    }

    /// Send a message to all connected clients.
    ///
    /// Uses `try_send` for backpressure — full channels drop the message
    /// with a warning; closed channels trigger client removal.
    pub fn broadcast_all(&self, msg: WebSocketMessage<serde_json::Value>) {
        let text = match serde_json::to_string(&msg) {
            Ok(t) => t,
            Err(e) => {
                warn!(error = %e, "failed to serialize broadcast message");
                return;
            }
        };

        let mut disconnected = Vec::new();
        for entry in self.connections.iter() {
            let conn_id = *entry.key();
            match entry.value().tx.try_send(WsOutbound::Text(text.clone())) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    warn!(%conn_id, "outbound channel full, message dropped");
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    disconnected.push(conn_id);
                }
            }
        }

        for conn_id in disconnected {
            self.remove_client(conn_id);
        }
    }

    /// Send a message to a specific connection.
    pub fn send_to(&self, conn_id: ConnectionId, msg: WebSocketMessage<serde_json::Value>) {
        let text = match serde_json::to_string(&msg) {
            Ok(t) => t,
            Err(e) => {
                warn!(
                    %conn_id, error = %e,
                    "failed to serialize unicast message"
                );
                return;
            }
        };

        self.send_raw_to(conn_id, WsOutbound::Text(text));
    }

    /// Send a raw outbound message to a specific connection.
    ///
    /// Used for non-`WebSocketMessage` payloads (e.g. error responses).
    pub fn send_raw_to(&self, conn_id: ConnectionId, outbound: WsOutbound) {
        if let Some(client) = self.connections.get(&conn_id) {
            match client.tx.try_send(outbound) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    warn!(%conn_id, "outbound channel full, message dropped");
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    drop(client);
                    self.remove_client(conn_id);
                }
            }
        }
    }

    /// Start the heartbeat check loop.
    ///
    /// Every `HEARTBEAT_INTERVAL` (30s), iterates all connections:
    /// 1. Timeout check — closes connections with no pong for `HEARTBEAT_TIMEOUT`
    /// 2. Token expiry — validates token and sends `auth-expired` if invalid
    /// 3. Sends a `ping` message with current timestamp
    ///
    /// Returns a `JoinHandle` — abort it to stop the heartbeat loop.
    pub fn start_heartbeat(&self, token_validator: TokenValidator) -> JoinHandle<()> {
        let connections = Arc::clone(&self.connections);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
            loop {
                interval.tick().await;
                heartbeat_tick(&connections, &token_validator);
            }
        })
    }
}

impl Default for WebSocketManager {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBroadcaster for WebSocketManager {
    fn broadcast(&self, event: WebSocketMessage<serde_json::Value>) {
        self.broadcast_all(event);
    }
}

/// Single heartbeat tick: check timeouts, token validity, send pings.
fn heartbeat_tick(connections: &DashMap<ConnectionId, ClientInfo>, token_validator: &TokenValidator) {
    let now = Instant::now();
    let mut to_remove = Vec::new();

    for entry in connections.iter() {
        let conn_id = *entry.key();
        let client = entry.value();

        // 1. Heartbeat timeout
        if now.duration_since(client.last_ping) > HEARTBEAT_TIMEOUT {
            info!(%conn_id, "heartbeat timeout, closing connection");
            let _ = client.tx.try_send(WsOutbound::Close(
                WebSocketCloseCode::PolicyViolation,
                "heartbeat timeout".into(),
            ));
            to_remove.push(conn_id);
            continue;
        }

        // 2. Token expiry
        if !token_validator(&client.token) {
            info!(%conn_id, "token expired, closing connection");
            let auth_expired = WebSocketMessage::new("auth-expired", json!({"message": "Token expired"}));
            if let Ok(text) = serde_json::to_string(&auth_expired) {
                let _ = client.tx.try_send(WsOutbound::Text(text));
            }
            let _ = client.tx.try_send(WsOutbound::Close(
                WebSocketCloseCode::PolicyViolation,
                "token expired".into(),
            ));
            to_remove.push(conn_id);
            continue;
        }

        // 3. Send ping
        let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let timestamp = duration.as_secs() * 1000 + u64::from(duration.subsec_millis());

        let ping = WebSocketMessage::new("ping", json!({"timestamp": timestamp}));
        if let Ok(text) = serde_json::to_string(&ping) {
            match client.tx.try_send(WsOutbound::Text(text)) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    warn!(%conn_id, "outbound channel full, ping dropped");
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    to_remove.push(conn_id);
                }
            }
        }
    }

    for conn_id in to_remove {
        connections.remove(&conn_id);
        debug!(%conn_id, "connection removed by heartbeat");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PER_CONNECTION_BUFFER;

    fn always_valid() -> TokenValidator {
        Arc::new(|_| true)
    }

    fn always_expired() -> TokenValidator {
        Arc::new(|_| false)
    }

    fn new_client_tx() -> (mpsc::Sender<WsOutbound>, mpsc::Receiver<WsOutbound>) {
        mpsc::channel(PER_CONNECTION_BUFFER)
    }

    #[test]
    fn add_client_assigns_sequential_ids() {
        let mgr = WebSocketManager::new();
        let (tx1, _rx1) = new_client_tx();
        let (tx2, _rx2) = new_client_tx();

        let id1 = mgr.add_client("token-a".into(), tx1);
        let id2 = mgr.add_client("token-b".into(), tx2);

        assert_eq!(id1, ConnectionId(1));
        assert_eq!(id2, ConnectionId(2));
        assert_eq!(mgr.client_count(), 2);
    }

    #[test]
    fn remove_client_decrements_count() {
        let mgr = WebSocketManager::new();
        let (tx, _rx) = new_client_tx();
        let id = mgr.add_client("token".into(), tx);

        assert_eq!(mgr.client_count(), 1);
        mgr.remove_client(id);
        assert_eq!(mgr.client_count(), 0);
    }

    #[test]
    fn remove_nonexistent_client_is_noop() {
        let mgr = WebSocketManager::new();
        mgr.remove_client(ConnectionId(999));
        assert_eq!(mgr.client_count(), 0);
    }

    #[test]
    fn update_last_ping_refreshes_timestamp() {
        let mgr = WebSocketManager::new();
        let (tx, _rx) = new_client_tx();
        let id = mgr.add_client("token".into(), tx);

        let before = mgr.connections.get(&id).map(|c| c.last_ping).unwrap();

        // Small busy-wait to ensure time advances
        std::thread::sleep(std::time::Duration::from_millis(5));

        mgr.update_last_ping(id);

        let after = mgr.connections.get(&id).map(|c| c.last_ping).unwrap();

        assert!(after > before);
    }

    #[test]
    fn update_last_ping_nonexistent_is_noop() {
        let mgr = WebSocketManager::new();
        mgr.update_last_ping(ConnectionId(999));
    }

    #[test]
    fn broadcast_all_delivers_to_all() {
        let mgr = WebSocketManager::new();
        let (tx1, mut rx1) = new_client_tx();
        let (tx2, mut rx2) = new_client_tx();

        mgr.add_client("t1".into(), tx1);
        mgr.add_client("t2".into(), tx2);

        let event = WebSocketMessage::new("test-event", json!({"key": "val"}));
        mgr.broadcast_all(event);

        let msg1 = rx1.try_recv().unwrap();
        let msg2 = rx2.try_recv().unwrap();

        match (&msg1, &msg2) {
            (WsOutbound::Text(t1), WsOutbound::Text(t2)) => {
                assert_eq!(t1, t2);
                assert!(t1.contains("test-event"));
            }
            _ => panic!("expected Text messages"),
        }
    }

    #[test]
    fn broadcast_all_removes_closed_channels() {
        let mgr = WebSocketManager::new();
        let (tx1, rx1) = new_client_tx();
        let (tx2, _rx2) = new_client_tx();

        mgr.add_client("t1".into(), tx1);
        mgr.add_client("t2".into(), tx2);

        // Drop rx1 to close the channel
        drop(rx1);

        let event = WebSocketMessage::new("test", json!(null));
        mgr.broadcast_all(event);

        // Client 1 should be removed
        assert_eq!(mgr.client_count(), 1);
    }

    #[test]
    fn broadcast_all_handles_full_channel() {
        let mgr = WebSocketManager::new();
        // Use a channel with capacity 1
        let (tx, _rx) = mpsc::channel(1);
        mgr.add_client("tok".into(), tx);

        // Fill the channel
        mgr.broadcast_all(WebSocketMessage::new("e1", json!(null)));
        // This should warn but not remove the client
        mgr.broadcast_all(WebSocketMessage::new("e2", json!(null)));

        assert_eq!(mgr.client_count(), 1);
    }

    #[test]
    fn send_to_delivers_to_target_only() {
        let mgr = WebSocketManager::new();
        let (tx1, mut rx1) = new_client_tx();
        let (tx2, mut rx2) = new_client_tx();

        let id1 = mgr.add_client("t1".into(), tx1);
        mgr.add_client("t2".into(), tx2);

        let msg = WebSocketMessage::new("unicast", json!({"for": "id1"}));
        mgr.send_to(id1, msg);

        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_err());
    }

    #[test]
    fn send_to_nonexistent_is_noop() {
        let mgr = WebSocketManager::new();
        let msg = WebSocketMessage::new("ghost", json!(null));
        mgr.send_to(ConnectionId(999), msg);
    }

    #[test]
    fn send_to_removes_closed_channel() {
        let mgr = WebSocketManager::new();
        let (tx, rx) = new_client_tx();
        let id = mgr.add_client("tok".into(), tx);
        drop(rx);

        mgr.send_to(id, WebSocketMessage::new("test", json!(null)));
        assert_eq!(mgr.client_count(), 0);
    }

    #[test]
    fn heartbeat_tick_sends_ping_to_healthy_connection() {
        let connections = Arc::new(DashMap::new());
        let (tx, mut rx) = new_client_tx();

        connections.insert(
            ConnectionId(1),
            ClientInfo {
                token: "valid".into(),
                last_ping: Instant::now(),
                tx,
            },
        );

        heartbeat_tick(&connections, &always_valid());

        // Should still be connected
        assert_eq!(connections.len(), 1);

        // Should have received a ping
        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["name"], "ping");
                assert!(parsed["data"]["timestamp"].is_u64());
            }
            _ => panic!("expected Text ping"),
        }
    }

    #[test]
    fn heartbeat_tick_removes_timed_out_connection() {
        let connections = Arc::new(DashMap::new());
        let (tx, mut rx) = new_client_tx();

        // Set last_ping to well past the timeout
        let old_ping = Instant::now() - (HEARTBEAT_TIMEOUT * 2);

        connections.insert(
            ConnectionId(1),
            ClientInfo {
                token: "valid".into(),
                last_ping: old_ping,
                tx,
            },
        );

        heartbeat_tick(&connections, &always_valid());

        // Connection should be removed
        assert_eq!(connections.len(), 0);

        // Should have received a close frame
        let msg = rx.try_recv().unwrap();
        assert_eq!(
            msg,
            WsOutbound::Close(WebSocketCloseCode::PolicyViolation, "heartbeat timeout".into())
        );
    }

    #[test]
    fn heartbeat_tick_removes_expired_token_connection() {
        let connections = Arc::new(DashMap::new());
        let (tx, mut rx) = new_client_tx();

        connections.insert(
            ConnectionId(1),
            ClientInfo {
                token: "expired-token".into(),
                last_ping: Instant::now(),
                tx,
            },
        );

        heartbeat_tick(&connections, &always_expired());

        // Connection should be removed
        assert_eq!(connections.len(), 0);

        // Should have received auth-expired event then close
        let msg1 = rx.try_recv().unwrap();
        match msg1 {
            WsOutbound::Text(text) => {
                let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(parsed["name"], "auth-expired");
            }
            _ => panic!("expected auth-expired Text"),
        }

        let msg2 = rx.try_recv().unwrap();
        assert_eq!(
            msg2,
            WsOutbound::Close(WebSocketCloseCode::PolicyViolation, "token expired".into())
        );
    }

    #[test]
    fn heartbeat_tick_timeout_takes_priority_over_token_check() {
        let connections = Arc::new(DashMap::new());
        let (tx, mut rx) = new_client_tx();

        // Both timed out AND expired token
        let old_ping = Instant::now() - (HEARTBEAT_TIMEOUT * 2);
        connections.insert(
            ConnectionId(1),
            ClientInfo {
                token: "expired".into(),
                last_ping: old_ping,
                tx,
            },
        );

        heartbeat_tick(&connections, &always_expired());

        assert_eq!(connections.len(), 0);

        // Only close frame from timeout (no auth-expired text)
        let msg = rx.try_recv().unwrap();
        assert_eq!(
            msg,
            WsOutbound::Close(WebSocketCloseCode::PolicyViolation, "heartbeat timeout".into())
        );
        // No more messages
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn heartbeat_tick_mixed_connections() {
        let connections = Arc::new(DashMap::new());

        // Healthy connection
        let (tx1, _rx1) = new_client_tx();
        connections.insert(
            ConnectionId(1),
            ClientInfo {
                token: "good".into(),
                last_ping: Instant::now(),
                tx: tx1,
            },
        );

        // Timed-out connection
        let (tx2, _rx2) = new_client_tx();
        connections.insert(
            ConnectionId(2),
            ClientInfo {
                token: "good".into(),
                last_ping: Instant::now() - (HEARTBEAT_TIMEOUT * 2),
                tx: tx2,
            },
        );

        let selective_validator: TokenValidator = Arc::new(|_| true);
        heartbeat_tick(&connections, &selective_validator);

        // Only healthy connection remains
        assert_eq!(connections.len(), 1);
        assert!(connections.contains_key(&ConnectionId(1)));
    }

    #[test]
    fn event_broadcaster_impl_delegates_to_broadcast_all() {
        let mgr = WebSocketManager::new();
        let (tx, mut rx) = new_client_tx();
        mgr.add_client("tok".into(), tx);

        let broadcaster: &dyn EventBroadcaster = &mgr;
        broadcaster.broadcast(WebSocketMessage::new("via-trait", json!({})));

        let msg = rx.try_recv().unwrap();
        match msg {
            WsOutbound::Text(text) => {
                assert!(text.contains("via-trait"));
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn default_creates_empty_manager() {
        let mgr = WebSocketManager::default();
        assert_eq!(mgr.client_count(), 0);
    }
}
