mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use common::{body_json, build_app, get_request};

#[tokio::test]
async fn health_check_returns_ok() {
    let (app, _) = build_app().await;

    let response = app
        .oneshot(get_request("/health"))
        .await
        .expect("request failed");

    assert_eq!(response.status(), StatusCode::OK);

    let json = body_json(response).await;
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn health_check_post_blocked_by_csrf() {
    let (app, _) = build_app().await;

    // POST without CSRF token is rejected by the global CSRF middleware
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request failed");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn unknown_route_returns_not_found() {
    let (app, _) = build_app().await;

    let response = app
        .oneshot(get_request("/nonexistent"))
        .await
        .expect("request failed");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn health_check_has_security_headers() {
    let (app, _) = build_app().await;

    let response = app
        .oneshot(get_request("/health"))
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
