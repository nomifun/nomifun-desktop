use nomifun_api_types::WebSocketMessage;
use tokio::sync::broadcast;
use tracing::warn;

/// Sink for events whose owning resource is the whole application instance.
///
/// This is intentionally a narrow boundary, not the default event API. Only
/// domains whose DB/API model is explicitly instance-owned may depend on it.
/// Any event derived from a user-owned row, request, session, or credential
/// must use [`UserEventSink`] so the audience cannot be dropped in transit.
///
/// Note: `send_to` (unicast) is intentionally NOT part of this trait.
/// Unicast is a connection-management concern handled by `WebSocketManager`.
pub trait EventBroadcaster: Send + Sync {
    /// Publish an instance-owned event to all connected clients.
    fn broadcast(&self, event: WebSocketMessage<serde_json::Value>);
}

/// Delivers an event only to connections authenticated as one application user.
///
/// User-owned runtime state must use this boundary instead of the process-wide
/// [`EventBroadcaster`]. Keeping the audience in the method signature makes it
/// impossible for a producer to accidentally publish private content without
/// naming its owner.
pub trait UserEventSink: Send + Sync {
    fn send_to_user(&self, user_id: &str, event: WebSocketMessage<serde_json::Value>);
}

/// Internal envelope for a user-scoped event. The owner travels with the event
/// through server-side observers and the WebSocket bridge, so audience
/// information is never reconstructed from payload fields.
#[derive(Debug, Clone)]
pub struct UserEventEnvelope {
    pub user_id: String,
    pub event: WebSocketMessage<serde_json::Value>,
}

/// Default implementation of [`EventBroadcaster`] backed by
/// `tokio::sync::broadcast` channel.
///
/// The broadcast channel is used for module-to-WebSocket event fan-out.
/// Each `WebSocketManager` connection subscribes to this channel and
/// forwards received events to its per-connection `mpsc` sender.
pub struct BroadcastEventBus {
    tx: broadcast::Sender<WebSocketMessage<serde_json::Value>>,
    user_tx: broadcast::Sender<UserEventEnvelope>,
}

impl BroadcastEventBus {
    /// Create a new event bus with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        let (user_tx, _user_rx) = broadcast::channel(capacity);
        Self { tx, user_tx }
    }

    /// Subscribe to receive broadcast events.
    ///
    /// Each WebSocket connection calls this once to get its own receiver.
    pub fn subscribe(&self) -> broadcast::Receiver<WebSocketMessage<serde_json::Value>> {
        self.tx.subscribe()
    }

    /// Subscribe to owner-scoped events for internal observers or the
    /// WebSocket user-delivery bridge.
    pub fn subscribe_user(&self) -> broadcast::Receiver<UserEventEnvelope> {
        self.user_tx.subscribe()
    }

    /// Returns the number of active subscribers.
    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }

    pub fn user_receiver_count(&self) -> usize {
        self.user_tx.receiver_count()
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

impl UserEventSink for BroadcastEventBus {
    fn send_to_user(&self, user_id: &str, event: WebSocketMessage<serde_json::Value>) {
        let envelope = UserEventEnvelope {
            user_id: user_id.to_owned(),
            event,
        };
        if let Err(error) = self.user_tx.send(envelope) {
            warn!(
                user_id,
                event_name = %error.0.event.name,
                "user event delivery failed: no active receivers"
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

    #[tokio::test]
    async fn user_events_preserve_owner_for_internal_subscribers() {
        let bus = BroadcastEventBus::new(16);
        let mut rx = bus.subscribe_user();

        bus.send_to_user(
            "owner-a",
            WebSocketMessage::new("message.stream", json!({"conversation_id": 1})),
        );

        let received = rx.recv().await.unwrap();
        assert_eq!(received.user_id, "owner-a");
        assert_eq!(received.event.name, "message.stream");
        assert_eq!(received.event.data["conversation_id"], 1);
    }

    #[test]
    fn trait_object_compatible() {
        let bus = BroadcastEventBus::new(16);
        let broadcaster: &dyn EventBroadcaster = &bus;
        let event = WebSocketMessage::new("test", json!(null));
        broadcaster.broadcast(event);
    }
}
