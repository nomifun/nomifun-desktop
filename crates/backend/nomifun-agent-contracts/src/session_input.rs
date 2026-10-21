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

/// Authoring commands carry replacement text only until the writer stores it
/// as a new accepted input. Queue facts themselves retain references only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub enum SessionInputQueueCommand {
    Replace {
        #[ts(type = "string")] #[schemars(with = "String")] message_id: EventId,
        content: String,
    },
    #[serde(untagged)]
    Control(SessionInputQueueAction),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionInputQueueChange {
    Replace { message_id: EventId, replacement_message_id: EventId },
    #[serde(untagged)]
    Control(SessionInputQueueAction),
}

impl From<SessionInputQueueAction> for SessionInputQueueChange {
    fn from(action: SessionInputQueueAction) -> Self { Self::Control(action) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub enum SessionInputPlacement { Queued, Steering, Transcript }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub enum SessionInputKind { Root, Steering }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub enum SessionInputState { Pending, Blocked, Claimed, Discarded, Cancelled }

/// A read projection of accepted input, claim and native delivery facts. This
/// is never written into an accepted payload or used as execution authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct SessionInputView {
    pub state: SessionInputState,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
    #[ts(type = "number")]
    pub event_seq: u64,
    pub placement: SessionInputPlacement,
    pub kind: SessionInputKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "string")] #[schemars(with = "Option<String>")]
    pub turn_id: Option<EventId>,
    #[ts(type = "number")]
    pub order_seq: u64,
}

/// Reference-only changes to the ordinary composer inbox. Message bodies stay
/// in their original accepted input, while all readers fold these same facts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionInputQueueChangedV1 {
    pub action: SessionInputQueueChange,
    pub expected_version: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub through_seq: Option<u64>,
    /// Explicit queue pause leaves visible starting/steering inputs alone.
    /// User Stop supplies no exemptions and fences its entire accepted batch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub except_message_ids: Vec<EventId>,
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

    #[test]
    fn authoring_commands_and_reference_only_replacements_have_distinct_strict_shapes() {
        let pause=json!({"kind":"pause"});
        let command:SessionInputQueueCommand=serde_json::from_value(pause.clone()).unwrap();
        assert_eq!(serde_json::to_value(command).unwrap(),pause);
        let replacement=json!({"kind":"replace","message_id":"original","content":"edited\ntext"});
        assert!(matches!(serde_json::from_value::<SessionInputQueueCommand>(replacement.clone()).unwrap(),SessionInputQueueCommand::Replace {..}));
        assert!(serde_json::from_value::<SessionInputQueueChange>(replacement).is_err());
        let reference=json!({"kind":"replace","message_id":"original","replacement_message_id":"replacement"});
        assert!(serde_json::from_value::<SessionInputQueueCommand>(reference.clone()).is_err());
        assert_eq!(serde_json::to_value(serde_json::from_value::<SessionInputQueueChange>(reference.clone()).unwrap()).unwrap(),reference);
        for value in [json!({"kind":"clear","message_id":"one"}),json!({"kind":"replace","message_id":"one","content":"edit","files":["injected"]})] {
            assert!(serde_json::from_value::<SessionInputQueueCommand>(value).is_err());
        }
    }
}
