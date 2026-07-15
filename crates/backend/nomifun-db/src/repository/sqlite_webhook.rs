use sqlx::SqlitePool;
use nomifun_common::WebhookId;

use crate::error::DbError;
use crate::models::WebhookRow;
use crate::repository::webhook::IWebhookRepository;

#[derive(Clone, Debug)]
pub struct SqliteWebhookRepository {
    pool: SqlitePool,
}

impl SqliteWebhookRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IWebhookRepository for SqliteWebhookRepository {
    async fn insert(&self, row: &WebhookRow) -> Result<WebhookId, DbError> {
        sqlx::query(
            "INSERT INTO webhooks (\
                id, name, platform, url, secret, description, enabled, created_at, updated_at\
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(row.id.as_str())
        .bind(&row.name)
        .bind(&row.platform)
        .bind(&row.url)
        .bind(&row.secret)
        .bind(&row.description)
        .bind(row.enabled)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(row.id.clone())
    }

    async fn update(&self, row: &WebhookRow) -> Result<(), DbError> {
        let result = sqlx::query(
            "UPDATE webhooks SET \
                name = ?, platform = ?, url = ?, secret = ?, description = ?, enabled = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(&row.name)
        .bind(&row.platform)
        .bind(&row.url)
        .bind(&row.secret)
        .bind(&row.description)
        .bind(row.enabled)
        .bind(row.updated_at)
        .bind(row.id.as_str())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("webhook {}", row.id)));
        }
        Ok(())
    }

    async fn delete(&self, id: &WebhookId) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM webhooks WHERE id = ?")
            .bind(id.as_str())
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("webhook {id}")));
        }
        Ok(())
    }

    async fn get_by_id(&self, id: &WebhookId) -> Result<Option<WebhookRow>, DbError> {
        let row = sqlx::query_as::<_, WebhookRow>("SELECT * FROM webhooks WHERE id = ?")
            .bind(id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    async fn list_all(&self) -> Result<Vec<WebhookRow>, DbError> {
        let rows = sqlx::query_as::<_, WebhookRow>("SELECT * FROM webhooks ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows)
    }
}
