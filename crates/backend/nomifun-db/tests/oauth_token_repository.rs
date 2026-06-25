//! Black-box integration tests for `IOAuthTokenRepository`.
//!
//! Tests exercise the repository trait interface without knowledge of
//! the underlying SQLite implementation details.

use std::sync::Arc;

use nomifun_db::{
    DbError, IOAuthTokenRepository, SqliteOAuthTokenRepository, UpsertOAuthTokenParams, init_database_memory,
};

async fn repo() -> (Arc<dyn IOAuthTokenRepository>, nomifun_db::Database) {
    let db = init_database_memory().await.unwrap();
    let r = Arc::new(SqliteOAuthTokenRepository::new(db.pool().clone()));
    (r as Arc<dyn IOAuthTokenRepository>, db)
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

// -- OA-1: Unauthenticated server --

#[tokio::test]
async fn get_by_url_nonexistent_returns_none() {
    let (r, _db) = repo().await;
    assert!(r.get_by_url("https://nope.com").await.unwrap().is_none());
}

// -- OA-2: Insert and retrieve --

#[tokio::test]
async fn upsert_insert_then_get_returns_token() {
    let (r, _db) = repo().await;
    let inserted = r.upsert(sample_params()).await.unwrap();

    assert_eq!(inserted.server_url, "https://mcp.example.com");
    assert_eq!(inserted.access_token, "enc_access_token_123");
    assert_eq!(inserted.refresh_token.as_deref(), Some("enc_refresh_token_456"));
    assert_eq!(inserted.token_type, "bearer");
    assert_eq!(inserted.expires_at, Some(1700000000000));
    assert!(inserted.created_at > 0);

    let found = r.get_by_url("https://mcp.example.com").await.unwrap().unwrap();
    assert_eq!(found.access_token, "enc_access_token_123");
}

// -- Upsert updates existing --

#[tokio::test]
async fn upsert_updates_existing_token() {
    let (r, _db) = repo().await;
    let original = r.upsert(sample_params()).await.unwrap();

    let updated = r
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

// -- Upsert without optional fields --

#[tokio::test]
async fn upsert_without_refresh_token_or_expires_at() {
    let (r, _db) = repo().await;
    let token = r
        .upsert(UpsertOAuthTokenParams {
            server_url: "https://simple.example.com",
            access_token: "simple_token",
            refresh_token: None,
            token_type: "bearer",
            expires_at: None,
        })
        .await
        .unwrap();

    assert!(token.refresh_token.is_none());
    assert!(token.expires_at.is_none());
}

// -- OA-6: Delete existing --

#[tokio::test]
async fn delete_existing_token() {
    let (r, _db) = repo().await;
    r.upsert(sample_params()).await.unwrap();

    r.delete("https://mcp.example.com").await.unwrap();
    assert!(r.get_by_url("https://mcp.example.com").await.unwrap().is_none());
}

// -- OA-7: Delete idempotency (returns NotFound for nonexistent) --

#[tokio::test]
async fn delete_nonexistent_returns_not_found() {
    let (r, _db) = repo().await;
    let err = r.delete("https://nope.com").await.unwrap_err();
    assert!(matches!(err, DbError::NotFound(_)));
}

// -- OA-3: List authenticated URLs --

#[tokio::test]
async fn list_authenticated_urls_empty() {
    let (r, _db) = repo().await;
    let urls = r.list_authenticated_urls().await.unwrap();
    assert!(urls.is_empty());
}

#[tokio::test]
async fn list_authenticated_urls_returns_all() {
    let (r, _db) = repo().await;
    r.upsert(sample_params()).await.unwrap();
    r.upsert(UpsertOAuthTokenParams {
        server_url: "https://other.example.com",
        access_token: "token2",
        refresh_token: None,
        token_type: "bearer",
        expires_at: None,
    })
    .await
    .unwrap();

    let urls = r.list_authenticated_urls().await.unwrap();
    assert_eq!(urls.len(), 2);
    assert!(urls.contains(&"https://mcp.example.com".to_string()));
    assert!(urls.contains(&"https://other.example.com".to_string()));
}

// -- Delete does not affect other tokens --

#[tokio::test]
async fn delete_one_does_not_affect_others() {
    let (r, _db) = repo().await;
    r.upsert(sample_params()).await.unwrap();
    r.upsert(UpsertOAuthTokenParams {
        server_url: "https://other.example.com",
        access_token: "token2",
        refresh_token: None,
        token_type: "bearer",
        expires_at: None,
    })
    .await
    .unwrap();

    r.delete("https://mcp.example.com").await.unwrap();

    let urls = r.list_authenticated_urls().await.unwrap();
    assert_eq!(urls.len(), 1);
    assert_eq!(urls[0], "https://other.example.com");
}

// -- Full lifecycle --

#[tokio::test]
async fn full_oauth_lifecycle() {
    let (r, _db) = repo().await;

    // Initially no tokens
    assert!(r.list_authenticated_urls().await.unwrap().is_empty());
    assert!(r.get_by_url("https://mcp.example.com").await.unwrap().is_none());

    // Store token
    let token = r.upsert(sample_params()).await.unwrap();
    assert_eq!(token.access_token, "enc_access_token_123");

    // Verify stored
    let urls = r.list_authenticated_urls().await.unwrap();
    assert_eq!(urls.len(), 1);

    // Update token (refresh)
    let refreshed = r
        .upsert(UpsertOAuthTokenParams {
            server_url: "https://mcp.example.com",
            access_token: "refreshed_token",
            refresh_token: Some("new_refresh"),
            token_type: "bearer",
            expires_at: Some(1900000000000),
        })
        .await
        .unwrap();
    assert_eq!(refreshed.access_token, "refreshed_token");
    assert_eq!(refreshed.created_at, token.created_at);

    // Logout (delete)
    r.delete("https://mcp.example.com").await.unwrap();
    assert!(r.list_authenticated_urls().await.unwrap().is_empty());
}
