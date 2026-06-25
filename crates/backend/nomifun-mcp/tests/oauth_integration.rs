//! Integration tests for McpOAuthService with real SQLite.
//!
//! Tests from test-plan §4 (OAuth) at the service layer.
//! These tests exercise check_status, logout, get_authenticated_servers,
//! and get_token with a real DB. The full login flow (browser + callback)
//! cannot be tested end-to-end here; it requires a mock OAuth server.

use std::sync::Arc;

use nomifun_db::{IOAuthTokenRepository, SqliteOAuthTokenRepository, UpsertOAuthTokenParams};
use nomifun_mcp::McpOAuthService;

async fn make_service() -> (McpOAuthService, Arc<dyn IOAuthTokenRepository>) {
    let db = nomifun_db::init_database_memory().await.unwrap();
    let repo: Arc<dyn IOAuthTokenRepository> = Arc::new(SqliteOAuthTokenRepository::new(db.pool().clone()));
    let svc = McpOAuthService::new(repo.clone(), reqwest::Client::new());
    // Keep db alive by leaking it (integration test only).
    std::mem::forget(db);
    (svc, repo)
}

// ---------------------------------------------------------------------------
// OA-1: Unauthenticated server returns false
// ---------------------------------------------------------------------------

#[tokio::test]
async fn check_status_unauthenticated_returns_false() {
    let (svc, _repo) = make_service().await;
    let status = svc.check_oauth_status("https://new-server.example.com").await.unwrap();
    assert!(!status.authenticated);
}

// ---------------------------------------------------------------------------
// OA-2: Authenticated server returns true
// ---------------------------------------------------------------------------

#[tokio::test]
async fn check_status_authenticated_returns_true() {
    let (svc, repo) = make_service().await;

    // Seed a valid token.
    repo.upsert(UpsertOAuthTokenParams {
        server_url: "https://mcp.example.com",
        access_token: "access_123",
        refresh_token: Some("refresh_456"),
        token_type: "bearer",
        // Expires in the far future.
        expires_at: Some(nomifun_common::now_ms() + 3_600_000),
    })
    .await
    .unwrap();

    let status = svc.check_oauth_status("https://mcp.example.com").await.unwrap();
    assert!(status.authenticated);
}

// ---------------------------------------------------------------------------
// OA-2b: Expired token treated as unauthenticated
// ---------------------------------------------------------------------------

#[tokio::test]
async fn check_status_expired_token_returns_false() {
    let (svc, repo) = make_service().await;

    repo.upsert(UpsertOAuthTokenParams {
        server_url: "https://expired.example.com",
        access_token: "old_token",
        refresh_token: None,
        token_type: "bearer",
        // Already expired.
        expires_at: Some(1000),
    })
    .await
    .unwrap();

    let status = svc.check_oauth_status("https://expired.example.com").await.unwrap();
    assert!(!status.authenticated);
}

// ---------------------------------------------------------------------------
// OA-2c: Token with no expiry treated as valid
// ---------------------------------------------------------------------------

#[tokio::test]
async fn check_status_no_expiry_treated_as_valid() {
    let (svc, repo) = make_service().await;

    repo.upsert(UpsertOAuthTokenParams {
        server_url: "https://no-expiry.example.com",
        access_token: "no_exp_token",
        refresh_token: None,
        token_type: "bearer",
        expires_at: None,
    })
    .await
    .unwrap();

    let status = svc.check_oauth_status("https://no-expiry.example.com").await.unwrap();
    assert!(status.authenticated);
}

// ---------------------------------------------------------------------------
// OA-3: Get all authenticated URLs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_authenticated_servers_returns_all_urls() {
    let (svc, repo) = make_service().await;

    repo.upsert(UpsertOAuthTokenParams {
        server_url: "https://a.example.com",
        access_token: "tok_a",
        refresh_token: None,
        token_type: "bearer",
        expires_at: None,
    })
    .await
    .unwrap();

    repo.upsert(UpsertOAuthTokenParams {
        server_url: "https://b.example.com",
        access_token: "tok_b",
        refresh_token: None,
        token_type: "bearer",
        expires_at: None,
    })
    .await
    .unwrap();

    let urls = svc.get_authenticated_servers().await.unwrap();
    assert_eq!(urls.len(), 2);
    assert!(urls.contains(&"https://a.example.com".to_string()));
    assert!(urls.contains(&"https://b.example.com".to_string()));
}

#[tokio::test]
async fn get_authenticated_servers_empty_when_no_tokens() {
    let (svc, _repo) = make_service().await;
    let urls = svc.get_authenticated_servers().await.unwrap();
    assert!(urls.is_empty());
}

// ---------------------------------------------------------------------------
// OA-5: Login with invalid URL (no OAuth endpoints discoverable)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn login_invalid_url_returns_error() {
    let (svc, _repo) = make_service().await;
    // This URL won't have .well-known endpoints.
    let result = svc.login("https://127.0.0.1:1").await;
    // Should return an McpError::OAuth about discovery failure.
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// OA-6: Logout deletes stored token
// ---------------------------------------------------------------------------

#[tokio::test]
async fn logout_deletes_stored_token() {
    let (svc, repo) = make_service().await;

    repo.upsert(UpsertOAuthTokenParams {
        server_url: "https://logout.example.com",
        access_token: "to_delete",
        refresh_token: None,
        token_type: "bearer",
        expires_at: None,
    })
    .await
    .unwrap();

    // Verify token exists.
    let status = svc.check_oauth_status("https://logout.example.com").await.unwrap();
    assert!(status.authenticated);

    // Logout.
    svc.logout("https://logout.example.com").await.unwrap();

    // Verify token is gone.
    let status = svc.check_oauth_status("https://logout.example.com").await.unwrap();
    assert!(!status.authenticated);
}

// ---------------------------------------------------------------------------
// OA-7: Logout is idempotent for non-authenticated URL
// ---------------------------------------------------------------------------

#[tokio::test]
async fn logout_idempotent_for_unauthenticated() {
    let (svc, _repo) = make_service().await;
    // Should not error.
    svc.logout("https://never-authed.example.com").await.unwrap();
}

// ---------------------------------------------------------------------------
// get_token tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_token_returns_none_for_unknown_url() {
    let (svc, _repo) = make_service().await;
    let token = svc.get_token("https://unknown.example.com").await.unwrap();
    assert!(token.is_none());
}

#[tokio::test]
async fn get_token_returns_access_token_when_valid() {
    let (svc, repo) = make_service().await;

    repo.upsert(UpsertOAuthTokenParams {
        server_url: "https://valid.example.com",
        access_token: "my_access_token",
        refresh_token: None,
        token_type: "bearer",
        expires_at: Some(nomifun_common::now_ms() + 3_600_000),
    })
    .await
    .unwrap();

    let token = svc.get_token("https://valid.example.com").await.unwrap();
    assert_eq!(token.as_deref(), Some("my_access_token"));
}

#[tokio::test]
async fn get_token_returns_expired_token_when_no_refresh_token() {
    let (svc, repo) = make_service().await;

    repo.upsert(UpsertOAuthTokenParams {
        server_url: "https://expired.example.com",
        access_token: "old_access",
        refresh_token: None,
        token_type: "bearer",
        expires_at: Some(1000),
    })
    .await
    .unwrap();

    // With no refresh_token, returns the expired token as-is.
    let token = svc.get_token("https://expired.example.com").await.unwrap();
    assert_eq!(token.as_deref(), Some("old_access"));
}

#[tokio::test]
async fn get_token_returns_no_expiry_token() {
    let (svc, repo) = make_service().await;

    repo.upsert(UpsertOAuthTokenParams {
        server_url: "https://noexp.example.com",
        access_token: "forever_token",
        refresh_token: None,
        token_type: "bearer",
        expires_at: None,
    })
    .await
    .unwrap();

    let token = svc.get_token("https://noexp.example.com").await.unwrap();
    assert_eq!(token.as_deref(), Some("forever_token"));
}
