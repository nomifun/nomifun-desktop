//! Realtime emission for robot status.
//!
//! Robot presence is not turn-scoped — an idle desktop must still learn that the
//! robot on the desk went away — so it travels on the owner's realtime channel
//! (`UserEventSink`), exactly like `ssh.status`.

use std::sync::Arc;

use nomifun_api_types::WebSocketMessage;
use nomifun_realtime::UserEventSink;
use tracing::error;

use crate::dto::RobotStatusDto;

/// Emits robot transitions to the installation owner only.
#[derive(Clone)]
pub struct RobotEventEmitter {
    user_events: Arc<dyn UserEventSink>,
}

impl RobotEventEmitter {
    pub fn new(user_events: Arc<dyn UserEventSink>) -> Self {
        Self { user_events }
    }

    /// `robot.status` — one robot changed phase.
    pub fn emit_status(&self, owner_id: &str, payload: &RobotStatusDto) {
        let value = match serde_json::to_value(payload) {
            Ok(v) => v,
            Err(e) => {
                error!(error = %e, "robot: status event serialize failed");
                return;
            }
        };
        self.user_events
            .send_to_user(owner_id, WebSocketMessage::new("robot.status", value));
    }
}
