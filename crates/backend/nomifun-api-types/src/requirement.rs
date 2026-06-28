use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Requirement status, serialized as a lowercase string matching the DB column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementStatus {
    Pending,
    InProgress,
    Done,
    Failed,
    Cancelled,
    /// The turn ended cleanly but the agent did NOT explicitly declare the
    /// requirement done or failed (via its completion tool / terminal marker).
    /// Rather than silently assuming success, the platform parks it here for a
    /// human to verify. Not claimable; not frozen (a human can move it on).
    NeedsReview,
}

impl RequirementStatus {
    /// DB string form (matches serde `snake_case`).
    pub fn as_db(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::NeedsReview => "needs_review",
        }
    }

    /// Parse from a DB string; unknown values map to `Pending`.
    pub fn from_db(s: &str) -> Self {
        match s {
            "in_progress" => Self::InProgress,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "needs_review" => Self::NeedsReview,
            _ => Self::Pending,
        }
    }
}

/// One attachment on a requirement (API view of an `attachments` row).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttachmentDto {
    pub id: String,
    pub file_name: String,
    pub mime: String,
    pub size_bytes: i64,
    pub created_at: TimestampMs,
    /// Absolute path resolved at read time (`data_dir` + `rel_path`) so the
    /// desktop frontend can render it via the image-base64 endpoint. Never
    /// persisted — the DB stores only the relative path.
    pub abs_path: String,
}

/// A freshly-uploaded file to bind as an attachment. `source_path` is the
/// absolute path returned by `POST /api/fs/upload` and MUST sit inside the
/// temp upload root (`<OS temp>/nomifun/`) — the service rejects anything else.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct NewAttachmentRef {
    pub source_path: String,
    pub file_name: String,
}

/// Requirement response object (the API view of a `RequirementRow`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Requirement {
    pub id: i64,
    pub title: String,
    pub content: String,
    pub tag: String,
    pub order_key: String,
    pub status: RequirementStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_note: Option<String>,
    /// Executing session id: a conversation OR a terminal, discriminated by
    /// `owner_kind`. Replaces the former `conversation_id` (which assumed a
    /// single conversation domain).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_session_id: Option<i64>,
    /// `'conversation'` | `'terminal'` | None (when unowned). Paired with
    /// `owner_session_id` — both null together, both set together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<TimestampMs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<TimestampMs>,
    pub attempt_count: i64,
    pub created_by: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
    /// Image attachments. Populated on get/create/update responses; list/board
    /// rows and claim/status events carry an empty list for leanness.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentDto>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateRequirementRequest {
    pub title: String,
    #[serde(default)]
    pub content: String,
    pub tag: String,
    #[serde(default)]
    pub order_key: Option<String>,
    #[serde(default)]
    pub status: Option<RequirementStatus>,
    /// 'user' (default) | 'agent'.
    #[serde(default)]
    pub created_by: Option<String>,
    /// Images to bind on create (uploaded beforehand via `POST /api/fs/upload`).
    #[serde(default)]
    pub attachments: Vec<NewAttachmentRef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateRequirementRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub order_key: Option<String>,
    #[serde(default)]
    pub status: Option<RequirementStatus>,
    #[serde(default)]
    pub completion_note: Option<String>,
    /// Images to add (uploaded beforehand via `POST /api/fs/upload`).
    #[serde(default)]
    pub add_attachments: Vec<NewAttachmentRef>,
    /// Existing attachment ids to remove (rows + files).
    #[serde(default)]
    pub remove_attachment_ids: Vec<String>,
}

/// Bulk delete by id (used by the list page's batch-delete action).
#[derive(Debug, Clone, Deserialize)]
pub struct BatchDeleteRequest {
    pub ids: Vec<i64>,
}

/// Result of a bulk delete.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BatchDeleteResponse {
    pub deleted: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ListRequirementsQuery {
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub status: Option<RequirementStatus>,
    #[serde(default)]
    pub conversation_id: Option<i64>,
    #[serde(default)]
    pub q: Option<String>,
    /// Sort column. Whitelisted server-side to `id | created_at | updated_at |
    /// status`; any other value falls back to the default queue order.
    #[serde(default)]
    pub order_by: Option<String>,
    /// Sort direction: `asc | desc` (default `desc` for an explicit `order_by`).
    #[serde(default)]
    pub order: Option<String>,
    #[serde(default)]
    pub page: Option<u32>,
    #[serde(default)]
    pub page_size: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClaimRequest {
    pub tag: String,
    pub conversation_id: i64,
    #[serde(default)]
    pub lease_ms: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateStatusRequest {
    pub status: RequirementStatus,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompleteRequest {
    #[serde(default)]
    pub completion_note: Option<String>,
}

/// Per-status counts for a single tag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TagSummary {
    pub tag: String,
    pub pending: i64,
    pub in_progress: i64,
    pub done: i64,
    pub failed: i64,
    pub cancelled: i64,
    #[serde(default)]
    pub needs_review: i64,
    pub total: i64,
    /// AutoWork is paused for this tag (a requirement exhausted its retries).
    /// While true, the orchestrator does not claim the tag's requirements until
    /// it is resumed. `#[serde(default)]` keeps older payloads parseable.
    #[serde(default)]
    pub paused: bool,
    /// Why the tag was paused (`requirement_failed` | `manual` | …), if paused.
    #[serde(default)]
    pub paused_reason: Option<String>,
}

/// Request body for `POST /api/requirements/tags/{tag}/resume`. Body is
/// optional; an empty body resumes without re-queuing anything.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ResumeTagRequest {
    /// Re-queue ALL failed requirements in the tag back to pending.
    #[serde(default)]
    pub requeue_failed: bool,
    /// Re-queue these specific failed requirement ids back to pending.
    #[serde(default)]
    pub requeue_ids: Vec<i64>,
}

/// Broadcast (`autowork.tagPaused`) when AutoWork pauses a tag after a
/// requirement exhausts its retries, so the UI can surface "needs attention".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TagPausedPayload {
    pub tag: String,
    pub reason: String,
    pub requirement_id: Option<i64>,
}

/// Kanban board for a tag: requirements grouped by status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BoardResponse {
    pub tag: String,
    pub pending: Vec<Requirement>,
    pub in_progress: Vec<Requirement>,
    pub done: Vec<Requirement>,
    pub failed: Vec<Requirement>,
    pub cancelled: Vec<Requirement>,
    #[serde(default)]
    pub needs_review: Vec<Requirement>,
}

/// What an AutoWork loop drives: a chat conversation's agent, or a terminal
/// session's PTY (running a vendor CLI). The `target_id` carries an explicit
/// domain prefix — `conv_*` for conversations, `term_*` for terminals — so the
/// two id-spaces are discriminable on sight and never collide. The kind is
/// still always carried explicitly (never sniffed from the prefix); ids with
/// any other prefix belong to neither domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoWorkTargetKind {
    #[default]
    Conversation,
    Terminal,
}

impl AutoWorkTargetKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Terminal => "terminal",
        }
    }

    /// Parse a path/segment value; `None` for anything unrecognized.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "conversation" => Some(Self::Conversation),
            "terminal" => Some(Self::Terminal),
            _ => None,
        }
    }
}

/// The display status of an AutoWork switch — drives the UI dot/colour.
/// `off` (not enabled) / `idle` (enabled, between or awaiting work) /
/// `active` (enabled, a requirement is in flight).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoWorkRunState {
    Off,
    Idle,
    Active,
}

fn default_conversation_kind() -> AutoWorkTargetKind {
    AutoWorkTargetKind::Conversation
}

#[derive(Debug, Clone, Deserialize)]
pub struct AutoWorkConfigRequest {
    /// Defaults to `conversation` for backward compatibility with older clients.
    #[serde(default = "default_conversation_kind")]
    pub kind: AutoWorkTargetKind,
    /// Session id handle. Accepts both a JSON integer (what the frontend sends —
    /// ids are numeric) and a JSON string; see
    /// [`crate::serde_util::deserialize_target_id`].
    #[serde(deserialize_with = "crate::serde_util::deserialize_target_id")]
    pub target_id: String,
    pub enabled: bool,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub max_requirements: Option<u32>,
    /// Set by the AutoWork admin (标签会话管理). When `true`, a disable request
    /// targeting an actively-executing session is rejected — the user must stop
    /// it from the session page so a live turn is not interrupted. Session-page
    /// toggles leave this unset (`false`) and may always disable.
    #[serde(default)]
    pub from_admin: bool,
}

/// Persisted-and-live AutoWork state for a session (conversation or terminal).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutoWorkState {
    pub kind: AutoWorkTargetKind,
    pub target_id: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub running: bool,
    pub run_state: AutoWorkRunState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_requirement_id: Option<String>,
    pub completed_count: u32,
}

impl AutoWorkState {
    /// Compute the tri-state display status from the persisted/live flags.
    pub fn run_state(enabled: bool, current_requirement_id: Option<&str>) -> AutoWorkRunState {
        if !enabled {
            AutoWorkRunState::Off
        } else if current_requirement_id.is_some() {
            AutoWorkRunState::Active
        } else {
            AutoWorkRunState::Idle
        }
    }
}

/// WS payload for `requirement.deleted`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RequirementDeletedPayload {
    pub id: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: enabling 会话→自动工作 POSTs `target_id` as a JSON NUMBER
    /// (the frontend models session ids numerically). The backend keeps it as a
    /// String handle; deserialization must accept the integer instead of
    /// rejecting it with "Failed to deserialize the JSON body into the target
    /// object" (the 400 testers hit).
    #[test]
    fn autowork_request_accepts_numeric_target_id() {
        // This is the exact body shape the frontend sends; the integer value of
        // `target_id` begins at column 36 — the column the failing 400 reported.
        let body = r#"{"kind":"conversation","target_id":12345,"enabled":true,"tag":"t"}"#;
        let req: AutoWorkConfigRequest = serde_json::from_str(body).expect("numeric target_id must deserialize");
        assert_eq!(req.target_id, "12345");
        assert_eq!(req.kind, AutoWorkTargetKind::Conversation);
        assert!(req.enabled);
        assert_eq!(req.tag.as_deref(), Some("t"));
    }

    /// A string `target_id` (forward-compatible / other clients) still works.
    #[test]
    fn autowork_request_accepts_string_target_id() {
        let body = r#"{"kind":"terminal","target_id":"term_7","enabled":false}"#;
        let req: AutoWorkConfigRequest = serde_json::from_str(body).expect("string target_id must deserialize");
        assert_eq!(req.target_id, "term_7");
        assert_eq!(req.kind, AutoWorkTargetKind::Terminal);
        assert!(!req.enabled);
    }

    /// `kind` still defaults to conversation when omitted (older clients), and a
    /// numeric target_id coexists with that default.
    #[test]
    fn autowork_request_defaults_kind_with_numeric_target_id() {
        let body = r#"{"target_id":42,"enabled":true}"#;
        let req: AutoWorkConfigRequest = serde_json::from_str(body).expect("must deserialize");
        assert_eq!(req.target_id, "42");
        assert_eq!(req.kind, AutoWorkTargetKind::Conversation);
    }
}
