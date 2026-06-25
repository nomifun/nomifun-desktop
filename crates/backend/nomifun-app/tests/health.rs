use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use nomifun_app::{AppConfig, AppServices};

fn build_request(method: &str, uri: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .expect("failed to build request")
}

async fn response_json(body: Body) -> serde_json::Value {
    let bytes = body.collect().await.expect("failed to read body").to_bytes();
    serde_json::from_slice(&bytes).expect("failed to parse JSON")
}

async fn build_app() -> axum::Router {
    let db = nomifun_db::init_database_memory().await.unwrap();
    let services = AppServices::from_config(db, &AppConfig::default()).await.unwrap();
    nomifun_app::create_router(&services).await
}

#[tokio::test]
async fn health_check_returns_ok() {
    let app = build_app().await;

    let response = app
        .oneshot(build_request("GET", "/health"))
        .await
        .expect("request failed");

    assert_eq!(response.status(), StatusCode::OK);

    let json = response_json(response.into_body()).await;
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn health_check_post_blocked_by_csrf() {
    let app = build_app().await;

    // POST without CSRF token is rejected by the global CSRF middleware
    let response = app
        .oneshot(build_request("POST", "/health"))
        .await
        .expect("request failed");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn unknown_route_returns_not_found() {
    let app = build_app().await;

    let response = app
        .oneshot(build_request("GET", "/nonexistent"))
        .await
        .expect("request failed");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn health_check_has_security_headers() {
    let app = build_app().await;

    let response = app
        .oneshot(build_request("GET", "/health"))
        .await
        .expect("request failed");

    assert_eq!(response.headers().get("x-frame-options").unwrap(), "DENY");
    assert_eq!(response.headers().get("x-content-type-options").unwrap(), "nosniff");
    assert_eq!(response.headers().get("x-xss-protection").unwrap(), "1; mode=block");
    assert_eq!(
        response.headers().get("referrer-policy").unwrap(),
        "strict-origin-when-cross-origin"
    );
}
