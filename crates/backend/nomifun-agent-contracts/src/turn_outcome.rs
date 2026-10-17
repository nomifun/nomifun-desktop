//! Public result of an exact canonical Turn, derived from its committed facts.
//! Closing a Turn does not prove completion of the user's broader task.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub enum AgentTurnOutcome {
    /// Exact, nonempty, sealed public messages; never reasoning or tool output.
    Answered { message_ids: Vec<String> },
    /// This exact Turn intentionally has no public reply. Physical actions
    /// and their acceptance/completion remain owned by their original facts.
    Silent {},
    /// The question remains in the original typed user-input request.
    InputRequired { request_id: String },
    Failed { message_ids: Vec<String> },
    Cancelled { message_ids: Vec<String> },
    Paused { message_ids: Vec<String> },
    /// Closed history without a provable public answer or input handoff.
    Unavailable,
}

/// Shared identity for journal, stream, and read-only result derivation.
pub fn canonical_assistant_step_message_id(root_message_id: &str, step: u16) -> Result<String, String> {
    let root = uuid::Uuid::parse_str(root_message_id).map_err(|error| format!("turn root is not a UUID: {error}"))?;
    if root.get_version_num() != 7 || step == 0 { return Err("turn root is not UUIDv7 or step is zero".into()); }
    let mut bytes = *root.as_bytes();
    let suffix = (u16::from_be_bytes([bytes[14], bytes[15]]) ^ step).to_be_bytes();
    bytes[14] = suffix[0];
    bytes[15] = suffix[1];
    Ok(uuid::Uuid::from_bytes(bytes).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outcome_has_one_discriminated_meaning_and_no_copied_question() {
        let value = serde_json::json!({"kind":"input_required","request_id":"ask-name"});
        assert_eq!(serde_json::from_value::<AgentTurnOutcome>(value.clone()).unwrap(),
            AgentTurnOutcome::InputRequired { request_id:"ask-name".into() });
        let mut invalid = value;
        invalid["question"] = serde_json::json!("another fact source");
        assert!(serde_json::from_value::<AgentTurnOutcome>(invalid).is_err());
        assert_eq!(serde_json::from_value::<AgentTurnOutcome>(serde_json::json!({"kind":"silent"})).unwrap(),
            AgentTurnOutcome::Silent {});
        assert!(serde_json::from_value::<AgentTurnOutcome>(serde_json::json!({"kind":"silent","message_ids":[]})).is_err());
    }
}
