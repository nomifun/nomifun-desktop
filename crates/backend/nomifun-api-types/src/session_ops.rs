//! Session-scoped conversation operations exposed over HTTP.
//!
//! These DTOs back the per-conversation routes that read or mutate one live
//! agent session: its model info, workspace listing, and side-question channel.
//! They are engine-neutral — the enclosing routes
//! dispatch through `AgentRuntimeHandle`.

use serde::{Deserialize, Serialize};

/// Request body for setting the session model.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetModelRequest {
    pub model_id: String,
}

/// A single available model entry in the frontend-facing model info response.
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfoEntry {
    pub id: String,
    pub label: String,
}

/// Frontend-compatible model info response.
///
/// `model_info` is `None` for a session whose model is fixed by the
/// conversation's provider binding rather than chosen per session; the UI then
/// hides the in-session model picker instead of showing an error.
#[derive(Debug, Serialize)]
pub struct GetModelInfoResponse {
    pub model_info: Option<ModelInfoPayload>,
}

/// Inner model info payload: the session's current model plus what it may
/// switch to.
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfoPayload {
    pub current_model_id: Option<String>,
    pub current_model_label: Option<String>,
    pub available_models: Vec<ModelInfoEntry>,
}

/// Query parameters for workspace browse.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceBrowseQuery {
    pub path: String,
    pub search: Option<String>,
}

/// A file or directory entry in the workspace browse response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub entry_type: String,
}

/// Request body for side question.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SideQuestionRequest {
    pub question: String,
}

/// Response for side question.
#[derive(Debug, Serialize)]
pub struct SideQuestionResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn set_model_request_serde() {
        let json = json!({ "model_id": "claude-sonnet-4" });
        let req: SetModelRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.model_id, "claude-sonnet-4");
    }

    #[test]
    fn workspace_entry_type_is_wire_named_type() {
        let entry = WorkspaceEntry {
            name: "src".into(),
            entry_type: "directory".into(),
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["type"], "directory");
        assert!(json.get("entry_type").is_none());
    }

    #[test]
    fn side_question_response_omits_absent_answer() {
        let resp = SideQuestionResponse {
            status: "unsupported".into(),
            answer: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["status"], "unsupported");
        assert!(json.get("answer").is_none());
    }
}
