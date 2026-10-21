//! User-authored delivery intent. Admission and native delivery remain Host facts.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use crate::EventId;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub enum SessionInputDeliveryIntent {
    Queue {},
    PreferCurrent {
        /// The source message of the canonical Turn observed at submission.
        #[ts(type = "string")]
        #[schemars(with = "String")]
        turn_id: EventId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub enum SessionInputQueueAction {
    Pause {},
    Resume {},
    Remove { #[ts(type = "string")] #[schemars(with = "String")] message_id: EventId },
    Clear {},
    Reorder { #[ts(type = "string[]")] #[schemars(with = "Vec<String>")] message_ids: Vec<EventId> },
    Promote {
        #[ts(type = "string")] #[schemars(with = "String")] message_id: EventId,
        #[ts(type = "string")] #[schemars(with = "String")] turn_id: EventId,
    },
}

/// Reference-only changes to the ordinary composer inbox. Message bodies stay
/// in their original accepted input, while all readers fold these same facts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionInputQueueChangedV1 {
    pub action: SessionInputQueueAction,
    pub expected_version: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub through_seq: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn delivery_and_scope_wide_queue_actions_reject_extra_fields() {
        assert_eq!(serde_json::to_value(SessionInputDeliveryIntent::Queue {}).unwrap(),json!({"kind":"queue"}));
        for delivery in [json!({"kind":"queue","turn_id":"unrelated"}),json!({"kind":"queue","content":"hidden input"}),json!({"kind":"automatic"})] {
            assert!(serde_json::from_value::<SessionInputDeliveryIntent>(delivery).is_err());
        }
        for kind in ["pause","resume","clear"] {
            assert!(serde_json::from_value::<SessionInputQueueAction>(json!({"kind":kind})).is_ok());
            assert!(serde_json::from_value::<SessionInputQueueAction>(json!({"kind":kind,"message_id":"only-one-input"})).is_err(),
                "a single-input-looking request must not silently become a whole-queue operation");
        }
    }

    #[test]
    fn reference_only_controls_reject_hidden_body_and_conflicting_targets() {
        assert!(serde_json::from_value::<SessionInputQueueAction>(json!({"kind":"remove","message_id":"one","turn_id":"other"})).is_err());
        assert!(serde_json::from_value::<SessionInputQueueAction>(json!({"kind":"promote","message_id":"one","turn_id":"observed","content":"another body"})).is_err());
        assert!(serde_json::from_value::<SessionInputDeliveryIntent>(json!({"kind":"prefer_current","turn_id":"observed","queue":true})).is_err());
    }
}
