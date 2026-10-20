//! Presentation references to original committed collaboration facts.
use serde::{Deserialize, Serialize};

/// Host-authored references to the original canonical delegation. Shared by
/// the domain writer and its read projection so their source contract cannot drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentExecutionSource {
    pub agent_session_id: String,
    pub turn_operation_id: String,
    pub tool_operation_id: String,
    pub effect_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationRecordSource {
    pub agent_session_id: String,
    pub turn_operation_id: String,
    pub source_message_id: String,
    pub tool_operation_id: String,
    pub tool_call_id: String,
    pub effect_id: String,
    pub tool_event_id: String,
    pub tool_event_seq: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationRecordAnchor {
    pub agent_session_id: String,
    pub event_id: String,
    pub seq: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationRecordKind { Delegated, Completed, Failed, Skipped }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationProcessRecord {
    pub id: String,
    pub kind: DelegationRecordKind,
    pub execution_id: String,
    pub execution_event_sequence: i64,
    pub step_id: String,
    pub attempt_id: Option<String>,
    pub title: String,
    /// Historical events without a captured parent cursor retain their exact
    /// source grouping, without inventing chronology across independent streams.
    pub source_scoped_order: bool,
    pub source: DelegationRecordSource,
    /// A cursor in the source Session, captured under the original domain writer.
    /// Execution sequences are never compared to this Session cursor.
    pub anchor: DelegationRecordAnchor,
}
