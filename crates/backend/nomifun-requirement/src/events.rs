use std::sync::Arc;

use nomifun_api_types::{AutoWorkState, Requirement, RequirementDeletedPayload, TagPausedPayload, WebSocketMessage};
use nomifun_realtime::EventBroadcaster;
use tracing::error;

/// Emits Requirements-Platform WebSocket events (`domain.camelCaseAction`).
#[derive(Clone)]
pub struct RequirementEventEmitter {
    broadcaster: Arc<dyn EventBroadcaster>,
}

impl RequirementEventEmitter {
    pub fn new(broadcaster: Arc<dyn EventBroadcaster>) -> Self {
        Self { broadcaster }
    }

    pub fn emit_created(&self, req: &Requirement) {
        self.broadcast("requirement.created", req);
    }

    pub fn emit_updated(&self, req: &Requirement) {
        self.broadcast("requirement.updated", req);
    }

    pub fn emit_status_changed(&self, req: &Requirement) {
        self.broadcast("requirement.statusChanged", req);
    }

    pub fn emit_deleted(&self, id: i64) {
        self.broadcast("requirement.deleted", &RequirementDeletedPayload { id });
    }

    pub fn emit_autowork_changed(&self, state: &AutoWorkState) {
        self.broadcast("autowork.statusChanged", state);
    }

    /// AutoWork paused a tag after a requirement exhausted its retries.
    pub fn emit_tag_paused(&self, payload: &TagPausedPayload) {
        self.broadcast("autowork.tagPaused", payload);
    }

    fn broadcast<T: serde::Serialize>(&self, event_name: &str, payload: &T) {
        let value = match serde_json::to_value(payload) {
            Ok(v) => v,
            Err(e) => {
                error!(event_name, error = %e, "Failed to serialize requirement event payload");
                return;
            }
        };
        self.broadcaster.broadcast(WebSocketMessage::new(event_name, value));
    }
}
