use sqlx::SqlitePool;

use crate::error::DbError;
use crate::models::OAuthTokenRow;
use crate::repository::oauth_token::{IOAuthTokenRepository, UpsertOAuthTokenParams};

/// SQLite-backed implementation of [`IOAuthTokenRepository`].
#[derive(Clone, Debug)]
pub struct SqliteOAuthTokenRepository {
    pool: SqlitePool,
}

impl SqliteOAuthTokenRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl IOAuthTokenRepository for SqliteOAuthTokenRepository {
    async fn get_by_url(&self, server_url: &str) -> Result<Option<OAuthTokenRow>, DbError> {
        let row = sqlx::query_as::<_, OAuthTokenRow>("SELECT * FROM oauth_tokens WHERE server_url = ?")
            .bind(server_url)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row)
    }

    async fn upsert(&self, params: UpsertOAuthTokenParams<'_>) -> Result<OAuthTokenRow, DbError> {
        let now = nomifun_common::now_ms();

        sqlx::query(
            "INSERT INTO oauth_tokens \
                (server_url, access_token, refresh_token, token_type, \
                 expires_at, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(server_url) DO UPDATE SET \
                access_token = excluded.access_token, \
                refresh_token = excluded.refresh_token, \
                token_type = excluded.token_type, \
                expires_at = excluded.expires_at, \
                updated_at = excluded.updated_at",
        )
        .bind(params.server_url)
        .bind(params.access_token)
        .bind(params.refresh_token)
        .bind(params.token_type)
        .bind(params.expires_at)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        // Fetch the row to get the correct created_at (preserved on conflict).
        let row = self
            .get_by_url(params.server_url)
            .await?
            .ok_or_else(|| DbError::Init("Upsert succeeded but row not found".to_string()))?;

        Ok(row)
    }

    async fn delete(&self, server_url: &str) -> Result<(), DbError> {
        let result = sqlx::query("DELETE FROM oauth_tokens WHERE server_url = ?")
            .bind(server_url)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(DbError::NotFound(format!("OAuth token for '{server_url}' not found")));
        }

        Ok(())
    }

    async fn list_authenticated_urls(&self) -> Result<Vec<String>, DbError> {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT server_url FROM oauth_tokens ORDER BY created_at ASC")
            .fetch_all(&self.pool)
            .await?;

        Ok(rows.into_iter().map(|(url,)| url).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_database_memory;

    async fn setup() -> (SqliteOAuthTokenRepository, crate::Database) {
        let db = init_database_memory().await.unwrap();
        let repo = SqliteOAuthTokenRepository::new(db.pool().clone());
        (repo, db)
    }

    fn sample_params() -> UpsertOAuthTokenParams<'static> {
        UpsertOAuthTokenParams {
            server_url: "https://mcp.example.com",
            access_token: "enc_access_token_123",
            refresh_token: Some("enc_refresh_token_456"),
            token_type: "bearer",
            expires_at: Some(1700000000000),
        }
    }

    #[tokio::test]
    async fn get_by_url_nonexistent() {
        let (repo, _db) = setup().await;
        assert!(repo.get_by_url("https://nope.com").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn upsert_insert_new_token() {
        let (repo, _db) = setup().await;
        let token = repo.upsert(sample_params()).await.unwrap();

        assert_eq!(token.server_url, "https://mcp.example.com");
        assert_eq!(token.access_token, "enc_access_token_123");
        assert_eq!(token.refresh_token.as_deref(), Some("enc_refresh_token_456"));
        assert_eq!(token.token_type, "bearer");
        assert_eq!(token.expires_at, Some(1700000000000));
        assert!(token.created_at > 0);
        assert_eq!(token.created_at, token.updated_at);
    }

    #[tokio::test]
    async fn upsert_updates_existing_token() {
        let (repo, _db) = setup().await;
        let original = repo.upsert(sample_params()).await.unwrap();

        let updated = repo
            .upsert(UpsertOAuthTokenParams {
                server_url: "https://mcp.example.com",
                access_token: "new_access_token",
                refresh_token: None,
                token_type: "bearer",
                expires_at: Some(1800000000000),
            })
            .await
            .unwrap();

        assert_eq!(updated.server_url, original.server_url);
        assert_eq!(updated.access_token, "new_access_token");
        assert!(updated.refresh_token.is_none());
        assert_eq!(updated.expires_at, Some(1800000000000));
        // created_at preserved from original insert
        assert_eq!(updated.created_at, original.created_at);
    }

    #[tokio::test]
    async fn get_by_url_returns_upserted_token() {
        let (repo, _db) = setup().await;
        repo.upsert(sample_params()).await.unwrap();

        let found = repo.get_by_url("https://mcp.example.com").await.unwrap().unwrap();
        assert_eq!(found.access_token, "enc_access_token_123");
    }

    #[tokio::test]
    async fn delete_existing_token() {
        let (repo, _db) = setup().await;
        repo.upsert(sample_params()).await.unwrap();

        repo.delete("https://mcp.example.com").await.unwrap();
        assert!(repo.get_by_url("https://mcp.example.com").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_nonexistent_returns_not_found() {
        let (repo, _db) = setup().await;
        let err = repo.delete("https://nope.com").await.unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[tokio::test]
    async fn list_authenticated_urls_empty() {
        let (repo, _db) = setup().await;
        let urls = repo.list_authenticated_urls().await.unwrap();
        assert!(urls.is_empty());
    }

    #[tokio::test]
    async fn list_authenticated_urls_returns_all() {
        let (repo, _db) = setup().await;
        repo.upsert(sample_params()).await.unwrap();
        repo.upsert(UpsertOAuthTokenParams {
            server_url: "https://other.example.com",
            access_token: "token2",
            refresh_token: None,
            token_type: "bearer",
            expires_at: None,
        })
        .await
        .unwrap();

        let urls = repo.list_authenticated_urls().await.unwrap();
        assert_eq!(urls.len(), 2);
        assert!(urls.contains(&"https://mcp.example.com".to_string()));
        assert!(urls.contains(&"https://other.example.com".to_string()));
    }
}
