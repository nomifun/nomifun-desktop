use crate::error::DbError;
use crate::models::TagSettingRow;

/// Data access abstraction for the `tag_settings` table — per-tag config
/// (bound webhook + description) layered over the implicit requirement tags.
#[async_trait::async_trait]
pub trait ITagSettingRepository: Send + Sync {
    /// Return the settings row for `tag`, or `None` if none was ever written.
    async fn get(&self, tag: &str) -> Result<Option<TagSettingRow>, DbError>;

    /// Insert-or-replace the settings for `tag` (keyed by tag name). Stamps
    /// `updated_at` at the call site (passed in via the row).
    async fn upsert(&self, row: &TagSettingRow) -> Result<(), DbError>;

    /// Return all tag settings rows.
    async fn list_all(&self) -> Result<Vec<TagSettingRow>, DbError>;

    /// Delete the settings for `tag`. Idempotent (absent tag is not an error).
    async fn delete(&self, tag: &str) -> Result<(), DbError>;
}
