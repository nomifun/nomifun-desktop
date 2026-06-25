use std::sync::Arc;

use nomifun_api_types::WebSocketMessage;
use nomifun_realtime::{BroadcastEventBus, EventBroadcaster};
use serde_json::json;

#[tokio::test]
async fn broadcast_to_multiple_subscribers() {
    let bus = Arc::new(BroadcastEventBus::new(64));
    let mut rx1 = bus.subscribe();
    let mut rx2 = bus.subscribe();
    let mut rx3 = bus.subscribe();

    let event = WebSocketMessage::new("test:broadcast", json!({"key": "value"}));
    bus.broadcast(event);

    let msg1 = rx1.recv().await.unwrap();
    let msg2 = rx2.recv().await.unwrap();
    let msg3 = rx3.recv().await.unwrap();

    assert_eq!(msg1.name, "test:broadcast");
    assert_eq!(msg2.name, "test:broadcast");
    assert_eq!(msg3.name, "test:broadcast");
    assert_eq!(msg1.data, msg2.data);
    assert_eq!(msg2.data, msg3.data);
}

#[tokio::test]
async fn late_subscriber_misses_earlier_events() {
    let bus = BroadcastEventBus::new(64);

    // Broadcast before any subscriber exists
    bus.broadcast(WebSocketMessage::new("early", json!({})));

    // Subscribe after the broadcast
    let mut rx = bus.subscribe();

    // Broadcast a new event
    bus.broadcast(WebSocketMessage::new("late", json!({})));

    let msg = rx.recv().await.unwrap();
    assert_eq!(msg.name, "late");
}

#[tokio::test]
async fn dropped_subscriber_does_not_block_broadcast() {
    let bus = BroadcastEventBus::new(64);
    let rx = bus.subscribe();
    assert_eq!(bus.receiver_count(), 1);

    drop(rx);
    assert_eq!(bus.receiver_count(), 0);

    // Broadcast should succeed without panic
    bus.broadcast(WebSocketMessage::new("after-drop", json!({})));
}

#[tokio::test]
async fn trait_object_via_arc() {
    let bus = Arc::new(BroadcastEventBus::new(64));
    let mut rx = bus.subscribe();

    let broadcaster: Arc<dyn EventBroadcaster> = bus.clone();
    broadcaster.broadcast(WebSocketMessage::new("via-trait", json!({"n": 42})));

    let msg = rx.recv().await.unwrap();
    assert_eq!(msg.name, "via-trait");
    assert_eq!(msg.data["n"], 42);
}

#[tokio::test]
async fn high_throughput_broadcast() {
    let bus = Arc::new(BroadcastEventBus::new(256));
    let mut rx = bus.subscribe();

    let count = 100;
    for i in 0..count {
        bus.broadcast(WebSocketMessage::new(format!("evt-{i}"), json!({"seq": i})));
    }

    for i in 0..count {
        let msg = rx.recv().await.unwrap();
        assert_eq!(msg.name, format!("evt-{i}"));
        assert_eq!(msg.data["seq"], i);
    }
}
