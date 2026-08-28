use sqlx::SqlitePool;

use crate::error::DbError;
use crate::repository::IInstanceTokenRepository;

/// SQLite-backed repository for the singleton installation Remote token.
#[derive(Clone, Debug)]
pub struct SqliteInstanceTokenRepository {
    pool: SqlitePool,
}

impl SqliteInstanceTokenRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IInstanceTokenRepository for SqliteInstanceTokenRepository {
    async fn get(&self) -> Result<Option<String>, DbError> {
        sqlx::query_scalar(
            "SELECT token_hash FROM instance_access_token WHERE singleton_key = 'instance'",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    async fn set(&self, token_hash: &str) -> Result<(), DbError> {
        let now = nomifun_common::now_ms();
        sqlx::query(
            "INSERT INTO instance_access_token (singleton_key, token_hash, created_at) \
             VALUES ('instance', ?1, ?2) \
             ON CONFLICT(singleton_key) DO UPDATE SET token_hash = ?1, created_at = ?2",
        )
        .bind(token_hash)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn clear(&self) -> Result<(), DbError> {
        sqlx::query("DELETE FROM instance_access_token WHERE singleton_key = 'instance'")
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;

    #[tokio::test]
    async fn instance_token_roundtrip() {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteInstanceTokenRepository::new(db.pool().clone());

        assert_eq!(repo.get().await.unwrap(), None);
        let legacy_table: Option<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_schema WHERE type = 'table' AND name = 'companion_access_token'",
        )
        .fetch_optional(db.pool())
        .await
        .unwrap();
        assert_eq!(legacy_table, None, "legacy companion tokens must not survive migration 058");
        repo.set("hash-a").await.unwrap();
        assert_eq!(repo.get().await.unwrap().as_deref(), Some("hash-a"));

        repo.set("hash-b").await.unwrap();
        assert_eq!(repo.get().await.unwrap().as_deref(), Some("hash-b"));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM instance_access_token")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 1);

        repo.clear().await.unwrap();
        repo.clear().await.unwrap();
        assert_eq!(repo.get().await.unwrap(), None);
    }
}
