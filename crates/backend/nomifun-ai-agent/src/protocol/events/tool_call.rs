use serde::{Deserialize, Serialize};

use crate::artifact_store::PersistedArtifact;
pub use nomifun_engine_core::ToolDisplayFacts;

/// Display projection of an admitted canonical action and its settled result,
/// never model-supplied authority or an independent permission fact.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolCallIdentity {
    #[serde(flatten, default)]
    pub display: nomifun_engine_core::ToolDisplayFacts,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_hold: Option<nomifun_agent_contracts::RuntimeToolHoldReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_required: Option<nomifun_engine_core::SystemPermissionRequired>,
    /// Exact canonical ToolCompleted (or cleanup settlement) event sequence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_seq: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallEventData {
    #[serde(flatten, default)]
    pub identity: ToolCallIdentity,
    pub call_id: String,
    pub name: String,
    pub status: ToolCallStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Verified user-visible outputs. Inline base64 is never placed on the
    /// event bus or in conversation history; only durable metadata is stored.
    // Keep an explicit empty array on Running/Error correction frames. Live
    // consumers merge lifecycle updates by call_id; omitting this field could
    // otherwise leave an earlier completed receipt visible after failure.
    #[serde(default)]
    pub artifacts: Vec<PersistedArtifact>,
}

/// Status of a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Running,
    Completed,
    Error,
    Canceled,
}

/// A single entry in a `ToolGroup` event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolGroupEntry {
    pub call_id: String,
    pub name: String,
    pub status: ToolCallStatus,
    #[serde(default)]
    pub description: Option<String>,
}
