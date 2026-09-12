use nomifun_common::validate_uuidv7;
use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::{TagSettingPatch, TagSettingRow};
use crate::repository::tag_setting::ITagSettingRepository;

#[derive(Clone, Debug)]
pub struct SqliteTagSettingRepository {
    pool: SqlitePool,
}

impl SqliteTagSettingRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl ITagSettingRepository for SqliteTagSettingRepository {
    async fn get(&self, tag: &str) -> Result<Option<TagSettingRow>, DbError> {
        let row = sqlx::query_as::<_, TagSettingRow>(
            "SELECT tag, webhook_id, description, notify_events, updated_at \
             FROM tag_settings WHERE tag = ?",
        )
        .bind(tag)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    async fn upsert(&self, tag: &str, patch: &TagSettingPatch) -> Result<TagSettingRow, DbError> {
        let mut tx = self.pool.begin().await?;
        let webhook_id = patch.webhook_id.as_ref().and_then(|id| id.as_deref());
        if let Some(webhook_id) = webhook_id {
            validate_uuidv7(webhook_id).map_err(|error| {
                DbError::Conflict(format!("invalid webhook_id '{webhook_id}': {error}"))
            })?;
            let parent = sqlx::query(
                "UPDATE webhooks SET updated_at = updated_at WHERE webhook_id = ?",
            )
            .bind(webhook_id)
            .execute(&mut *tx)
            .await?;
            if parent.rows_affected() == 0 {
                return Err(DbError::Conflict(format!(
                    "tag setting webhook '{webhook_id}' does not exist"
                )));
            }
        }
        let row = sqlx::query_as::<_, TagSettingRow>(
            "INSERT INTO tag_settings (tag, webhook_id, description, notify_events, updated_at) \
             VALUES (?, ?, COALESCE(?, ''), COALESCE(?, 'done,failed,needs_review'), ?) \
             ON CONFLICT(tag) DO UPDATE SET \
                webhook_id = CASE WHEN ? THEN excluded.webhook_id ELSE tag_settings.webhook_id END, \
                description = COALESCE(?, tag_settings.description), \
                notify_events = COALESCE(?, tag_settings.notify_events), \
                updated_at = excluded.updated_at \
             RETURNING tag, webhook_id, description, notify_events, updated_at",
        )
        .bind(tag)
        .bind(webhook_id)
        .bind(&patch.description)
        .bind(&patch.notify_events)
        .bind(patch.updated_at)
        .bind(patch.webhook_id.is_some())
        .bind(&patch.description)
        .bind(&patch.notify_events)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    async fn list_all(&self) -> Result<Vec<TagSettingRow>, DbError> {
        let rows = sqlx::query_as::<_, TagSettingRow>(
            "SELECT tag, webhook_id, description, notify_events, updated_at \
             FROM tag_settings ORDER BY tag ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn delete(&self, tag: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM tag_settings WHERE tag = ?")
            .bind(tag)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
