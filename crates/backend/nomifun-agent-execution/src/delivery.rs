//! Stable delivery boundary for Agent Execution.
//!
//! The application session owner maps its conversation-service receipt into
//! this DTO. Agent Execution consumes only this boundary and never owns or
//! constructs a second conversation session.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentExecutionDelivery {
    pub message_id: String,
    /// `false` only for the atomic INSERT winner that was admitted to execute.
    /// Existing accepted/completed receipts and same-boot in-flight followers
    /// are absorbing replays and must not be awaited as newly-started work.
    pub replayed: bool,
    pub completed: bool,
    pub result_ok: Option<bool>,
    pub result_text: Option<String>,
    pub result_error: Option<String>,
    /// Stable snake_case terminal error token, or `None` before completion.
    pub result_error_code: Option<String>,
    /// Whether the terminal failure is safe to retry automatically.
    pub result_error_retryable: Option<bool>,
}
