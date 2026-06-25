use std::sync::Arc;

use nomifun_common::{AppError, Confirmation};
use serde_json::{Value, json};
use tracing::warn;

use crate::shared_kernel::approval_key;

use super::OpenClawAgentManager;

/// OpenClaw-specific operations reached through `AgentInstance::OpenClaw(..)`
/// matches in the routes + services (e.g. `persist_session_key` uses
/// `get_session_key`, and `get_openclaw_runtime` calls `get_diagnostics`).
impl OpenClawAgentManager {
    pub fn confirm(&self, _msg_id: &str, call_id: &str, _data: Value, always_allow: bool) -> Result<(), AppError> {
        if let Ok(mut state) = self.state.try_write() {
            if always_allow && let Some(conf) = state.confirmations.iter().find(|c| c.call_id == call_id) {
                let key = approval_key(conf.action.as_deref(), conf.command_type.as_deref());
                state.approval_memory.insert(key, true);
            }
            state.confirmations.retain(|c| c.call_id != call_id);
        }

        let connection = Arc::clone(&self.connection);
        let call_id = call_id.to_owned();
        let option_id = if always_allow { "allow_always" } else { "allow_once" };
        let option_id = option_id.to_owned();
        tokio::spawn(async move {
            let params = json!({
                "requestId": call_id,
                "optionId": option_id,
            });
            if let Err(e) = connection.request::<Value>("exec.approval.respond", params).await {
                warn!(error = %e, "Failed to send OpenClaw approval response");
            }
        });

        Ok(())
    }

    pub fn get_confirmations(&self) -> Vec<Confirmation> {
        self.state
            .try_read()
            .map(|g| g.confirmations.clone())
            .unwrap_or_default()
    }

    pub fn check_approval(&self, action: &str, command_type: Option<&str>) -> bool {
        self.state
            .try_read()
            .map(|g| {
                let key = approval_key(Some(action), command_type);
                g.approval_memory.get(&key).copied().unwrap_or(false)
            })
            .unwrap_or(false)
    }

    pub fn get_session_key(&self) -> Option<String> {
        self.state.try_read().ok().and_then(|g| g.session_key.clone())
    }
}
