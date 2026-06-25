//! Settings and client preferences CRUD tests with auth.

mod common;

use axum::http::StatusCode;
use serde_json::json;
use tower::ServiceExt;

use common::{body_json, build_app, get_with_token, json_with_token, setup_and_login};

// ===========================================================================
// Settings CRUD
// ===========================================================================

#[tokio::test]
async fn settings_get_default_values_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let resp = app.oneshot(get_with_token("/api/settings", &token)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["language"], "en-US");
    assert_eq!(json["data"]["notification_enabled"], true);
}

#[tokio::test]
async fn settings_patch_and_get_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "PATCH",
        "/api/settings",
        json!({"language": "zh-CN", "notification_enabled": false}),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["language"], "zh-CN");
    assert_eq!(json["data"]["notification_enabled"], false);

    let resp = app.oneshot(get_with_token("/api/settings", &token)).await.unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["data"]["language"], "zh-CN");
    assert_eq!(json["data"]["notification_enabled"], false);
}

#[tokio::test]
async fn settings_invalid_language_rejected_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "PATCH",
        "/api/settings",
        json!({"language": "invalid-lang"}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ===========================================================================
// Client Preferences CRUD
// ===========================================================================

#[tokio::test]
async fn client_prefs_empty_then_write_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let resp = app
        .clone()
        .oneshot(get_with_token("/api/settings/client", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"], json!({}));

    let req = json_with_token(
        "PUT",
        "/api/settings/client",
        json!({"theme": "dark", "companion.size": 360, "system.closeToTray": true}),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = app
        .clone()
        .oneshot(get_with_token("/api/settings/client", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["data"]["theme"], "dark");
    assert_eq!(json["data"]["companion.size"], 360);
    assert_eq!(json["data"]["system.closeToTray"], true);

    let req = json_with_token("PUT", "/api/settings/client", json!({"theme": null}), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = app
        .oneshot(get_with_token("/api/settings/client", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert!(json["data"].get("theme").is_none());
    assert_eq!(json["data"]["companion.size"], 360);
}

#[tokio::test]
async fn client_prefs_key_filter_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "PUT",
        "/api/settings/client",
        json!({"a": 1, "b": 2, "c": 3}),
        &token,
        &csrf,
    );
    app.clone().oneshot(req).await.unwrap();

    let resp = app
        .oneshot(get_with_token("/api/settings/client?keys=a,c", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    let data = json["data"].as_object().unwrap();
    assert_eq!(data.len(), 2);
    assert_eq!(data["a"], 1);
    assert_eq!(data["c"], 3);
}
