//! Stable delivery boundary for Agent Execution.
//!
//! The application session owner maps its conversation-service receipt into
//! this DTO. Agent Execution consumes only this boundary and never owns or
//! constructs a second conversation session.

use serde::{Deserialize, Serialize};

pub use nomifun_common::AgentExecutionSource;

/// Missing receipt evidence stays absent. Explicit JSON null is an invalid
/// original invocation reference to the existing cleanup reader.
pub(crate) fn with_terminal_reference(mut payload: serde_json::Value,
    terminal: Option<(String,String)>) -> serde_json::Value {
    if let Some((operation,event))=terminal {
        payload["canonical_operation_id"]=operation.into();
        payload["terminal_event_id"]=event.into();
    }
    payload
}

#[cfg(test)]
mod terminal_reference_tests {
    use super::with_terminal_reference;
    #[test]
    fn optional_terminal_reference_preserves_the_original_cleanup_contract() {
        let payload=serde_json::json!({"turn_operation_key":"original-initial","step_status":"failed"});
        let absent=with_terminal_reference(payload.clone(),None);
        assert_eq!(absent,payload);
        assert!(absent.get("canonical_operation_id").is_none()&&absent.get("terminal_event_id").is_none());
        let exact=with_terminal_reference(payload,Some(("original-operation".into(),"original-terminal".into())));
        assert_eq!(exact["canonical_operation_id"],"original-operation");assert_eq!(exact["terminal_event_id"],"original-terminal");
        assert_eq!(exact["turn_operation_key"],"original-initial");
    }
}

/// Host-verified canonical human control input. The original typed event owns
/// the utterance and its captured target; this reference never mirrors text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentExecutionControlSource {
    pub agent_session_id: String,
    pub input_event_id: String,
}

/// The exact cancellation event acknowledged for one canonical control input.
/// A replay refers to that original event, even if the aggregate later reopens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentExecutionControlReceipt {
    pub source: AgentExecutionControlSource,
    pub execution_id: String,
    pub execution_event_sequence: i64,
    pub status: nomifun_common::AgentExecutionStatus,
    pub replayed: bool,
}

/// Exact settled Attempt references carried with an execution report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentExecutionAttemptReport {
    pub step_id: String,
    pub attempt_id: String,
    pub agent_session_id: String,
    pub turn_operation_id: String,
    pub terminal_event_id: String,
    pub result_ok: Option<bool>,
    pub output_integrity_ok: bool,
    pub output_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentExecutionReport {
    pub source: AgentExecutionSource,
    pub execution_id: String,
    pub execution_event_sequence: i64,
    pub status: String,
    pub attempts: Vec<AgentExecutionAttemptReport>,
}

/// An existing DecisionRequested event is the durable input-notice outbox.
/// Question text and presentation versions are read from that exact owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentExecutionInput {
    pub source: AgentExecutionSource,
    pub execution_id: String,
    pub request_event_sequence: i64,
    pub step_id: String,
    pub attempt_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentExecutionDelivery {
    pub message_id: String,
    /// `false` only for the atomic INSERT winner that was admitted to execute.
    /// Existing accepted/completed receipts and same-boot in-flight followers
    /// are absorbing replays and must not be awaited as newly-started work.
    pub replayed: bool,
    pub completed: bool,
    /// Public result derived from the exact canonical Turn. A closed lifecycle
    /// alone never establishes an answer, input handoff or task delivery.
    pub turn_outcome: Option<nomifun_agent_contracts::AgentTurnOutcome>,
    /// Exact question from a parked Turn or a normal closed handoff.
    pub user_input_request: Option<nomifun_agent_contracts::AgentUserInputRequest>,
    /// Terminal execution result. Suspension leaves this unset.
    pub result_ok: Option<bool>,
    pub result_text: Option<String>,
    pub result_error: Option<String>,
    /// Stable snake_case terminal error token, or `None` before completion.
    pub result_error_code: Option<String>,
    /// Whether the terminal failure is safe to retry automatically.
    pub result_error_retryable: Option<bool>,
}

/// Read-only output of one canonical Turn. Paths are verified against successful
/// owner receipts in that Turn; no Message projection or directory discovery
/// participates in this query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentExecutionTurnOutput {
    pub canonical_operation_id: String,
    pub terminal_event_id: Option<String>,
    pub delivery: AgentExecutionDelivery,
    pub output_files: Vec<String>,
    pub integrity_ok: bool,
}

/// Read-only progress of one exact canonical operation. The cursor is only a
/// wakeup hint; a terminal output must still pass the canonical output reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentExecutionTurnProgress {
    pub found: bool,
    pub terminal: bool,
    pub cursor: u64,
    pub suspended: bool,
}
