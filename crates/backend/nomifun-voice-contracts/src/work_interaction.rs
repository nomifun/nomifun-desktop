//! Exact, durable work interaction identities. Media has no execution authority.
use serde::{Deserialize, Serialize};
use schemars::JsonSchema;
use crate::{AgentSessionId, OperationId};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkTarget {
    pub agent_session_id: AgentSessionId,
    pub binding_version: u64,
    pub turn_operation_id: OperationId,
    pub execution_generation: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkSteeringPolicy {
    #[default]
    SafeBoundary,
    SupersedeModelStep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkInteractionStatus {
    Accepted,
    Applied,
    PendingBoundary,
    Rejected,
    Deferred,
    Terminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all="snake_case")]
pub enum ApprovalInteractionMode { VoiceAllowed, ExplicitClick }

/// Immutable presentation of an already-pending canonical user decision.
/// It cannot grant Actions absent from the Agent's existing authorization.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApprovalTarget {
    pub work_target: WorkTarget,
    pub interaction_agent_session_id: AgentSessionId,
    pub interaction_binding_version: u64,
    pub execution_id: String,
    pub step_id: String,
    pub attempt_id: String,
    pub expected_execution_version: i64,
    pub expected_step_version: i64,
    pub expected_attempt_version: i64,
    pub request_event_sequence: i64,
    pub action_id: crate::ActionId,
    pub question_digest: crate::DigestHex,
    pub presentation_id: String,
    pub presentation_digest: crate::DigestHex,
    pub interaction_mode: ApprovalInteractionMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceApprovalAnswer {
    pub target: ApprovalTarget,
    pub answer: String,
    /// Application-proven presentation context; never guessed from "yes".
    pub presented_context_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceApprovalPresentation { pub target: ApprovalTarget, pub question: String }
