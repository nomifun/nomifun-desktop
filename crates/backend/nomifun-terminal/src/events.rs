use std::sync::Arc;

use nomifun_api_types::{
    TerminalExitEvent, TerminalOutputEvent, TerminalRemovedPayload, TerminalSessionResponse, WebSocketMessage,
};
use nomifun_realtime::EventBroadcaster;
use tracing::error;

/// Broadcasts terminal lifecycle + stream events over the realtime WebSocket bus.
#[derive(Clone)]
pub struct TerminalEventEmitter {
    broadcaster: Arc<dyn EventBroadcaster>,
}

impl TerminalEventEmitter {
    pub fn new(broadcaster: Arc<dyn EventBroadcaster>) -> Self {
        Self { broadcaster }
    }

    /// A chunk of PTY output (base64-encoded bytes).
    pub fn emit_output(&self, id: i64, data_b64: String) {
        self.broadcast("terminal.output", &TerminalOutputEvent { id, data_b64 });
    }

    /// The child process exited.
    pub fn emit_exit(&self, id: i64, exit_code: Option<i32>) {
        self.broadcast("terminal.exit", &TerminalExitEvent { id, exit_code });
    }

    pub fn emit_created(&self, session: &TerminalSessionResponse) {
        self.broadcast("terminal.created", session);
    }

    pub fn emit_updated(&self, session: &TerminalSessionResponse) {
        self.broadcast("terminal.updated", session);
    }

    pub fn emit_removed(&self, id: i64) {
        self.broadcast("terminal.removed", &TerminalRemovedPayload { id });
    }

    fn broadcast<T: serde::Serialize>(&self, event_name: &str, payload: &T) {
        let value = match serde_json::to_value(payload) {
            Ok(v) => v,
            Err(e) => {
                error!(event_name, error = %e, "Failed to serialize terminal event payload");
                return;
            }
        };
        self.broadcaster.broadcast(WebSocketMessage::new(event_name, value));
    }
}
