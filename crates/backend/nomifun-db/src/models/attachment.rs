use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

/// Row in the `attachments` table — requirement images. The former generic
/// (kind, target_id) polymorphism is collapsed to a real requirement_id FK
/// (only the requirement kind was ever used). id stays a string `att_` because
/// it rides the requirement DTO into the owning Agent's ACP transcript.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AttachmentRow {
    pub id: String,
    pub requirement_id: String,
    /// Original display name, deduped per requirement (`name(2).ext`).
    pub file_name: String,
    /// Path relative to the data dir, e.g. `attachments/{requirement_id}/{id}.png`.
    /// Stored relative so desktop data-dir relocation never has to rewrite it.
    pub rel_path: String,
    pub mime: String,
    pub size_bytes: i64,
    pub created_by: Option<String>,
    pub created_at: TimestampMs,
}
