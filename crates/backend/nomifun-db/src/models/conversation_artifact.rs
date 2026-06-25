use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Row mapping for the `conversation_artifacts` table.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ConversationArtifactRow {
    pub id: i64,
    pub conversation_id: i64,
    pub cron_job_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub payload: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}
