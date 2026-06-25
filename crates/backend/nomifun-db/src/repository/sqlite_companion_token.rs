use sqlx::SqlitePool;

use crate::error::DbError;
use crate::repository::ICompanionTokenRepository;

/// SQLite-backed [`ICompanionTokenRepository`]. Keyed on `companion_id`;
/// `upsert_for_companion` rotates a companion's single token row.
#[derive(Clone, Debug)]
pub struct SqliteCompanionTokenRepository {
    pool: SqlitePool,
}

impl SqliteCompanionTokenRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl ICompanionTokenRepository for SqliteCompanionTokenRepository {
    async fn list_all(&self) -> Result<Vec<(String, String)>, DbError> {
        let rows = sqlx::query_as::<_, (String, String)>(
            "SELECT companion_id, token_hash FROM companion_access_token",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn upsert_for_companion(&self, companion_id: &str, token_hash: &str) -> Result<(), DbError> {
        let now = nomifun_common::now_ms();
        sqlx::query(
            "INSERT INTO companion_access_token (companion_id, token_hash, created_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(companion_id) DO UPDATE SET token_hash = ?2, created_at = ?3",
        )
        .bind(companion_id)
        .bind(token_hash)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_for_companion(&self, companion_id: &str) -> Result<(), DbError> {
        sqlx::query("DELETE FROM companion_access_token WHERE companion_id = ?1")
            .bind(companion_id)
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
    async fn companion_token_roundtrip() {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteCompanionTokenRepository::new(db.pool().clone());

        // Empty until minted.
        assert!(repo.list_all().await.unwrap().is_empty());

        repo.upsert_for_companion("comp-a", "hash-a").await.unwrap();
        repo.upsert_for_companion("comp-b", "hash-b").await.unwrap();
        let mut all = repo.list_all().await.unwrap();
        all.sort();
        assert_eq!(
            all,
            vec![
                ("comp-a".to_string(), "hash-a".to_string()),
                ("comp-b".to_string(), "hash-b".to_string()),
            ]
        );

        // Re-mint for the same companion rotates its hash (keyed on companion_id).
        repo.upsert_for_companion("comp-a", "hash-a2").await.unwrap();
        let all = repo.list_all().await.unwrap();
        assert!(all.contains(&("comp-a".to_string(), "hash-a2".to_string())));
        assert_eq!(all.len(), 2);

        // Revocation is idempotent.
        repo.delete_for_companion("comp-a").await.unwrap();
        repo.delete_for_companion("comp-a").await.unwrap();
        let all = repo.list_all().await.unwrap();
        assert_eq!(all, vec![("comp-b".to_string(), "hash-b".to_string())]);
    }
}
