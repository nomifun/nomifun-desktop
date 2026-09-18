//! E2E integration tests for the retained Terminal workspace route.

mod common;

use axum::http::StatusCode;
use serde_json::json;
use tower::ServiceExt;

use common::{body_json, get_with_token, setup_and_login};

async fn build_app() -> (axum::Router, nomifun_app::compatibility::AppServices) {
    let root = tempfile::Builder::new()
        .prefix("nomifun-auxiliary-e2e-")
        .tempdir()
        .unwrap()
        .keep();
    let db = nomifun_db::init_database_memory().await.unwrap();
    let services = nomifun_app::compatibility::AppServices::from_config(
        db,
        &nomifun_app::AppConfig {
            data_dir: root.join("data"),
            work_dir: root.join("work"),
            ..nomifun_app::AppConfig::default()
        },
    )
    .await
    .unwrap();
    let router = nomifun_app::compatibility::create_router(&services).await;
    (router, services)
}

async fn setup_owner(
    app: &mut axum::Router,
    services: &nomifun_app::compatibility::AppServices,
) -> (String, String) {
    setup_and_login(app, services, "admin", "StrongP@ss1").await
}

/// Create a terminal session row without a live PTY. `defer_spawn: true`
/// persists the row and defers the PTY until the first resize.
async fn create_terminal_with_cwd(
    app: &mut axum::Router,
    token: &str,
    csrf: &str,
    cwd: &str,
) -> String {
    let req = common::json_with_token(
        "POST",
        "/api/terminals",
        json!({
            "name": "Test Terminal",
            "cwd": cwd,
            "command": "cat",
            "defer_spawn": true
        }),
        token,
        csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "terminal create should succeed"
    );
    let json = common::body_json(resp).await;
    json["data"]["terminal_id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn terminal_workspace_requires_auth() {
    let (app, _) = build_app().await;
    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/api/terminals/0190f5fe-7c00-7a00-8abc-012345678901/workspace?path=")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn terminal_workspace_lists_cwd_entries() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_owner(&mut app, &services).await;

    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("hello.txt"), b"hi").unwrap();
    let cwd = tmp.path().to_string_lossy().into_owned();
    let terminal_id = create_terminal_with_cwd(&mut app, &token, &csrf, &cwd).await;

    let req = get_with_token(
        &format!("/api/terminals/{terminal_id}/workspace?path="),
        &token,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let entries = json["data"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["name"], "hello.txt");
    assert_eq!(entries[0]["type"], "file");
}

#[tokio::test]
async fn terminal_workspace_not_found() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_owner(&mut app, &services).await;
    let req = get_with_token(
        "/api/terminals/0190f5fe-7c00-7a00-8abc-012345679999/workspace?path=",
        &token,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
