//! Live phase tracking. One writer (`publish`) both updates the snapshot and
//! emits the event, so the REST snapshot and the WebSocket stream cannot drift.

use std::collections::BTreeMap;

use tokio::sync::RwLock;

use crate::dto::RobotStatusDto;
use crate::events::RobotEventEmitter;

/// What a robot is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RobotPhase {
    Offline,
    Idle,
    Listening,
    Speaking,
}

impl RobotPhase {
    /// Wire name (shared contract with the UI).
    pub fn as_wire(&self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Idle => "idle",
            Self::Listening => "listening",
            Self::Speaking => "speaking",
        }
    }
}

/// Owns current phases and publishes transitions.
pub struct RobotStatusRegistry {
    emitter: RobotEventEmitter,
    owner_id: String,
    inner: RwLock<BTreeMap<String, RobotStatusDto>>,
}

impl RobotStatusRegistry {
    pub async fn publish_for_connection(&self, registry: &crate::registry::RobotRegistry,
        robot_id: &str, connection_id: Option<&str>, companion_id: Option<&str>, phase: RobotPhase, now_ms: i64)
    {
        let Some(connection_id) = connection_id else { return; };
        let Some(_lease) = registry.hold_connection(robot_id, Some(connection_id)).await else { return; };
        self.publish(robot_id, companion_id, phase, now_ms).await;
    }

    pub async fn mark_offline_if_disconnected(&self, registry: &crate::registry::RobotRegistry, robot_id: &str, now_ms: i64) {
        let Some(_lease) = registry.hold_connection(robot_id, None).await else { return; };
        self.mark_offline(robot_id, now_ms).await;
    }
    pub fn new(emitter: RobotEventEmitter, owner_id: String) -> Self {
        Self {
            emitter,
            owner_id,
            inner: RwLock::new(BTreeMap::new()),
        }
    }

    /// Record a phase and emit it. A repeat of the current phase is dropped
    /// (identical phases are not news and would spam the socket).
    pub async fn publish(
        &self,
        robot_id: &str,
        companion_id: Option<&str>,
        phase: RobotPhase,
        now_ms: i64,
    ) {
        let payload = {
            let mut map = self.inner.write().await;
            if let Some(existing) = map.get(robot_id)
                && existing.phase == phase.as_wire()
                && (companion_id.is_none() || existing.companion_id.as_deref() == companion_id)
            {
                return;
            }
            let payload = RobotStatusDto {
                robot_id: robot_id.to_owned(),
                companion_id: companion_id
                    .map(str::to_owned)
                    .or_else(|| map.get(robot_id).and_then(|e| e.companion_id.clone())),
                phase: phase.as_wire().to_owned(),
                changed_at: now_ms,
            };
            map.insert(robot_id.to_owned(), payload.clone());
            payload
        };
        self.emitter.emit_status(&self.owner_id, &payload);
    }

    /// Transition a robot to offline, preserving its known binding.
    pub async fn mark_offline(&self, robot_id: &str, now_ms: i64) {
        self.publish(robot_id, None, RobotPhase::Offline, now_ms)
            .await;
    }

    /// All known phases, ordered by `robot_id`.
    pub async fn snapshot(&self) -> Vec<RobotStatusDto> {
        self.inner.read().await.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::RobotEventEmitter;
    use nomifun_api_types::WebSocketMessage;
    use nomifun_realtime::UserEventSink;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Recording {
        sent: Mutex<Vec<(String, WebSocketMessage<serde_json::Value>)>>,
    }

    impl UserEventSink for Recording {
        fn send_to_user(&self, user_id: &str, event: WebSocketMessage<serde_json::Value>) {
            self.sent.lock().unwrap().push((user_id.to_owned(), event));
        }
    }

    fn registry(sink: Arc<Recording>) -> RobotStatusRegistry {
        RobotStatusRegistry::new(RobotEventEmitter::new(sink), "owner-1".to_owned())
    }

    #[test]
    fn phase_wire_names_match_the_shared_contract() {
        assert_eq!(RobotPhase::Offline.as_wire(), "offline");
        assert_eq!(RobotPhase::Idle.as_wire(), "idle");
        assert_eq!(RobotPhase::Listening.as_wire(), "listening");
        assert_eq!(RobotPhase::Speaking.as_wire(), "speaking");
    }

    #[tokio::test]
    async fn old_connection_status_and_cleanup_cannot_override_a_reconnected_device() {
        let dir = tempfile::tempdir().unwrap();
        let devices = crate::registry::RobotRegistry::load(dir.path()).await.unwrap();
        let (device, _) = devices.upsert_on_report(crate::registry::RobotReport {
            robot_id: "robot-1".to_owned(), client_id: "client".to_owned(), board: "test".to_owned(), firmware_version: "test".to_owned(),
        }, 1).await.unwrap();
        devices.claim(device.activation_code.as_deref().unwrap(), "companion-1").await.unwrap();
        let status = registry(Arc::new(Recording::default()));
        devices.connect("robot-1", "companion-1", "socket-1").await.unwrap();
        status.publish_for_connection(&devices, "robot-1", Some("socket-1"), Some("companion-1"), RobotPhase::Idle, 1).await;
        devices.connect("robot-1", "companion-1", "socket-2").await.unwrap();
        status.publish_for_connection(&devices, "robot-1", Some("socket-1"), Some("companion-1"), RobotPhase::Speaking, 2).await;
        status.mark_offline_if_disconnected(&devices, "robot-1", 3).await;
        assert_eq!(status.snapshot().await[0].phase, "idle");
        devices.patch("robot-1", None, Some(Some("companion-2".to_owned()))).await.unwrap();
        devices.connect("robot-1", "companion-2", "socket-3").await.unwrap();
        status.publish_for_connection(&devices, "robot-1", Some("socket-3"), Some("companion-2"), RobotPhase::Idle, 4).await;
        assert_eq!(status.snapshot().await[0].companion_id.as_deref(), Some("companion-2"));
        devices.disconnect("robot-1", "socket-3").await;
        status.mark_offline_if_disconnected(&devices, "robot-1", 5).await;
        assert_eq!(status.snapshot().await[0].phase, "offline");
    }

    #[tokio::test]
    async fn publish_emits_to_the_owner_and_updates_the_snapshot() {
        let sink = Arc::new(Recording::default());
        let reg = registry(sink.clone());

        reg.publish(
            "aa:bb",
            Some("companion-1"),
            RobotPhase::Listening,
            1_700_000_000_000,
        )
        .await;

        let sent = sink.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "owner-1");
        assert_eq!(sent[0].1.name, "robot.status");
        let payload: RobotStatusDto = serde_json::from_value(sent[0].1.data.clone()).unwrap();
        assert_eq!(payload.robot_id, "aa:bb");
        assert_eq!(payload.companion_id.as_deref(), Some("companion-1"));
        assert_eq!(payload.phase, "listening");
        assert_eq!(payload.changed_at, 1_700_000_000_000);
        drop(sent);

        let snap = reg.snapshot().await;
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].phase, "listening");
    }

    #[tokio::test]
    async fn repeated_identical_phase_does_not_re_emit() {
        let sink = Arc::new(Recording::default());
        let reg = registry(sink.clone());
        reg.publish("aa:bb", None, RobotPhase::Idle, 1).await;
        reg.publish("aa:bb", None, RobotPhase::Idle, 2).await;
        assert_eq!(
            sink.sent.lock().unwrap().len(),
            1,
            "same phase is not news"
        );
        assert_eq!(
            reg.snapshot().await[0].changed_at,
            1,
            "changed_at keeps the first transition"
        );
    }

    #[tokio::test]
    async fn mark_offline_transitions_and_emits() {
        let sink = Arc::new(Recording::default());
        let reg = registry(sink.clone());
        reg.publish("aa:bb", Some("c1"), RobotPhase::Speaking, 1).await;
        reg.mark_offline("aa:bb", 5).await;

        let snap = reg.snapshot().await;
        assert_eq!(snap[0].phase, "offline");
        assert_eq!(snap[0].changed_at, 5);
        assert_eq!(
            snap[0].companion_id.as_deref(),
            Some("c1"),
            "binding survives going offline"
        );
        assert_eq!(sink.sent.lock().unwrap().len(), 2);
    }
}
