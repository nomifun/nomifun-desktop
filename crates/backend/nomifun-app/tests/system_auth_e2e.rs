//! Auth protection tests — all system endpoints return 403 without auth.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use common::{build_app, get_request};

#[tokio::test]
async fn auth_required_get_settings() {
    let (app, _) = build_app().await;
    let resp = app.oneshot(get_request("/api/settings")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_patch_settings() {
    let (app, _) = build_app().await;
    let req = Request::builder()
        .method("PATCH")
        .uri("/api/settings")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"language":"en-US"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_get_client_prefs() {
    let (app, _) = build_app().await;
    let resp = app.oneshot(get_request("/api/settings/client")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_put_client_prefs() {
    let (app, _) = build_app().await;
    let req = Request::builder()
        .method("PUT")
        .uri("/api/settings/client")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"key":"value"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_get_providers() {
    let (app, _) = build_app().await;
    let resp = app.oneshot(get_request("/api/providers")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_post_providers() {
    let (app, _) = build_app().await;
    let req = Request::builder()
        .method("POST")
        .uri("/api/providers")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"platform":"openai","name":"Test","base_url":"https://api.openai.com","api_key":"sk-test"}"#,
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_delete_provider() {
    let (app, _) = build_app().await;
    let req = Request::builder()
        .method("DELETE")
        .uri("/api/providers/some-id")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_system_info() {
    let (app, _) = build_app().await;
    let resp = app.oneshot(get_request("/api/system/info")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_check_update() {
    let (app, _) = build_app().await;
    let req = Request::builder()
        .method("POST")
        .uri("/api/system/check-update")
        .header("content-type", "application/json")
        .body(Body::from(r#"{}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_detect_protocol() {
    let (app, _) = build_app().await;
    let req = Request::builder()
        .method("POST")
        .uri("/api/providers/detect-protocol")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"base_url":"https://api.example.com","api_key":"sk-test"}"#,
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn auth_required_fetch_models() {
    let (app, _) = build_app().await;
    let req = Request::builder()
        .method("POST")
        .uri("/api/providers/some-id/models")
        .header("content-type", "application/json")
        .body(Body::from(r#"{}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
