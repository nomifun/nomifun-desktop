use crate::error::DbError;
use crate::models::SystemSettings;

/// System settings data access abstraction.
///
/// The `system_settings` table holds a single row (id=1).
/// `get_settings` returns `None` if no row exists yet (caller uses defaults).
/// `upsert_settings` inserts or replaces the single row.
#[async_trait::async_trait]
pub trait ISettingsRepository: Send + Sync {
    /// Returns the settings row, or `None` if no settings have been persisted.
    async fn get_settings(&self) -> Result<Option<SystemSettings>, DbError>;

    /// Inserts or replaces the single settings row.
    async fn upsert_settings(
        &self,
        language: &str,
        notification_enabled: bool,
        cron_notification_enabled: bool,
        command_queue_enabled: bool,
        save_upload_to_workspace: bool,
    ) -> Result<SystemSettings, DbError>;
}
