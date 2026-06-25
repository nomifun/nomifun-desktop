use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::TagSettingRow;
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
        let row = sqlx::query_as::<_, TagSettingRow>("SELECT * FROM tag_settings WHERE tag = ?")
            .bind(tag)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    async fn upsert(&self, row: &TagSettingRow) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO tag_settings (tag, webhook_id, description, notify_events, updated_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT(tag) DO UPDATE SET \
                webhook_id = excluded.webhook_id, \
                description = excluded.description, \
                notify_events = excluded.notify_events, \
                updated_at = excluded.updated_at",
        )
        .bind(&row.tag)
        .bind(&row.webhook_id)
        .bind(&row.description)
        .bind(&row.notify_events)
        .bind(row.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_all(&self) -> Result<Vec<TagSettingRow>, DbError> {
        let rows = sqlx::query_as::<_, TagSettingRow>("SELECT * FROM tag_settings ORDER BY tag ASC")
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
