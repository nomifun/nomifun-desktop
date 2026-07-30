//! Integration tests for the auth middleware's sliding session renewal
//! (audit 2026-07-30, finding E).
//!
//! A cookie session past half of its lifetime must receive a re-signed
//! `Set-Cookie` on the response; fresh sessions and bearer-token clients
//! must not.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::middleware;
use axum::routing::get;
use tower::ServiceExt;

use nomifun_auth::middleware::{AuthState, auth_middleware};
use nomifun_auth::{CookieConfig, JwtService};
use nomifun_common::UserId;
use nomifun_db::models::User;
use nomifun_db::{DbError, IUserRepository};

const TEST_USER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";

/// Stub repository: `find_by_id` resolves the fixed test user; everything
/// else is unreachable in these tests.
struct StubUserRepo;

fn test_user() -> User {
    User {
        id: 1,
        user_id: UserId::parse(TEST_USER_ID).unwrap(),
        username: "admin".into(),
        email: None,
        password_hash: "x".into(),
        avatar_path: None,
        jwt_secret: None,
        created_at: 0,
        updated_at: 0,
        last_login: None,
    }
}

#[async_trait::async_trait]
impl IUserRepository for StubUserRepo {
    async fn has_users(&self) -> Result<bool, DbError> {
        Ok(true)
    }
    async fn get_system_user(&self) -> Result<Option<User>, DbError> {
        unreachable!()
    }
    async fn get_primary_webui_user(&self) -> Result<Option<User>, DbError> {
        unreachable!()
    }
    async fn set_system_user_credentials(&self, _: &str, _: &str) -> Result<(), DbError> {
        unreachable!()
    }
    async fn set_system_user_credentials_if_uninitialized(&self, _: &str, _: &str) -> Result<bool, DbError> {
        unreachable!()
    }
    async fn set_system_user_password_if_uninitialized(&self, _: &str) -> Result<bool, DbError> {
        unreachable!()
    }
    async fn create_user(&self, _: &str, _: &str) -> Result<User, DbError> {
        unreachable!()
    }
    async fn find_by_username(&self, _: &str) -> Result<Option<User>, DbError> {
        unreachable!()
    }
    async fn find_by_id(&self, id: &str) -> Result<Option<User>, DbError> {
        Ok((id == TEST_USER_ID).then(test_user))
    }
    async fn list_users(&self) -> Result<Vec<User>, DbError> {
        unreachable!()
    }
    async fn count_users(&self) -> Result<i64, DbError> {
        unreachable!()
    }
    async fn update_password(&self, _: &str, _: &str) -> Result<(), DbError> {
        unreachable!()
    }
    async fn update_username(&self, _: &str, _: &str) -> Result<(), DbError> {
        unreachable!()
    }
    async fn update_last_login(&self, _: &str) -> Result<(), DbError> {
        unreachable!()
    }
    async fn update_jwt_secret(&self, _: &str, _: &str) -> Result<(), DbError> {
        unreachable!()
    }
}

fn app(jwt: Arc<JwtService>) -> Router {
    let state = AuthState {
        jwt_service: jwt,
        user_repo: Arc::new(StubUserRepo),
        cookie_config: Arc::new(CookieConfig {
            secure: false,
            same_site: "Lax",
        }),
    };
    Router::new()
        .route("/api/protected", get(|| async { "ok" }))
        .route_layer(middleware::from_fn_with_state(state, auth_middleware))
}

fn session_cookies(response: &axum::response::Response) -> Vec<String> {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter(|c| c.starts_with("nomifun-session="))
        .map(str::to_owned)
        .collect()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[tokio::test]
async fn cookie_session_past_half_life_is_renewed() {
    let jwt = Arc::new(JwtService::new("renewal-secret".into()));
    // 20 days old, 10 remaining — past the midpoint of a 30-day window.
    let old_token = jwt
        .sign_with_window(TEST_USER_ID, "admin", now_secs() - 20 * 86400, now_secs() + 10 * 86400)
        .unwrap();

    let response = app(jwt.clone())
        .oneshot(
            Request::get("/api/protected")
                .header(header::COOKIE, format!("nomifun-session={old_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let cookies = session_cookies(&response);
    assert_eq!(cookies.len(), 1, "expected exactly one renewal cookie: {cookies:?}");
    let renewed_token = cookies[0]
        .trim_start_matches("nomifun-session=")
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    assert_ne!(renewed_token, old_token, "renewal must mint a NEW token");
    // The renewed token restarts the full session window.
    let payload = jwt.verify(&renewed_token).unwrap();
    assert!(!nomifun_auth::is_past_half_life(&payload));
    assert!(cookies[0].contains("HttpOnly"), "renewed cookie keeps attributes");
}

#[tokio::test]
async fn fresh_cookie_session_is_not_renewed() {
    let jwt = Arc::new(JwtService::new("renewal-secret".into()));
    let token = jwt.sign(TEST_USER_ID, "admin").unwrap();

    let response = app(jwt)
        .oneshot(
            Request::get("/api/protected")
                .header(header::COOKIE, format!("nomifun-session={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        session_cookies(&response).is_empty(),
        "a token in its first half-life must not be re-issued"
    );
}

#[tokio::test]
async fn bearer_clients_are_never_renewed_via_cookie() {
    let jwt = Arc::new(JwtService::new("renewal-secret".into()));
    let old_token = jwt
        .sign_with_window(TEST_USER_ID, "admin", now_secs() - 20 * 86400, now_secs() + 10 * 86400)
        .unwrap();

    let response = app(jwt)
        .oneshot(
            Request::get("/api/protected")
                .header(header::AUTHORIZATION, format!("Bearer {old_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        session_cookies(&response).is_empty(),
        "bearer clients manage their own token; no cookie renewal"
    );
}

#[tokio::test]
async fn expired_session_still_rejects() {
    let jwt = Arc::new(JwtService::new("renewal-secret".into()));
    let expired = jwt
        .sign_with_window(TEST_USER_ID, "admin", now_secs() - 40 * 86400, now_secs() - 10 * 86400)
        .unwrap();

    let response = app(jwt)
        .oneshot(
            Request::get("/api/protected")
                .header(header::COOKIE, format!("nomifun-session={expired}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "renewal must never resurrect an already-expired session"
    );
}
