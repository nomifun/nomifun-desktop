use std::sync::Arc;

use nomifun_api_types::{
    TeamAgentRemovedPayload, TeamAgentRenamedPayload, TeamAgentShutdownPayload, TeamAgentSpawnedPayload,
    TeamAgentStatusPayload, WebSocketMessage,
};
use nomifun_realtime::EventBroadcaster;

use crate::types::{TeamAgent, TeammateStatus};

pub struct TeamEventEmitter {
    team_id: String,
    broadcaster: Arc<dyn EventBroadcaster>,
}

impl TeamEventEmitter {
    pub fn new(team_id: String, broadcaster: Arc<dyn EventBroadcaster>) -> Self {
        Self { team_id, broadcaster }
    }

    pub fn team_id(&self) -> &str {
        &self.team_id
    }

    pub fn broadcast_agent_status(&self, slot_id: &str, status: TeammateStatus) {
        let payload = TeamAgentStatusPayload {
            team_id: self.team_id.clone(),
            slot_id: slot_id.to_owned(),
            status: status.to_string(),
        };
        let event = WebSocketMessage::new(
            "team.agent.status",
            serde_json::to_value(payload).expect("serialize status payload"),
        );
        self.broadcaster.broadcast(event);
    }

    pub fn broadcast_agent_spawned(&self, agent: &TeamAgent) {
        let payload = TeamAgentSpawnedPayload {
            team_id: self.team_id.clone(),
            agent: agent.to_response(),
        };
        let event = WebSocketMessage::new(
            "team.agent.spawned",
            serde_json::to_value(payload).expect("serialize spawned payload"),
        );
        self.broadcaster.broadcast(event);
    }

    pub fn broadcast_agent_removed(&self, slot_id: &str) {
        let payload = TeamAgentRemovedPayload {
            team_id: self.team_id.clone(),
            slot_id: slot_id.to_owned(),
        };
        let event = WebSocketMessage::new(
            "team.agent.removed",
            serde_json::to_value(payload).expect("serialize removed payload"),
        );
        self.broadcaster.broadcast(event);
    }

    /// Emit `team.agent.shutdown` to signal that the named teammate has
    /// acknowledged a Lead-initiated shutdown request. The actual removal
    /// (and `team.agent.removed`) follows once the agent process is killed
    /// and scheduler state is cleared.
    pub fn broadcast_agent_shutdown(&self, slot_id: &str) {
        let payload = TeamAgentShutdownPayload {
            team_id: self.team_id.clone(),
            slot_id: slot_id.to_owned(),
        };
        let event = WebSocketMessage::new(
            "team.agent.shutdown",
            serde_json::to_value(payload).expect("serialize shutdown payload"),
        );
        self.broadcaster.broadcast(event);
    }

    pub fn broadcast_agent_renamed(&self, slot_id: &str, name: &str) {
        let payload = TeamAgentRenamedPayload {
            team_id: self.team_id.clone(),
            slot_id: slot_id.to_owned(),
            name: name.to_owned(),
        };
        let event = WebSocketMessage::new(
            "team.agent.renamed",
            serde_json::to_value(payload).expect("serialize renamed payload"),
        );
        self.broadcaster.broadcast(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TeammateRole;
    use nomifun_api_types::{
        TeamAgentRemovedPayload, TeamAgentRenamedPayload, TeamAgentShutdownPayload, TeamAgentSpawnedPayload,
        TeamAgentStatusPayload,
    };

    struct RecordingBroadcaster {
        events: std::sync::Mutex<Vec<WebSocketMessage<serde_json::Value>>>,
    }

    impl RecordingBroadcaster {
        fn new() -> Self {
            Self {
                events: std::sync::Mutex::new(vec![]),
            }
        }

        fn events(&self) -> Vec<WebSocketMessage<serde_json::Value>> {
            self.events.lock().unwrap().clone()
        }
    }

    impl EventBroadcaster for RecordingBroadcaster {
        fn broadcast(&self, event: WebSocketMessage<serde_json::Value>) {
            self.events.lock().unwrap().push(event);
        }
    }

    fn make_emitter() -> (TeamEventEmitter, Arc<RecordingBroadcaster>) {
        let bc = Arc::new(RecordingBroadcaster::new());
        let emitter = TeamEventEmitter::new("team-1".into(), bc.clone());
        (emitter, bc)
    }

    #[test]
    fn status_event_has_correct_shape() {
        let (emitter, bc) = make_emitter();
        emitter.broadcast_agent_status("slot-1", TeammateStatus::Working);

        let events = bc.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "team.agent.status");

        let payload: TeamAgentStatusPayload = serde_json::from_value(events[0].data.clone()).unwrap();
        assert_eq!(payload.team_id, "team-1");
        assert_eq!(payload.slot_id, "slot-1");
        assert_eq!(payload.status, "working");
    }

    #[test]
    fn spawned_event_has_correct_shape() {
        let (emitter, bc) = make_emitter();
        let agent = TeamAgent {
            slot_id: "slot-2".into(),
            name: "Worker".into(),
            role: TeammateRole::Teammate,
            conversation_id: "conv-2".into(),
            backend: "acp".into(),
            model: "claude".into(),
            custom_agent_id: None,
            status: Some(TeammateStatus::Idle),
            conversation_type: None,
            cli_path: None,
        };
        emitter.broadcast_agent_spawned(&agent);

        let events = bc.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "team.agent.spawned");

        let payload: TeamAgentSpawnedPayload = serde_json::from_value(events[0].data.clone()).unwrap();
        assert_eq!(payload.team_id, "team-1");
        assert_eq!(payload.agent.slot_id, "slot-2");
        assert_eq!(payload.agent.name, "Worker");
        assert_eq!(payload.agent.role, "teammate");
    }

    #[test]
    fn removed_event_has_correct_shape() {
        let (emitter, bc) = make_emitter();
        emitter.broadcast_agent_removed("slot-3");

        let events = bc.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "team.agent.removed");

        let payload: TeamAgentRemovedPayload = serde_json::from_value(events[0].data.clone()).unwrap();
        assert_eq!(payload.team_id, "team-1");
        assert_eq!(payload.slot_id, "slot-3");
    }

    #[test]
    fn shutdown_event_has_correct_shape() {
        let (emitter, bc) = make_emitter();
        emitter.broadcast_agent_shutdown("slot-9");

        let events = bc.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "team.agent.shutdown");

        let payload: TeamAgentShutdownPayload = serde_json::from_value(events[0].data.clone()).unwrap();
        assert_eq!(payload.team_id, "team-1");
        assert_eq!(payload.slot_id, "slot-9");
    }

    #[test]
    fn renamed_event_has_correct_shape() {
        let (emitter, bc) = make_emitter();
        emitter.broadcast_agent_renamed("slot-1", "New Name");

        let events = bc.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "team.agent.renamed");

        let payload: TeamAgentRenamedPayload = serde_json::from_value(events[0].data.clone()).unwrap();
        assert_eq!(payload.team_id, "team-1");
        assert_eq!(payload.slot_id, "slot-1");
        assert_eq!(payload.name, "New Name");
    }

    #[test]
    fn team_id_accessor() {
        let (emitter, _) = make_emitter();
        assert_eq!(emitter.team_id(), "team-1");
    }

    #[test]
    fn multiple_events_accumulate() {
        let (emitter, bc) = make_emitter();
        emitter.broadcast_agent_status("s1", TeammateStatus::Working);
        emitter.broadcast_agent_status("s1", TeammateStatus::Idle);
        emitter.broadcast_agent_removed("s2");

        let events = bc.events();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].name, "team.agent.status");
        assert_eq!(events[1].name, "team.agent.status");
        assert_eq!(events[2].name, "team.agent.removed");
    }

    #[test]
    fn all_status_variants_serialize() {
        let (emitter, bc) = make_emitter();
        let statuses = [
            TeammateStatus::Idle,
            TeammateStatus::Working,
            TeammateStatus::Thinking,
            TeammateStatus::ToolUse,
            TeammateStatus::Completed,
            TeammateStatus::Error,
        ];
        for s in statuses {
            emitter.broadcast_agent_status("s1", s);
        }

        let events = bc.events();
        assert_eq!(events.len(), 6);
        let expected = ["idle", "working", "thinking", "tool_use", "completed", "error"];
        for (event, exp) in events.iter().zip(expected.iter()) {
            let payload: TeamAgentStatusPayload = serde_json::from_value(event.data.clone()).unwrap();
            assert_eq!(payload.status, *exp);
        }
    }
}
