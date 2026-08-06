//! Realtime emission for SSH link status. Event names follow the
//! `domain.camelCaseAction` convention; the payload is `dto::SshStatusEvent`.
//!
//! Link status is not turn-scoped — an idle conversation must still learn that
//! its host went away — so it travels on the owner's realtime channel rather
//! than through the agent's per-turn stream.

use std::sync::Arc;

use nomifun_api_types::WebSocketMessage;
use nomifun_realtime::UserEventSink;
use tracing::error;

use crate::dto::SshStatusEvent;

/// Emits SSH link transitions to the host's owner only.
#[derive(Clone)]
pub struct SshEventEmitter {
    user_events: Arc<dyn UserEventSink>,
}

impl SshEventEmitter {
    pub fn new(user_events: Arc<dyn UserEventSink>) -> Self {
        Self { user_events }
    }

    /// `ssh.status` — one link changed phase (connecting, dropped, closed, ...).
    pub fn emit_status(&self, owner_id: &str, payload: &SshStatusEvent) {
        self.send(owner_id, "ssh.status", payload);
    }

    fn send<T: serde::Serialize>(&self, owner_id: &str, event_name: &str, payload: &T) {
        let value = match serde_json::to_value(payload) {
            Ok(v) => v,
            Err(e) => {
                error!(event = event_name, error = %e, "SSH event serialize failed");
                return;
            }
        };
        self.user_events
            .send_to_user(owner_id, WebSocketMessage::new(event_name, value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::SshLinkState;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingUserEvents {
        deliveries: Mutex<Vec<(String, WebSocketMessage<serde_json::Value>)>>,
    }
    impl UserEventSink for RecordingUserEvents {
        fn send_to_user(&self, user_id: &str, event: WebSocketMessage<serde_json::Value>) {
            self.deliveries
                .lock()
                .unwrap()
                .push((user_id.to_owned(), event));
        }
    }

    #[test]
    fn emits_ssh_status_to_the_owner() {
        let sink = Arc::new(RecordingUserEvents::default());
        let emitter = SshEventEmitter::new(sink.clone());
        let payload = SshStatusEvent::from_state(
            "0190f5fe-7c00-7a00-8000-0000000000aa",
            "0190f5fe-7c00-7a00-8000-0000000000bb",
            &SshLinkState::Reconnecting {
                attempt: 2,
                next_retry_in_ms: 2_000,
            },
            1_700_000_000_000,
        );

        emitter.emit_status("0190f5fe-7c00-7a00-8000-000000000001", &payload);

        let deliveries = sink.deliveries.lock().unwrap();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].0, "0190f5fe-7c00-7a00-8000-000000000001");
        assert_eq!(deliveries[0].1.name, "ssh.status");
        let round_tripped: SshStatusEvent =
            serde_json::from_value(deliveries[0].1.data.clone()).expect("payload round-trips");
        assert_eq!(round_tripped, payload);
    }
}
