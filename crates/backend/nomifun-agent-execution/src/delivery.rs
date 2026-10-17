//! Stable delivery boundary for Agent Execution.
//!
//! The application session owner maps its conversation-service receipt into
//! this DTO. Agent Execution consumes only this boundary and never owns or
//! constructs a second conversation session.

use serde::{Deserialize, Serialize};

/// Host-authored provenance for one product-directed delegation. These are
/// references to the original canonical facts, never another task ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectedDelegationSource {
    pub agent_session_id: String,
    pub turn_operation_id: String,
    pub tool_operation_id: String,
    pub effect_id: String,
}

/// Exact settled Attempt references carried with a directed execution report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectedDelegationAttemptReport {
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
pub struct DirectedDelegationReport {
    pub source: DirectedDelegationSource,
    pub execution_id: String,
    pub execution_event_sequence: i64,
    pub status: String,
    pub attempts: Vec<DirectedDelegationAttemptReport>,
}

/// An existing DecisionRequested event is the durable input-notice outbox.
/// Question text and presentation versions are read from that exact owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectedDelegationInput {
    pub source: DirectedDelegationSource,
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
    /// A normal handoff on a completed Turn; it does not complete the task.
    pub user_input_request: Option<nomifun_agent_contracts::AgentUserInputRequest>,
    /// Exact operation's canonical nonterminal pause. A pause is not a
    /// delivery receipt or permission to retry the task in a new Session.
    pub paused_reason: Option<String>,
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
