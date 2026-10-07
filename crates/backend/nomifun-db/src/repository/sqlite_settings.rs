use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::SystemSettings;
use crate::repository::ISettingsRepository;

/// SQLite-backed implementation of [`ISettingsRepository`].
#[derive(Clone, Debug)]
pub struct SqliteSettingsRepository {
    pool: SqlitePool,
}

impl SqliteSettingsRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl ISettingsRepository for SqliteSettingsRepository {
    async fn get_settings(&self) -> Result<Option<SystemSettings>, DbError> {
        let row = sqlx::query_as::<_, SystemSettings>(
            "SELECT * FROM system_settings WHERE singleton_key = 'system'",
        )
            .fetch_optional(&self.pool)
            .await?;

        Ok(row)
    }

    async fn upsert_settings(
        &self,
        language: Option<&str>,
        notification_enabled: Option<bool>,
        cron_notification_enabled: Option<bool>,
        command_queue_enabled: Option<bool>,
        save_upload_to_workspace: Option<bool>,
    ) -> Result<SystemSettings, DbError> {
        let now = nomifun_common::now_ms();

        let row = sqlx::query_as::<_, SystemSettings>(
            "INSERT INTO system_settings \
                (singleton_key, language, notification_enabled, cron_notification_enabled, \
                 command_queue_enabled, save_upload_to_workspace, updated_at) \
             VALUES ('system', COALESCE(?1, 'en-US'), COALESCE(?2, 1), COALESCE(?3, 0), COALESCE(?4, 0), COALESCE(?5, 0), ?6) \
             ON CONFLICT(singleton_key) DO UPDATE SET \
                language = COALESCE(?1, system_settings.language), \
                notification_enabled = COALESCE(?2, system_settings.notification_enabled), \
                cron_notification_enabled = COALESCE(?3, system_settings.cron_notification_enabled), \
                command_queue_enabled = COALESCE(?4, system_settings.command_queue_enabled), \
                save_upload_to_workspace = COALESCE(?5, system_settings.save_upload_to_workspace), \
                updated_at = excluded.updated_at \
             RETURNING *",
        )
        .bind(language)
        .bind(notification_enabled)
        .bind(cron_notification_enabled)
        .bind(command_queue_enabled)
        .bind(save_upload_to_workspace)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }
}
