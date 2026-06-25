use tracing::debug;

use super::{TeammateManager, is_settled};
use crate::error::TeamError;
use crate::types::{MailboxMessageType, TeammateRole, TeammateStatus};

impl TeammateManager {
    pub async fn set_status(&self, slot_id: &str, status: TeammateStatus) -> Result<(), TeamError> {
        {
            let mut slots = self.slots.lock().await;
            let slot = slots
                .get_mut(slot_id)
                .ok_or_else(|| TeamError::AgentNotFound(slot_id.to_owned()))?;
            slot.status = status;
            slot.agent.status = Some(status);
        }
        self.events.broadcast_agent_status(slot_id, status);
        debug!(team_id = %self.team_id, slot_id, %status, "agent status changed");
        Ok(())
    }

    pub async fn get_status(&self, slot_id: &str) -> Result<TeammateStatus, TeamError> {
        let slots = self.slots.lock().await;
        let slot = slots
            .get(slot_id)
            .ok_or_else(|| TeamError::AgentNotFound(slot_id.to_owned()))?;
        Ok(slot.status)
    }

    pub async fn try_wake(&self, slot_id: &str) -> Result<Option<super::WakePayload>, TeamError> {
        let current = self.get_status(slot_id).await?;
        if current != TeammateStatus::Idle {
            debug!(
                team_id = %self.team_id,
                slot_id,
                current_status = %current,
                "skip wake: agent not idle"
            );
            return Ok(None);
        }
        self.set_status(slot_id, TeammateStatus::Working).await?;
        let payload = self.build_wake_payload(slot_id).await?;
        Ok(Some(payload))
    }

    pub async fn mark_idle(&self, slot_id: &str, summary: Option<&str>) -> Result<Option<String>, TeamError> {
        self.set_status(slot_id, TeammateStatus::Idle).await?;

        let is_lead = {
            let slots = self.slots.lock().await;
            let slot = slots
                .get(slot_id)
                .ok_or_else(|| TeamError::AgentNotFound(slot_id.to_owned()))?;
            slot.agent.role == TeammateRole::Lead
        };

        if is_lead {
            return Ok(None);
        }

        if let Some(lead_slot_id) = self.find_lead_slot_id().await
            && lead_slot_id != slot_id
        {
            self.mailbox
                .write(
                    &self.team_id,
                    &lead_slot_id,
                    slot_id,
                    MailboxMessageType::IdleNotification,
                    summary.unwrap_or("idle"),
                    summary,
                )
                .await?;
        }

        self.maybe_wake_leader_when_all_idle().await
    }

    pub async fn take_needs_role_prompt(&self, slot_id: &str) -> bool {
        let mut slots = self.slots.lock().await;
        if let Some(slot) = slots.get_mut(slot_id) {
            let needed = slot.needs_role_prompt;
            slot.needs_role_prompt = false;
            needed
        } else {
            false
        }
    }

    pub(crate) async fn maybe_wake_leader_when_all_idle(&self) -> Result<Option<String>, TeamError> {
        let slots = self.slots.lock().await;

        let mut lead_slot_id = None;
        let mut all_teammates_settled = true;
        let mut has_teammates = false;

        for slot in slots.values() {
            if slot.agent.role == TeammateRole::Lead {
                lead_slot_id = Some(slot.agent.slot_id.clone());
                continue;
            }
            has_teammates = true;
            if !is_settled(slot.status) {
                all_teammates_settled = false;
                break;
            }
        }

        let Some(lead_id) = lead_slot_id else {
            return Ok(None);
        };

        if !has_teammates {
            return Ok(None);
        }

        if !all_teammates_settled {
            return Ok(None);
        }

        let lead_is_idle = slots
            .get(&lead_id)
            .map(|s| s.status == TeammateStatus::Idle)
            .unwrap_or(false);

        if !lead_is_idle {
            return Ok(None);
        }

        drop(slots);

        debug!(
            team_id = %self.team_id,
            lead_slot_id = %lead_id,
            "all teammates settled — signaling to wake leader"
        );

        Ok(Some(lead_id))
    }
}
