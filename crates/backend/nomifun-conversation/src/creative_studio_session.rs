//! Canonical Creative Studio AgentSession wire types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CreativeStudioAgentHistoryRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CreativeStudioAgentHistoryStatus {
    Complete,
    Running,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreativeStudioAgentHistoryMessage {
    pub id: String,
    pub role: CreativeStudioAgentHistoryRole,
    pub status: CreativeStudioAgentHistoryStatus,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreativeStudioAgentModelRef {
    pub provider_id: String,
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveCreativeStudioCanvasAgentSessionRequest {
    pub canvas_id: String,
    pub session_id: String,
    pub model: CreativeStudioAgentModelRef,
    pub pending_turn_idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreativeStudioCanvasAgentSessionBindingResponse {
    pub ownership: &'static str,
    pub canvas_id: String,
    pub session_id: String,
    pub conversation_id: String,
    pub model: CreativeStudioAgentModelRef,
    pub history_key: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolveCreativeStudioCanvasAgentSessionResponse {
    pub binding: CreativeStudioCanvasAgentSessionBindingResponse,
    pub history: Vec<CreativeStudioAgentHistoryMessage>,
    pub applied_proposal_message_ids: Vec<String>,
    pub created: bool,
}
