use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::ClientPreference;
use crate::repository::IClientPreferenceRepository;

/// SQLite-backed implementation of [`IClientPreferenceRepository`].
#[derive(Clone, Debug)]
pub struct SqliteClientPreferenceRepository {
    pool: SqlitePool,
}

impl SqliteClientPreferenceRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IClientPreferenceRepository for SqliteClientPreferenceRepository {
    async fn get_all(&self) -> Result<Vec<ClientPreference>, DbError> {
        let rows = sqlx::query_as::<_, ClientPreference>("SELECT * FROM client_preferences ORDER BY key")
            .fetch_all(&self.pool)
            .await?;

        Ok(rows)
    }

    async fn get_by_keys(&self, keys: &[&str]) -> Result<Vec<ClientPreference>, DbError> {
        if keys.is_empty() {
            return Ok(vec![]);
        }

        // Build dynamic IN clause with positional placeholders
        let placeholders: Vec<&str> = keys.iter().map(|_| "?").collect();
        let sql = format!(
            "SELECT * FROM client_preferences WHERE key IN ({}) ORDER BY key",
            placeholders.join(", ")
        );

        let mut query = sqlx::query_as::<_, ClientPreference>(&sql);
        for key in keys {
            query = query.bind(*key);
        }

        let rows = query.fetch_all(&self.pool).await?;
        Ok(rows)
    }

    async fn upsert_batch(&self, entries: &[(&str, &str)]) -> Result<(), DbError> {
        if entries.is_empty() {
            return Ok(());
        }

        let now = nomifun_common::now_ms();

        // Use a transaction for atomicity
        let mut tx = self.pool.begin().await?;

        for (key, value) in entries {
            sqlx::query(
                "INSERT INTO client_preferences (key, value, updated_at) \
                 VALUES (?, ?, ?) \
                 ON CONFLICT(key) DO UPDATE SET \
                    value = excluded.value, \
                    updated_at = excluded.updated_at",
            )
            .bind(*key)
            .bind(*value)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn delete_keys(&self, keys: &[&str]) -> Result<(), DbError> {
        if keys.is_empty() {
            return Ok(());
        }

        let placeholders: Vec<&str> = keys.iter().map(|_| "?").collect();
        let sql = format!(
            "DELETE FROM client_preferences WHERE key IN ({})",
            placeholders.join(", ")
        );

        let mut query = sqlx::query(&sql);
        for key in keys {
            query = query.bind(*key);
        }

        query.execute(&self.pool).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;

    async fn setup() -> (SqliteClientPreferenceRepository, crate::Database) {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteClientPreferenceRepository::new(db.pool().clone());
        (repo, db)
    }

    #[tokio::test]
    async fn get_all_empty() {
        let (repo, _db) = setup().await;
        let prefs = repo.get_all().await.unwrap();
        assert!(prefs.is_empty());
    }

    #[tokio::test]
    async fn upsert_and_get_all() {
        let (repo, _db) = setup().await;
        repo.upsert_batch(&[("theme", "\"dark\""), ("companion.size", "360")])
            .await
            .unwrap();

        let prefs = repo.get_all().await.unwrap();
        assert_eq!(prefs.len(), 2);
        assert_eq!(prefs[0].key, "companion.size");
        assert_eq!(prefs[0].value, "360");
        assert_eq!(prefs[1].key, "theme");
        assert_eq!(prefs[1].value, "\"dark\"");
    }

    #[tokio::test]
    async fn get_by_keys_filters_correctly() {
        let (repo, _db) = setup().await;
        repo.upsert_batch(&[("a", "1"), ("b", "2"), ("c", "3")]).await.unwrap();

        let prefs = repo.get_by_keys(&["a", "c", "nonexistent"]).await.unwrap();
        assert_eq!(prefs.len(), 2);

        let keys: Vec<&str> = prefs.iter().map(|p| p.key.as_str()).collect();
        assert!(keys.contains(&"a"));
        assert!(keys.contains(&"c"));
    }

    #[tokio::test]
    async fn get_by_keys_empty_input() {
        let (repo, _db) = setup().await;
        let prefs = repo.get_by_keys(&[]).await.unwrap();
        assert!(prefs.is_empty());
    }

    #[tokio::test]
    async fn upsert_overwrites_existing_key() {
        let (repo, _db) = setup().await;
        repo.upsert_batch(&[("k", "v1")]).await.unwrap();
        repo.upsert_batch(&[("k", "v2")]).await.unwrap();

        let prefs = repo.get_all().await.unwrap();
        assert_eq!(prefs.len(), 1);
        assert_eq!(prefs[0].value, "v2");
    }

    #[tokio::test]
    async fn delete_keys_removes_entries() {
        let (repo, _db) = setup().await;
        repo.upsert_batch(&[("a", "1"), ("b", "2"), ("c", "3")]).await.unwrap();

        repo.delete_keys(&["a", "c"]).await.unwrap();

        let prefs = repo.get_all().await.unwrap();
        assert_eq!(prefs.len(), 1);
        assert_eq!(prefs[0].key, "b");
    }

    #[tokio::test]
    async fn delete_keys_nonexistent_is_noop() {
        let (repo, _db) = setup().await;
        repo.delete_keys(&["ghost"]).await.unwrap();
        assert!(repo.get_all().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_keys_empty_input() {
        let (repo, _db) = setup().await;
        repo.upsert_batch(&[("x", "1")]).await.unwrap();
        repo.delete_keys(&[]).await.unwrap();
        assert_eq!(repo.get_all().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn upsert_empty_batch_is_noop() {
        let (repo, _db) = setup().await;
        repo.upsert_batch(&[]).await.unwrap();
        assert!(repo.get_all().await.unwrap().is_empty());
    }
}
