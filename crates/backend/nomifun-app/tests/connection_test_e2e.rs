//! E2E tests for Bedrock test-connection endpoint.

mod common;

use axum::http::StatusCode;
use serde_json::json;
use tower::ServiceExt;

use common::{body_json, build_app, json_with_token, setup_and_login};

// ── 8.1 Bedrock Connection Test ─────────────────────────────────────

#[tokio::test]
async fn t8_1_bedrock_missing_config() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token("POST", "/api/bedrock/test-connection", json!({}), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let json = body_json(resp).await;
    assert_eq!(json["success"], false);
}

#[tokio::test]
async fn t8_1_bedrock_missing_region() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/bedrock/test-connection",
        json!({
            "bedrock_config": {
                "auth_method": "accessKey",
                "region": "",
                "access_key_id": "AKIAIOSFODNN7",
                "secret_access_key": "wJalrXUtnFEMI"
            }
        }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let json = body_json(resp).await;
    assert_eq!(json["success"], false);
    assert!(json["error"].as_str().unwrap().contains("region"));
}

#[tokio::test]
async fn t8_1_bedrock_access_key_missing_key_id() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/bedrock/test-connection",
        json!({
            "bedrock_config": {
                "auth_method": "accessKey",
                "region": "us-east-1",
                "secret_access_key": "wJalrXUtnFEMI"
            }
        }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let json = body_json(resp).await;
    assert!(json["error"].as_str().unwrap().contains("accessKeyId"));
}

#[tokio::test]
async fn t8_1_bedrock_access_key_missing_secret() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/bedrock/test-connection",
        json!({
            "bedrock_config": {
                "auth_method": "accessKey",
                "region": "us-east-1",
                "access_key_id": "AKIAIOSFODNN7"
            }
        }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let json = body_json(resp).await;
    assert!(json["error"].as_str().unwrap().contains("secretAccessKey"));
}

#[tokio::test]
async fn t8_1_bedrock_profile_missing() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/bedrock/test-connection",
        json!({
            "bedrock_config": {
                "auth_method": "profile",
                "region": "us-east-1"
            }
        }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let json = body_json(resp).await;
    assert!(json["error"].as_str().unwrap().contains("profile"));
}

#[tokio::test]
async fn t8_1_bedrock_unauthenticated() {
    let (app, _services) = build_app().await;

    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/api/bedrock/test-connection")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::to_vec(&json!({
                "bedrock_config": {
                    "auth_method": "accessKey",
                    "region": "us-east-1",
                    "access_key_id": "AKIA",
                    "secret_access_key": "secret"
                }
            }))
            .unwrap(),
        ))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    // CSRF middleware returns 403 for POST without CSRF token
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn t8_1_bedrock_invalid_credentials() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/bedrock/test-connection",
        json!({
            "bedrock_config": {
                "auth_method": "accessKey",
                "region": "us-east-1",
                "access_key_id": "AKIAFAKEKEY1234567890",
                "secret_access_key": "fakesecretkey1234567890abcdefghijklmnopq"
            }
        }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    // Fake credentials fail at the AWS API level → 422 Unprocessable Entity
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let json = body_json(resp).await;
    assert_eq!(json["success"], false);
    assert!(json["error"].as_str().unwrap().contains("Bedrock credentials invalid"));
}
