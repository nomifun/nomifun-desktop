//! Owner-scoped, revision-checked documents for the MiniApp product workspace.
use crate::{DbError, SqlitePool};

#[derive(Clone)]
pub struct MiniAppProductDocuments {
    pool: SqlitePool,
}

impl MiniAppProductDocuments {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(&self, owner: &str, key: &str) -> Result<Option<(i64, String)>, DbError> {
        Ok(sqlx::query_as("SELECT revision, content_json FROM miniapp_product_documents WHERE owner_user_id = ? AND document_key = ?")
            .bind(owner).bind(key).fetch_optional(&self.pool).await?)
    }

    pub async fn list(
        &self,
        owner: &str,
        prefix: &str,
    ) -> Result<Vec<(String, i64, String)>, DbError> {
        Ok(sqlx::query_as("SELECT document_key, revision, content_json FROM miniapp_product_documents WHERE owner_user_id = ? AND document_key LIKE ? ORDER BY updated_at DESC")
            .bind(owner).bind(format!("{prefix}%")).fetch_all(&self.pool).await?)
    }

    /// `expected = 0` creates; all subsequent changes compare the known revision.
    pub async fn put(
        &self,
        owner: &str,
        key: &str,
        expected: i64,
        json: &str,
    ) -> Result<i64, DbError> {
        if expected < 0 || expected == i64::MAX {
            return Err(DbError::Conflict("Invalid document revision".into()));
        }
        let result = if expected == 0 {
            sqlx::query("INSERT OR IGNORE INTO miniapp_product_documents (owner_user_id, document_key, revision, content_json, updated_at) SELECT ?, ?, 1, ?, ? WHERE EXISTS (SELECT 1 FROM users WHERE user_id = ?)")
                .bind(owner).bind(key).bind(json).bind(nomifun_common::now_ms()).bind(owner).execute(&self.pool).await?
        } else {
            sqlx::query("UPDATE miniapp_product_documents SET revision = revision + 1, content_json = ?, updated_at = ? WHERE owner_user_id = ? AND document_key = ? AND revision = ?")
                .bind(json).bind(nomifun_common::now_ms()).bind(owner).bind(key).bind(expected).execute(&self.pool).await?
        };
        if result.rows_affected() != 1 {
            return Err(DbError::Conflict(
                "The MiniApp document changed. Reload before retrying.".into(),
            ));
        }
        Ok(expected + 1)
    }

    pub async fn delete(&self, owner: &str, key: &str, expected: i64) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM miniapp_product_documents WHERE owner_user_id = ? AND document_key = ? AND revision = ?")
            .bind(owner).bind(key).bind(expected).execute(&self.pool).await?;
        if result.rows_affected() != 1 {
            return Err(DbError::Conflict("The MiniApp document changed".into()));
        }
        Ok(())
    }
}
