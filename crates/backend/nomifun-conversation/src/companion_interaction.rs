//! Trusted, per-turn device context. Never deserialize this from public JSON.
//!
//! A device connection is an endpoint of the Companion's Conversation, not a
//! Conversation or an Agent identity of its own.

use nomifun_api_types::SessionMcpServer;
use nomifun_common::{AppError, CompanionId, ProviderWithModel};
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub struct PreparedDesktopDeviceTurn {
    pub context: CompanionDeviceTurn,
    pub pre_send_hook: Arc<dyn crate::service::BackgroundTurnPreSendHook>,
}

#[async_trait::async_trait]
pub trait CompanionDesktopTurnProvider: Send + Sync {
    async fn prepare(&self, owner_id: &str, conversation_id: &str, request_id: &str)
        -> Result<Option<PreparedDesktopDeviceTurn>, AppError>;
}

/// Resolved by the authenticated device host for one admitted utterance.
#[derive(Clone)]
pub struct CompanionDeviceTurn {
    pub from_desktop: bool,
    /// Retain revocable transports and observation ownership until the model
    /// turn actually exits, including when its HTTP acknowledgement returned.
    pub resources: Option<Arc<dyn std::any::Any + Send + Sync>>,
    pub companion_id: String,
    pub robot_id: String,
    pub connection_id: String,
    pub request_id: String,
    pub agent_revision: Option<(String, i64)>,
    /// A device host may select the Companion's fallback for this turn only.
    pub model: Option<ProviderWithModel>,
    pub fallback_model: Option<ProviderWithModel>,
    /// Server-issued, revocable registrations; never persisted on Conversation.
    pub mcp_servers: Vec<SessionMcpServer>,
    pub system_prompt: String,
}

impl CompanionDeviceTurn {
    pub fn validate_revision(&self, preset_id: Option<&str>, revision: Option<i64>) -> Result<(), AppError> {
        let actual = preset_id.zip(revision).map(|(id, revision)| (id.to_owned(), revision));
        if actual != self.agent_revision {
            return Err(AppError::Conflict("Companion Agent changed before device turn admission".to_owned()));
        }
        Ok(())
    }
    /// Device identities participate in deduplication even when two endpoints
    /// accidentally submit the same utterance token.
    pub fn idempotency_key(&self) -> String {
        if self.from_desktop { return self.request_id.clone(); }
        let identity = serde_json::json!([
            self.companion_id, self.robot_id, self.connection_id, self.request_id,
        ]);
        format!("companion-device:{:x}", Sha256::digest(identity.to_string().as_bytes()))
    }

    pub fn validate_owner(&self, conversation_extra: &serde_json::Value) -> Result<(), AppError> {
        CompanionId::parse(&self.companion_id)
            .map_err(|error| AppError::BadRequest(format!("invalid Companion identity: {error}")))?;
        for (name, value) in [
            ("robot", self.robot_id.as_str()),
            ("connection", self.connection_id.as_str()),
            ("request", self.request_id.as_str()),
        ] {
            if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
                return Err(AppError::BadRequest(format!("invalid device {name} identity")));
            }
        }
        if conversation_extra.get("companion_session").and_then(serde_json::Value::as_bool) != Some(true)
            || conversation_extra.get("companion_id").and_then(serde_json::Value::as_str)
                != Some(self.companion_id.as_str())
        {
            return Err(AppError::Forbidden("device turn belongs to another Companion".to_owned()));
        }
        Ok(())
    }

    /// Non-secret provenance persisted with the message and echoed to the UI.
    pub fn provenance(&self) -> serde_json::Value {
        serde_json::json!({
            "kind": if self.from_desktop { "desktop" } else { "robot" },
            "robot_id": self.robot_id,
            "connection_id": self.connection_id,
            "request_id": self.request_id,
            "agent_revision": self.agent_revision,
            "input_modality": if self.from_desktop { "text" } else { "speech" },
            "output_mode": if self.from_desktop { "desktop" } else { "spoken" },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const COMPANION: &str = "0190f5fe-7c00-7a00-8000-0000000000aa";

    fn turn() -> CompanionDeviceTurn {
        CompanionDeviceTurn {
            from_desktop: false, resources: None,
            companion_id: COMPANION.to_owned(),
            robot_id: "aa:bb:cc:dd:ee:ff".to_owned(),
            connection_id: "connection-1".to_owned(),
            request_id: "utterance-1".to_owned(),
            agent_revision: None,
            model: None,
            fallback_model: None,
            mcp_servers: vec![],
            system_prompt: "private resolved persona".to_owned(),
        }
    }

    #[test]
    fn device_identity_cannot_address_another_companion_or_an_ordinary_chat() {
        let turn = turn();
        assert!(turn.validate_owner(&serde_json::json!({
            "companion_session": true, "companion_id": COMPANION,
        })).is_ok());
        assert!(turn.validate_owner(&serde_json::json!({
            "companion_session": true,
            "companion_id": "0190f5fe-7c00-7a00-8000-0000000000bb",
        })).is_err());
        assert!(turn.validate_owner(&serde_json::json!({"companion_id": COMPANION})).is_err());
    }

    #[test]
    fn incomplete_or_noncanonical_connection_identity_is_rejected() {
        let extra = serde_json::json!({"companion_session": true, "companion_id": COMPANION});
        for invalid in ["", " connection-1", "connection-1\n"] {
            let mut turn = turn();
            turn.connection_id = invalid.to_owned();
            assert!(turn.validate_owner(&extra).is_err());
        }
    }

    #[test]
    fn message_provenance_contains_no_model_credentials_tools_or_persona() {
        let value = turn().provenance();
        assert_eq!(value["robot_id"], "aa:bb:cc:dd:ee:ff");
        assert_eq!(value["request_id"], "utterance-1");
        assert!(value.get("system_prompt").is_none());
        assert!(value.get("mcp_servers").is_none());
        assert!(value.get("model").is_none());
    }

    #[test]
    fn identical_utterance_tokens_are_scoped_to_device_and_connection() {
        let first = turn();
        let mut second = first.clone();
        assert_eq!(first.idempotency_key(), second.idempotency_key());
        second.robot_id = "aa:bb:cc:dd:ee:00".to_owned();
        assert_ne!(first.idempotency_key(), second.idempotency_key());
        second = first.clone();
        second.connection_id = "connection-2".to_owned();
        assert_ne!(first.idempotency_key(), second.idempotency_key());
    }

    #[test]
    fn public_message_json_cannot_supply_trusted_device_context() {
        for key in ["companion_device_turn", "interaction", "robot_id", "reply_endpoint"] {
            let mut request = serde_json::json!({"content": "hello"});
            request[key] = turn().provenance();
            assert!(serde_json::from_value::<nomifun_api_types::SendMessageRequest>(request).is_err());
        }
    }

    #[test]
    fn a_prepared_device_turn_cannot_outlive_its_companion_agent_revision() {
        let mut context = turn();
        context.agent_revision = Some(("preset-1".to_owned(), 2));
        assert!(context.validate_revision(Some("preset-1"), Some(2)).is_ok());
        assert!(context.validate_revision(Some("preset-1"), Some(3)).is_err());
        assert!(context.validate_revision(Some("preset-2"), Some(2)).is_err());
        assert!(context.validate_revision(None, None).is_err());
    }
}
