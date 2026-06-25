use crate::error::DbError;
use crate::models::WebhookRow;

/// Data access abstraction for the `webhooks` table.
///
/// Webhooks are global (not per-user) — they are an admin-managed pool of
/// outbound endpoints reused by features such as AutoWork completion
/// notifications.
#[async_trait::async_trait]
pub trait IWebhookRepository: Send + Sync {
    /// Insert a new webhook row. The `id` field of `row` is ignored — the
    /// `webhooks.id INTEGER PRIMARY KEY AUTOINCREMENT` column assigns it — and
    /// the DB-assigned id is returned.
    async fn insert(&self, row: &WebhookRow) -> Result<i64, DbError>;

    /// Replace the mutable columns (name/platform/url/secret/description/enabled/
    /// updated_at) of an existing webhook. Returns `DbError::NotFound` if absent.
    async fn update(&self, row: &WebhookRow) -> Result<(), DbError>;

    /// Delete a webhook by id. Returns `DbError::NotFound` if absent.
    async fn delete(&self, id: i64) -> Result<(), DbError>;

    /// Return a single webhook by id, or `None`.
    async fn get_by_id(&self, id: i64) -> Result<Option<WebhookRow>, DbError>;

    /// Return all webhooks ordered by creation time descending (newest first).
    async fn list_all(&self) -> Result<Vec<WebhookRow>, DbError>;
}
