use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq, Eq)]
pub struct CronJobRunRow {
    pub id: String,
    pub job_id: String,
    pub executed_at_ms: TimestampMs,
    pub status: String,
    pub created_at_ms: TimestampMs,
}
