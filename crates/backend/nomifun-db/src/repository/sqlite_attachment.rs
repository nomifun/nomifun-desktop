use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::AttachmentRow;
use crate::repository::attachment::IAttachmentRepository;

#[derive(Clone, Debug)]
pub struct SqliteAttachmentRepository {
    pool: SqlitePool,
}

impl SqliteAttachmentRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IAttachmentRepository for SqliteAttachmentRepository {
    async fn insert(&self, row: &AttachmentRow) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO attachments (\
                id, requirement_id, file_name, rel_path, mime, size_bytes, created_by, created_at\
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(row.requirement_id)
        .bind(&row.file_name)
        .bind(&row.rel_path)
        .bind(&row.mime)
        .bind(row.size_bytes)
        .bind(&row.created_by)
        .bind(row.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<AttachmentRow>, DbError> {
        let row = sqlx::query_as::<_, AttachmentRow>("SELECT * FROM attachments WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    async fn list_for_requirement(&self, requirement_id: i64) -> Result<Vec<AttachmentRow>, DbError> {
        let rows = sqlx::query_as::<_, AttachmentRow>(
            "SELECT * FROM attachments WHERE requirement_id = ? ORDER BY created_at ASC, id ASC",
        )
        .bind(requirement_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn delete(&self, id: &str) -> Result<bool, DbError> {
        let result = sqlx::query("DELETE FROM attachments WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
