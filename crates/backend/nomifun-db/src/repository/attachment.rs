use crate::error::DbError;
use crate::models::AttachmentRow;

/// Data access abstraction for the `attachments` table (requirement images).
#[async_trait::async_trait]
pub trait IAttachmentRepository: Send + Sync {
    async fn insert(&self, row: &AttachmentRow) -> Result<(), DbError>;

    async fn get_by_id(&self, id: &str) -> Result<Option<AttachmentRow>, DbError>;

    /// All attachments for a requirement, oldest first.
    async fn list_for_requirement(&self, requirement_id: i64) -> Result<Vec<AttachmentRow>, DbError>;

    /// Delete by id. Returns whether a row was deleted (absent id is not an
    /// error — callers do best-effort cleanup).
    async fn delete(&self, id: &str) -> Result<bool, DbError>;
}
