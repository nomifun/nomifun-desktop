use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Row mapping for the `conversations` table.
///
/// Enum-like fields (`type`, `status`, `source`) are stored as TEXT strings.
/// The service layer converts them to/from `nomifun_common` enums
/// (`AgentType`, `ConversationStatus`, `ConversationSource`).
///
/// JSON fields (`extra`, `model`) are stored as TEXT in SQLite and
/// deserialized by the service layer.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ConversationRow {
    pub id: i64,
    pub user_id: String,
    pub name: String,
    /// Agent type string (e.g. "gemini", "acp", "remote").
    #[sqlx(rename = "type")]
    pub r#type: String,
    /// JSON object: type-specific extra data.
    pub extra: String,
    /// JSON object: `ProviderWithModel` serialized.
    pub model: Option<String>,
    /// One of: "pending", "running", "finished". NULL in legacy rows.
    pub status: Option<String>,
    /// One of: "nomifun", "telegram", "lark", "dingtalk", "weixin".
    pub source: Option<String>,
    /// Channel isolation ID (e.g. "user:xxx", "group:xxx").
    pub channel_chat_id: Option<String>,
    /// Whether this conversation is pinned (SQLite INTEGER 0/1).
    pub pinned: bool,
    pub pinned_at: Option<TimestampMs>,
    /// The cron job that created this conversation (was `extra.cronJobId`;
    /// now a real nullable FK column to `cron_jobs`).
    pub cron_job_id: Option<String>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}
