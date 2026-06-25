//! WebSocket event emission for IDMM. Mirrors `nomifun_requirement::events`.
//! Event names follow the `domain.camelCaseAction` convention.

use std::sync::Arc;

use nomifun_api_types::{IdmmState, InterventionRecord, WebSocketMessage};
use nomifun_realtime::EventBroadcaster;
use tracing::error;

/// Emits IDMM status + intervention events through the shared broadcaster.
#[derive(Clone)]
pub struct IdmmEventEmitter {
    broadcaster: Arc<dyn EventBroadcaster>,
}

impl IdmmEventEmitter {
    pub fn new(broadcaster: Arc<dyn EventBroadcaster>) -> Self {
        Self { broadcaster }
    }

    /// `idmm.statusChanged` — armed/disabled/intervening transitions.
    pub fn emit_status_changed(&self, state: &IdmmState) {
        self.broadcast("idmm.statusChanged", state);
    }

    /// `idmm.intervention` — one intervention happened (detected → action → outcome).
    pub fn emit_intervention(&self, record: &InterventionRecord) {
        self.broadcast("idmm.intervention", record);
    }

    fn broadcast<T: serde::Serialize>(&self, event_name: &str, payload: &T) {
        let value = match serde_json::to_value(payload) {
            Ok(v) => v,
            Err(e) => {
                error!(event = event_name, error = %e, "IDMM event serialize failed");
                return;
            }
        };
        self.broadcaster.broadcast(WebSocketMessage::new(event_name, value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_api_types::{IdmmRunState, IdmmTargetKind};
    use std::sync::Mutex;

    #[derive(Default)]
    struct CapturingBroadcaster {
        events: Mutex<Vec<WebSocketMessage<serde_json::Value>>>,
    }
    impl EventBroadcaster for CapturingBroadcaster {
        fn broadcast(&self, event: WebSocketMessage<serde_json::Value>) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[test]
    fn emits_status_changed_with_payload() {
        let bc = Arc::new(CapturingBroadcaster::default());
        let emitter = IdmmEventEmitter::new(bc.clone());
        let st = IdmmState {
            kind: IdmmTargetKind::Conversation,
            target_id: "c1".into(),
            enabled: true,
            fault_enabled: false,
            decision_enabled: true,
            run_state: IdmmRunState::Armed,
            interventions_count: 0,
            last_signal: None,
            last_intervention_at: None,
            sidecar_provider_resolved: false,
            config: None,
        };
        emitter.emit_status_changed(&st);
        let evs = bc.events.lock().unwrap();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].name, "idmm.statusChanged");
        assert_eq!(evs[0].data["target_id"], "c1");
        assert_eq!(evs[0].data["run_state"], "armed");
    }

    #[test]
    fn emits_intervention_with_payload() {
        let bc = Arc::new(CapturingBroadcaster::default());
        let emitter = IdmmEventEmitter::new(bc.clone());
        let rec = InterventionRecord {
            id: "idmmrec_x".into(),
            target_kind: "conversation".into(),
            target_id: "t1".into(),
            watch: "fault".into(),
            at: 123,
            stall_class: "provider_error".into(),
            tier_used: "rule".into(),
            category: None,
            action: "retry".into(),
            detail: None,
            outcome: "applied".into(),
            reason: Some("transient 500".into()),
            confidence: None,
            bypass_model: None,
        };
        emitter.emit_intervention(&rec);
        let evs = bc.events.lock().unwrap();
        assert_eq!(evs[0].name, "idmm.intervention");
        assert_eq!(evs[0].data["action"], "retry");
    }
}
