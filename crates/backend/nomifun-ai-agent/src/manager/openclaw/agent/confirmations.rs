use std::sync::Arc;

use nomifun_common::{AppError, Confirmation};
use serde_json::{Value, json};
use tracing::warn;

use crate::session::approval_key;

use super::OpenClawAgentManager;

/// OpenClaw-specific operations reached through `AgentRuntimeHandle::OpenClaw(..)`
/// matches in the routes + services (e.g. `persist_session_key` uses
/// `get_session_key`, and `get_openclaw_runtime` calls `get_diagnostics`).
impl OpenClawAgentManager {
    pub fn confirm(&self, _msg_id: &str, call_id: &str, data: Value, always_allow: bool) -> Result<(), AppError> {
        let request_id = match self.state.try_write() {
            Ok(mut state) => {
                let request_id = state
                    .confirmations
                    .iter()
                    .find(|confirmation| confirmation.call_id == call_id)
                    .map(|confirmation| confirmation.id.clone())
                    .ok_or_else(|| AppError::NotFound(format!("OpenClaw approval '{call_id}' not found")))?;
                if always_allow
                    && let Some(conf) = state.confirmations.iter().find(|c| c.call_id == call_id)
                {
                    let key = approval_key(conf.action.as_deref(), conf.command_type.as_deref());
                    state.approval_memory.insert(key, true);
                }
                state.confirmations.retain(|c| c.call_id != call_id);
                request_id
            }
            Err(_) => return Err(AppError::Conflict("OpenClaw approval state is busy".into())),
        };

        let connection = Arc::clone(&self.connection);
        let decision = confirmation_option_id(&data)
            .map(|decision| normalize_approval_decision(&decision))
            .unwrap_or_else(|| if always_allow { "allow-always" } else { "allow-once" }.to_owned());
        tokio::spawn(async move {
            let params = json!({
                "id": request_id,
                "decision": decision,
            });
            if let Err(e) = connection.request::<Value>("exec.approval.resolve", params).await {
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

fn confirmation_option_id(data: &Value) -> Option<String> {
    match data {
        Value::String(value) => Some(value.clone()),
        Value::Object(map) => map
            .get("option_id")
            .or_else(|| map.get("optionId"))
            .or_else(|| map.get("value"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        _ => None,
    }
}

fn normalize_approval_decision(value: &str) -> String {
    match value {
        "allow_once" | "proceed_once" => "allow-once".to_owned(),
        "allow_always" | "proceed_always" | "proceed_always_server" | "proceed_always_tool" => {
            "allow-always".to_owned()
        }
        "deny_once" | "reject" | "cancel" => "deny".to_owned(),
        other => other.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_frontend_permission_values() {
        assert_eq!(
            confirmation_option_id(&json!({ "value": "proceed_once" })).as_deref(),
            Some("proceed_once")
        );
        assert_eq!(normalize_approval_decision("proceed_once"), "allow-once");
        assert_eq!(normalize_approval_decision("proceed_always"), "allow-always");
        assert_eq!(normalize_approval_decision("cancel"), "deny");
    }
}
