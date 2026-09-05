//! IDMM-owned supervision primitives.
//!
//! The supervisor consumes this small contract instead of reaching into the
//! Conversation implementation. The application host adapter is responsible
//! for translating its runtime into these values.

use nomifun_common::{AppError, ProviderWithModel};
use nomifun_db::models::ConversationRow;

/// Typed admission hook consumed by the supervisor lifecycle.
///
/// The current host's Conversation hook is translated to this contract by the
/// application composition layer. The lifecycle manager therefore carries only
/// the IDMM-owned turn token.
pub trait SessionSupervisionPort: Send + Sync {
    fn admit_conversation_turn(&self, conversation_id: &str, scope: SupervisionTurnScope);
}

/// Stable identity of the exact turn an IDMM action is allowed to address.
///
/// This is deliberately owned by IDMM's supervision boundary. It is an
/// observation/admission token, not a runtime handle and not permission to
/// create a replacement turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupervisionTurnScope {
    pub(crate) wire_turn_id: String,
    pub(crate) generation: u64,
}

impl SupervisionTurnScope {
    /// Construct an exact-turn token at a trusted session adapter boundary.
    pub fn new(wire_turn_id: impl Into<String>, generation: u64) -> Self {
        Self {
            wire_turn_id: wire_turn_id.into(),
            generation,
        }
    }

    /// Stable wire identity of the admitted turn.
    pub fn wire_turn_id(&self) -> &str {
        &self.wire_turn_id
    }

    /// Monotonic generation of the admitted turn.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

/// Resolve the canonical provider/model selected on a Conversation row.
///
/// IDMM only needs this value as a fallback for its sidecar. Parsing remains
/// strict: malformed JSON, unknown fields, non-canonical provider IDs, and
/// untrimmed model names are persisted-state errors rather than an implicit
/// "no model" result.
pub(crate) fn conversation_fallback_model(
    row: &ConversationRow,
) -> Result<Option<(String, String)>, AppError> {
    row.model
        .as_deref()
        .map(parse_provider_model)
        .transpose()
}

fn parse_provider_model(value: &str) -> Result<(String, String), AppError> {
    let model: ProviderWithModel = serde_json::from_str(value)
        .map_err(|error| AppError::Internal(format!("invalid persisted conversation model: {error}")))?;
    model
        .validate()
        .map_err(|error| AppError::Internal(format!("invalid persisted conversation model: {error}")))?;
    Ok((model.provider_id, model.model))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_common::ConversationId;

    const PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";

    fn row_with_model(model: Option<&str>) -> ConversationRow {
        ConversationRow {
            id: 0,
            conversation_id: ConversationId::new().into_string(),
            user_id: "0190f5fe-7c00-7a00-8000-000000000002".into(),
            name: "test".into(),
            r#type: "nomi".into(),
            extra: "{}".into(),
            delegation_policy: "automatic".into(),
            execution_model_pool: None,
            decision_policy: "automatic".into(),
            execution_template_id: None,
            model: model.map(ToOwned::to_owned),
            status: Some("running".into()),
            source: None,
            channel_chat_id: None,
            pinned: false,
            pinned_at: None,
            cron_job_id: None,
            preset_id: None,
            preset_revision: None,
            preset_snapshot: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn resolves_only_the_canonical_session_model() {
        let row = row_with_model(Some(&format!(
            r#"{{"provider_id":"{PROVIDER_ID}","model":"gpt-5"}}"#
        )));
        assert_eq!(
            conversation_fallback_model(&row).unwrap(),
            Some((PROVIDER_ID.to_owned(), "gpt-5".to_owned()))
        );
    }

    #[test]
    fn rejects_legacy_or_invalid_session_model_shapes() {
        let legacy = row_with_model(Some(&format!(
            r#"{{"providerId":"{PROVIDER_ID}","model":"gpt-5"}}"#
        )));
        assert!(conversation_fallback_model(&legacy).is_err());

        let untrimmed = row_with_model(Some(&format!(
            r#"{{"provider_id":"{PROVIDER_ID}","model":" gpt-5"}}"#
        )));
        assert!(conversation_fallback_model(&untrimmed).is_err());
    }

    #[test]
    fn missing_session_model_is_not_an_error() {
        assert_eq!(conversation_fallback_model(&row_with_model(None)).unwrap(), None);
    }
}
