//! E2E tests for the Creative Studio public read-only file channel. Asset files
//! must be reachable WITHOUT credentials (so `<img>`/`<video>` subresource
//! loads work under the desktop's local-trust policy), while every management
//! route stays authenticated and retired Workshop routes stay absent.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;

use common::{body_json, build_app, get_request, json_with_token, setup_and_login};

/// A text asset uploaded (with auth) is then served over the public file
/// channel with NO credentials — 200 + bytes + Content-Type + Cache-Control.
#[tokio::test]
async fn workshop_file_channel_serves_without_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pass123").await;

    // Register a text asset (authenticated management route).
    let create = json_with_token(
        "POST",
        "/api/creative-studio/assets",
        serde_json::json!({ "kind": "text", "title": "notes", "text_content": "hello workshop" }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(create).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "text asset should be created");
    let json = body_json(resp).await;
    let asset_id = json["data"]["asset_id"]
        .as_str()
        .expect("asset id")
        .to_owned();
    assert!(json["data"].get("id").is_none());
    nomifun_common::WorkshopAssetId::parse(&asset_id)
        .expect("asset id must be a bare UUIDv7");

    // Serve it back over the PUBLIC channel with no auth header / cookie.
    let resp = app
        .clone()
        .oneshot(get_request(&format!("/api/creative-studio/files/{asset_id}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "public file serve must not require auth");
    assert_eq!(
        resp.headers()[header::CONTENT_TYPE],
        "text/plain; charset=utf-8",
        "correct Content-Type"
    );
    assert_eq!(
        resp.headers()[header::CACHE_CONTROL],
        "private, max-age=3600",
        "Cache-Control present"
    );
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..], b"hello workshop");
}

/// An unknown id on the public channel is a clean 404 — NOT 401/403. A 401/403
/// would mean the route is still auth-gated (the very failure mode this split
/// exists to avoid).
#[tokio::test]
async fn workshop_public_serve_missing_is_404_not_auth_rejected() {
    let (app, _services) = build_app().await;

    for uri in ["/api/creative-studio/files/0190f5fe-7c00-7a00-8000-000000009991"] {
        let resp = app.clone().oneshot(get_request(uri)).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "{uri} must be a clean 404 (auth-exempt), got {}",
            resp.status()
        );
    }
}

/// Every Creative Studio management route stays authenticated: unauthenticated
/// GETs and writes are rejected (401/403), never served.
#[tokio::test]
async fn workshop_management_routes_still_require_auth() {
    let (app, _services) = build_app().await;

    for uri in ["/api/creative-studio/assets"] {
        let resp = app.clone().oneshot(get_request(uri)).await.unwrap();
        assert!(
            resp.status() == StatusCode::UNAUTHORIZED || resp.status() == StatusCode::FORBIDDEN,
            "{uri} must stay auth-gated, got {}",
            resp.status()
        );
    }

    // A write route without auth is likewise rejected.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/creative-studio/assets")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::UNAUTHORIZED || resp.status() == StatusCode::FORBIDDEN,
        "POST create asset must stay auth-gated, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn retired_workshop_and_unowned_creation_routes_are_not_mounted() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pass123").await;

    async fn database_snapshot(database: &nomifun_db::Database) -> (i64, i64, i64, i64) {
        nomifun_db::sqlx::query_as(
            "SELECT \
                (SELECT COUNT(*) FROM sqlite_master \
                    WHERE type = 'table' AND name = 'workshop_canvases'), \
                (SELECT COUNT(*) FROM creative_studio_projects), \
                (SELECT COUNT(*) FROM creation_tasks), \
                (SELECT COUNT(*) FROM workshop_assets)",
        )
        .fetch_one(database.pool())
        .await
        .unwrap()
    }

    let before = database_snapshot(&services.database).await;
    assert_eq!(before.0, 0, "the retired workshop_canvases table must stay dropped");

    // This is the exact historical Method + URI surface, not a GET-only sample.
    // Authenticated requests bypass the auth/CSRF boundary so a 404 can only be
    // the app router's unmatched-route fallback. The empty body distinguishes
    // that fallback from a mounted resource handler returning domain NotFound.
    for (method, uri, body) in [
        ("GET", "/api/workshop/canvases", serde_json::json!({})),
        (
            "POST",
            "/api/workshop/canvases",
            serde_json::json!({ "title": "retired canvas" }),
        ),
        (
            "GET",
            "/api/workshop/canvases/0190f5fe-7c00-7a00-8000-000000009991",
            serde_json::json!({}),
        ),
        (
            "PATCH",
            "/api/workshop/canvases/0190f5fe-7c00-7a00-8000-000000009991",
            serde_json::json!({ "title": "retired canvas" }),
        ),
        (
            "DELETE",
            "/api/workshop/canvases/0190f5fe-7c00-7a00-8000-000000009991",
            serde_json::json!({}),
        ),
        (
            "PUT",
            "/api/workshop/canvases/0190f5fe-7c00-7a00-8000-000000009991/doc",
            serde_json::json!({ "doc": {} }),
        ),
        (
            "GET",
            "/api/workshop/canvases/0190f5fe-7c00-7a00-8000-000000009991/pending-ops",
            serde_json::json!({}),
        ),
        (
            "POST",
            "/api/workshop/canvases/0190f5fe-7c00-7a00-8000-000000009991/pending-ops/ack",
            serde_json::json!({ "op_ids": [] }),
        ),
        (
            "GET",
            "/api/workshop/canvas-thumbs/0190f5fe-7c00-7a00-8000-000000009991",
            serde_json::json!({}),
        ),
        ("GET", "/api/creation/tasks", serde_json::json!({})),
        (
            "POST",
            "/api/creation/tasks",
            serde_json::json!({
                "canvas_id": "0190f5fe-7c00-7a00-8000-000000009991",
                "node_id": "0190f5fe-7c00-7a00-8000-000000009992",
                "provider_id": "0190f5fe-7c00-7a00-8000-000000009993",
                "model": "retired-model",
                "capability": "t2i",
                "params": {},
                "inputs": []
            }),
        ),
        (
            "GET",
            "/api/creation/tasks/0190f5fe-7c00-7a00-8000-000000009994",
            serde_json::json!({}),
        ),
        (
            "POST",
            "/api/creation/tasks/0190f5fe-7c00-7a00-8000-000000009994/cancel",
            serde_json::json!({}),
        ),
    ] {
        let response = app
            .clone()
            .oneshot(json_with_token(method, uri, body, &token, &csrf))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "retired {method} {uri} must not be mounted"
        );
        let response_body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(
            response_body.is_empty(),
            "retired {method} {uri} must hit the empty router fallback, not a mounted handler"
        );
    }

    let after = database_snapshot(&services.database).await;
    assert_eq!(after, before, "retired HTTP requests must not write Creative Studio data");
}
