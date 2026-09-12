use crate::error::DbError;
use crate::models::{TagSettingPatch, TagSettingRow};

/// Data access abstraction for the `tag_settings` table — per-tag config
/// (bound webhook + description) layered over the implicit requirement tags.
#[async_trait::async_trait]
pub trait ITagSettingRepository: Send + Sync {
    /// Return the settings row for `tag`, or `None` if none was ever written.
    async fn get(&self, tag: &str) -> Result<Option<TagSettingRow>, DbError>;

    /// Atomically insert/update supplied fields and return the persisted row.
    /// A supplied webhook binding is checked in the same transaction.
    async fn upsert(&self, tag: &str, patch: &TagSettingPatch) -> Result<TagSettingRow, DbError>;

    /// Return all tag settings rows.
    async fn list_all(&self) -> Result<Vec<TagSettingRow>, DbError>;

    /// Delete the settings for `tag`. Idempotent (absent tag is not an error).
    async fn delete(&self, tag: &str) -> Result<(), DbError>;
}
