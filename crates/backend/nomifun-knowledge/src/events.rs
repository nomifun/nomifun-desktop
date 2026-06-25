//! WS push events for the knowledge domain. Same shape as `CompanionEventEmitter`:
//! a thin wrapper over the global `EventBroadcaster`.

use std::sync::Arc;

use nomifun_api_types::WebSocketMessage;
use nomifun_realtime::EventBroadcaster;

#[derive(Clone)]
pub struct KnowledgeEventEmitter {
    broadcaster: Arc<dyn EventBroadcaster>,
}

impl KnowledgeEventEmitter {
    pub fn new(broadcaster: Arc<dyn EventBroadcaster>) -> Self {
        Self { broadcaster }
    }

    fn broadcast<T: serde::Serialize>(&self, event_name: &str, payload: &T) {
        let value = match serde_json::to_value(payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, event_name, "failed to serialize knowledge event");
                return;
            }
        };
        self.broadcaster.broadcast(WebSocketMessage::new(event_name, value));
    }

    pub fn emit_base_created<T: serde::Serialize>(&self, base: &T) {
        self.broadcast("knowledge.base-created", base);
    }

    pub fn emit_base_updated<T: serde::Serialize>(&self, base: &T) {
        self.broadcast("knowledge.base-updated", base);
    }

    pub fn emit_base_deleted(&self, id: &str) {
        self.broadcast("knowledge.base-deleted", &serde_json::json!({ "id": id }));
    }

    pub fn emit_binding_changed<T: serde::Serialize>(&self, binding: &T) {
        self.broadcast("knowledge.binding-changed", binding);
    }
}
