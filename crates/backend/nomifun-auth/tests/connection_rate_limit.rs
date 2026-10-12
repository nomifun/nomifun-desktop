//! The loopback webview and in-process managers are locally trusted. They must
//! not share the per-IP API budget with real remote/browser/device callers:
//! trusted requests bypass it entirely, untrusted ones keep the 60/min budget.
use std::sync::Arc;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware::from_fn_with_state;
use nomifun_auth::{AuthPolicy, AuthRouterState, CookieConfig, JwtService, LOCAL_TRUST_HEADER, QrTokenStore, TrustState, auth_routes, trust_resolve_middleware};
use nomifun_db::{IUserRepository, SqliteUserRepository};
use tower::ServiceExt;

async fn app(secret: &'static str) -> axum::Router {
    let database = nomifun_db::init_database_memory().await.unwrap();
    let repo = Arc::new(SqliteUserRepository::new(database.pool().clone()));
    let user = repo.get_system_user().await.unwrap().unwrap();
    let jwt = Arc::new(JwtService::new("rate-limit-test-secret".into()));
    let app = auth_routes(AuthRouterState {
        auth_policy: AuthPolicy::TrustLocalToken,
        jwt_service: jwt, user_repo: repo,
        cookie_config: Arc::new(CookieConfig { secure: false, same_site: "Lax" }),
        qr_token_store: Arc::new(QrTokenStore::new()),
    });
    std::mem::forget(database);
    app.layer(from_fn_with_state(
        TrustState {
            policy: AuthPolicy::TrustLocalToken,
            local_trust_secret: Some(Arc::from(secret)),
            authoritative_user_id: Arc::from(user.user_id.as_str()),
        },
        trust_resolve_middleware,
    ))
}

#[tokio::test]
async fn locally_trusted_requests_skip_the_shared_api_budget() {
    let secret = "test-local-trust-secret";
    let app = app(secret).await;
    let trusted = |path: &str| {
        Request::get(path).header(LOCAL_TRUST_HEADER, secret).body(Body::empty()).unwrap()
    };
    for _ in 0..90 {
        let response = app.clone().oneshot(trusted("/api/connections/devices")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    for _ in 0..90 {
        let response = app.clone().oneshot(trusted("/api/connections/info")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    let mut ok = 0usize;
    let mut limited = 0usize;
    for _ in 0..70 {
        match app.clone().oneshot(Request::get("/api/connections/info").body(Body::empty()).unwrap()).await.unwrap().status() {
            StatusCode::OK => ok += 1,
            StatusCode::TOO_MANY_REQUESTS => limited += 1,
            other => panic!("unexpected status {other}"),
        }
    }
    assert_eq!((ok, limited), (60, 10));
}
