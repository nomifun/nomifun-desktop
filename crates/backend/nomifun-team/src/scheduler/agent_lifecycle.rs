use tracing::debug;

use super::{AgentSlot, TeammateManager};
use crate::error::TeamError;
use crate::types::{TeamAgent, TeammateStatus};

impl TeammateManager {
    pub async fn add_agent(&self, agent: &TeamAgent) {
        let mut slots = self.slots.lock().await;
        slots.insert(
            agent.slot_id.clone(),
            AgentSlot {
                agent: agent.clone(),
                status: TeammateStatus::Idle,
                needs_role_prompt: true,
            },
        );
        self.events.broadcast_agent_spawned(agent);
        debug!(
            team_id = %self.team_id,
            slot_id = %agent.slot_id,
            name = %agent.name,
            "agent added to scheduler"
        );
    }

    pub async fn remove_agent(&self, slot_id: &str) -> Result<Option<String>, TeamError> {
        let mut slots = self.slots.lock().await;
        let removed = slots
            .remove(slot_id)
            .ok_or_else(|| TeamError::AgentNotFound(slot_id.to_owned()))?;
        let conversation_id = removed.agent.conversation_id.clone();
        drop(slots);
        self.clear_agent_state(slot_id, &conversation_id);
        self.events.broadcast_agent_removed(slot_id);
        debug!(team_id = %self.team_id, slot_id, "agent removed from scheduler");
        Ok(Some(conversation_id))
    }

    pub fn notify_shutdown_acknowledged(&self, slot_id: &str) {
        self.events.broadcast_agent_shutdown(slot_id);
        debug!(team_id = %self.team_id, slot_id, "agent shutdown acknowledged");
    }

    pub fn clear_agent_state(&self, slot_id: &str, conversation_id: &str) {
        self.active_wakes.remove(slot_id);
        self.clear_wake_timeout(slot_id);
        self.finalized_turns.remove(conversation_id);
    }

    pub async fn rename_agent(&self, slot_id: &str, new_name: &str) -> Result<(), TeamError> {
        let normalized = super::normalize_name(new_name);
        if normalized.is_empty() {
            return Err(TeamError::InvalidRequest(
                "rename_agent.new_name is empty after normalization".into(),
            ));
        }

        let mut slots = self.slots.lock().await;
        let _target = slots
            .get(slot_id)
            .ok_or_else(|| TeamError::AgentNotFound(slot_id.to_owned()))?;

        // Check all other agents for name collision (exclude self).
        let conflict = slots
            .iter()
            .any(|(id, s)| id != slot_id && super::normalize_name(&s.agent.name) == normalized);
        if conflict {
            return Err(TeamError::DuplicateAgentName(new_name.to_owned()));
        }

        let slot = slots.get_mut(slot_id).unwrap();
        slot.agent.name = new_name.to_owned();
        drop(slots);
        self.events.broadcast_agent_renamed(slot_id, new_name);
        debug!(team_id = %self.team_id, slot_id, new_name, "agent renamed");
        Ok(())
    }
}
