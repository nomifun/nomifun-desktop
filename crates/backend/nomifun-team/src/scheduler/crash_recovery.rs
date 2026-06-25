use super::TeammateManager;
use crate::crash_detection::CrashReason;
use crate::error::TeamError;
use crate::types::{MailboxMessageType, TeammateRole, TeammateStatus};

pub fn format_crash_testament(agent_name: &str, reason: &CrashReason, last_message: Option<&str>) -> String {
    let reason_str = match reason {
        CrashReason::ProcessExited => "ProcessExited",
        CrashReason::SessionNotFound => "SessionNotFound",
        CrashReason::Unknown(msg) => return format_with_unknown(agent_name, msg, last_message),
    };
    if let Some(msg) = last_message {
        format!(
            "Teammate '{}' crashed during task (reason: {}). Last message: {}. Please investigate.",
            agent_name, reason_str, msg
        )
    } else {
        format!(
            "Teammate '{}' crashed during task (reason: {}). Please investigate.",
            agent_name, reason_str
        )
    }
}

fn format_with_unknown(agent_name: &str, reason_msg: &str, last_message: Option<&str>) -> String {
    if let Some(msg) = last_message {
        format!(
            "Teammate '{}' crashed during task (reason: Unknown — {}). Last message: {}. Please investigate.",
            agent_name, reason_msg, msg
        )
    } else {
        format!(
            "Teammate '{}' crashed during task (reason: Unknown — {}). Please investigate.",
            agent_name, reason_msg
        )
    }
}

impl TeammateManager {
    pub async fn handle_agent_crash(
        &self,
        slot_id: &str,
        reason: CrashReason,
        last_message: Option<&str>,
    ) -> Result<Option<String>, TeamError> {
        let (agent_name, is_lead) = {
            let slots = self.slots.lock().await;
            let slot = slots
                .get(slot_id)
                .ok_or_else(|| TeamError::AgentNotFound(slot_id.to_owned()))?;
            (slot.agent.name.clone(), slot.agent.role == TeammateRole::Lead)
        };

        self.write_crash_testament(slot_id, &agent_name, &reason, last_message)
            .await?;

        self.set_status(slot_id, TeammateStatus::Error).await?;

        self.release_wake_lock(slot_id);
        self.clear_wake_timeout(slot_id);

        if is_lead {
            return Ok(None);
        }
        Ok(self.find_lead_slot_id().await)
    }

    pub async fn handle_inactivity_timeout(&self, slot_id: &str) -> Result<Option<String>, TeamError> {
        let (agent_name, is_lead) = {
            let slots = self.slots.lock().await;
            let slot = slots
                .get(slot_id)
                .ok_or_else(|| TeamError::AgentNotFound(slot_id.to_owned()))?;
            (slot.agent.name.clone(), slot.agent.role == TeammateRole::Lead)
        };

        self.set_status(slot_id, TeammateStatus::Error).await?;
        self.release_wake_lock(slot_id);
        self.clear_wake_timeout(slot_id);

        if is_lead {
            return Ok(None);
        }

        let Some(lead_slot_id) = self.find_lead_slot_id().await else {
            return Ok(None);
        };
        let message = format!(
            "Teammate '{}' timed out after 60s of inactivity. Please investigate.",
            agent_name
        );
        self.mailbox
            .write(
                &self.team_id,
                &lead_slot_id,
                slot_id,
                MailboxMessageType::Message,
                &message,
                None,
            )
            .await?;
        Ok(Some(lead_slot_id))
    }

    pub async fn write_crash_testament(
        &self,
        slot_id: &str,
        agent_name: &str,
        reason: &CrashReason,
        last_message: Option<&str>,
    ) -> Result<(), TeamError> {
        let Some(lead_slot_id) = self.find_lead_slot_id().await else {
            return Ok(());
        };
        if lead_slot_id == slot_id {
            return Ok(());
        }
        let testament = format_crash_testament(agent_name, reason, last_message);
        self.mailbox
            .write(
                &self.team_id,
                &lead_slot_id,
                slot_id,
                MailboxMessageType::Message,
                &testament,
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn notify_shutdown_rejected(&self, from_slot_id: &str, reason: &str) -> Result<(), TeamError> {
        let Some(lead_slot_id) = self.find_lead_slot_id().await else {
            return Ok(());
        };
        if lead_slot_id == from_slot_id {
            return Ok(());
        }
        let agent_name = self
            .get_agent(from_slot_id)
            .await
            .map(|a| a.name)
            .unwrap_or_else(|_| from_slot_id.to_owned());
        let content = format!("Teammate '{agent_name}' declined shutdown: {reason}");
        self.mailbox
            .write(
                &self.team_id,
                &lead_slot_id,
                from_slot_id,
                MailboxMessageType::Message,
                &content,
                None,
            )
            .await?;
        Ok(())
    }
}
