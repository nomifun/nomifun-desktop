use crate::error::DbError;
use crate::models::SystemSettings;

/// System settings data access abstraction.
///
/// The `system_settings` table holds one logical singleton (`singleton_key = system`).
/// `get_settings` returns `None` if no row exists yet (caller uses defaults).
/// `upsert_settings` atomically updates supplied fields, using defaults on insert.
#[async_trait::async_trait]
pub trait ISettingsRepository: Send + Sync {
    /// Returns the settings row, or `None` if no settings have been persisted.
    async fn get_settings(&self) -> Result<Option<SystemSettings>, DbError>;

    /// Updates only `Some` fields. `None` preserves the current value or insert default.
    async fn upsert_settings(
        &self,
        language: Option<&str>,
        notification_enabled: Option<bool>,
        cron_notification_enabled: Option<bool>,
        command_queue_enabled: Option<bool>,
        save_upload_to_workspace: Option<bool>,
    ) -> Result<SystemSettings, DbError>;
}
