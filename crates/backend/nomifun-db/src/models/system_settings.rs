use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Row mapping for the `system_settings` table.
///
/// Single-row table (id is always 1). Boolean fields are stored as INTEGER
/// in SQLite (0/1) and mapped to `bool` via sqlx.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SystemSettings {
    pub id: i64,
    pub language: String,
    pub notification_enabled: bool,
    pub cron_notification_enabled: bool,
    pub command_queue_enabled: bool,
    pub save_upload_to_workspace: bool,
    pub updated_at: TimestampMs,
}
