//! Regression checks for product surfaces that have been intentionally removed.

mod common;

use axum::http::StatusCode;
use tower::ServiceExt;

use common::{build_app, get_with_token, setup_and_login};

#[tokio::test]
async fn removed_console_home_api_is_not_registered() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let resp = app
        .oneshot(get_with_token("/api/console/home", &token))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// The per-session usage endpoint mirrored the ACP SDK `UsageUpdate` schema and
/// reported `None` for every other engine. No client ever read it.
#[tokio::test]
async fn removed_conversation_usage_api_is_not_registered() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let resp = app
        .oneshot(get_with_token(
            "/api/conversations/0190f5fe-7c00-7a00-8000-0000000000ff/usage",
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// OpenClaw runtime diagnostics were a dead lane end to end: the route, the
/// service method and the ipcBridge binding existed, but nothing in the UI ever
/// called them.
#[tokio::test]
async fn removed_openclaw_runtime_api_is_not_registered() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let resp = app
        .oneshot(get_with_token(
            "/api/conversations/0190f5fe-7c00-7a00-8000-0000000000ff/openclaw/runtime",
            &token,
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
