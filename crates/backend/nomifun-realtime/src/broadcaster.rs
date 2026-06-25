use nomifun_api_types::WebSocketMessage;
use tokio::sync::broadcast;
use tracing::warn;

/// Trait for broadcasting WebSocket events to all connected clients.
///
/// Business modules depend on this trait (via `Arc<dyn EventBroadcaster>`)
/// to push events without coupling to WebSocket internals.
///
/// Note: `send_to` (unicast) is intentionally NOT part of this trait.
/// Unicast is a connection-management concern handled by `WebSocketManager`.
pub trait EventBroadcaster: Send + Sync {
    /// Broadcast an event to all connected WebSocket clients.
    fn broadcast(&self, event: WebSocketMessage<serde_json::Value>);
}

/// Default implementation of [`EventBroadcaster`] backed by
/// `tokio::sync::broadcast` channel.
///
/// The broadcast channel is used for module-to-WebSocket event fan-out.
/// Each `WebSocketManager` connection subscribes to this channel and
/// forwards received events to its per-connection `mpsc` sender.
pub struct BroadcastEventBus {
    tx: broadcast::Sender<WebSocketMessage<serde_json::Value>>,
}

impl BroadcastEventBus {
    /// Create a new event bus with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Subscribe to receive broadcast events.
    ///
    /// Each WebSocket connection calls this once to get its own receiver.
    pub fn subscribe(&self) -> broadcast::Receiver<WebSocketMessage<serde_json::Value>> {
        self.tx.subscribe()
    }

    /// Returns the number of active subscribers.
    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

impl EventBroadcaster for BroadcastEventBus {
    fn broadcast(&self, event: WebSocketMessage<serde_json::Value>) {
        if let Err(e) = self.tx.send(event) {
            warn!(
                event_name = %e.0.name,
                "broadcast failed: no active receivers"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_bus_has_zero_receivers() {
        let bus = BroadcastEventBus::new(16);
        assert_eq!(bus.receiver_count(), 0);
    }

    #[test]
    fn subscribe_increments_receiver_count() {
        let bus = BroadcastEventBus::new(16);
        let _rx1 = bus.subscribe();
        assert_eq!(bus.receiver_count(), 1);
        let _rx2 = bus.subscribe();
        assert_eq!(bus.receiver_count(), 2);
    }

    #[test]
    fn drop_receiver_decrements_count() {
        let bus = BroadcastEventBus::new(16);
        let rx = bus.subscribe();
        assert_eq!(bus.receiver_count(), 1);
        drop(rx);
        assert_eq!(bus.receiver_count(), 0);
    }

    #[test]
    fn broadcast_without_receivers_does_not_panic() {
        let bus = BroadcastEventBus::new(16);
        let event = WebSocketMessage::new("test", json!({}));
        bus.broadcast(event);
    }

    #[tokio::test]
    async fn broadcast_delivers_to_subscriber() {
        let bus = BroadcastEventBus::new(16);
        let mut rx = bus.subscribe();

        let event = WebSocketMessage::new("chat:update", json!({"id": 1}));
        bus.broadcast(event);

        let received = rx.recv().await.unwrap();
        assert_eq!(received.name, "chat:update");
        assert_eq!(received.data["id"], 1);
    }

    #[tokio::test]
    async fn broadcast_delivers_to_all_subscribers() {
        let bus = BroadcastEventBus::new(16);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        let event = WebSocketMessage::new("ping", json!({"ts": 100}));
        bus.broadcast(event);

        let msg1 = rx1.recv().await.unwrap();
        let msg2 = rx2.recv().await.unwrap();
        assert_eq!(msg1.name, "ping");
        assert_eq!(msg2.name, "ping");
        assert_eq!(msg1.data, msg2.data);
    }

    #[tokio::test]
    async fn multiple_broadcasts_in_order() {
        let bus = BroadcastEventBus::new(16);
        let mut rx = bus.subscribe();

        for i in 0..5 {
            let event = WebSocketMessage::new(format!("event-{i}"), json!({"seq": i}));
            bus.broadcast(event);
        }

        for i in 0..5 {
            let msg = rx.recv().await.unwrap();
            assert_eq!(msg.name, format!("event-{i}"));
            assert_eq!(msg.data["seq"], i);
        }
    }

    #[test]
    fn trait_object_compatible() {
        let bus = BroadcastEventBus::new(16);
        let broadcaster: &dyn EventBroadcaster = &bus;
        let event = WebSocketMessage::new("test", json!(null));
        broadcaster.broadcast(event);
    }
}
