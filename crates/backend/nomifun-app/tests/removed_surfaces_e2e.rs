//! Regression checks for product surfaces that have been intentionally removed.

mod common;

use axum::http::StatusCode;
use serde_json::json;
use tower::ServiceExt;

use common::{build_app, delete_with_token, get_with_token, json_with_token, setup_and_login};

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

/// The `ExtAcpAdapter` contribution type went away with ACP itself, so
/// extensions can no longer declare adapters and there is nothing left for this
/// route to list.
#[tokio::test]
async fn removed_extension_acp_adapters_api_is_not_registered() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let resp = app
        .oneshot(get_with_token("/api/extensions/acp-adapters", &token))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// The remote-agent registry went away with the `remote` engine. These routes
/// sat behind `protect_instance_owner`, so a non-owner would have seen 403 —
/// logging in as the canonical owner is what makes a 404 here meaningful.
#[tokio::test]
async fn removed_remote_agent_api_is_not_registered() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let id = "0190f5fe-7c00-7a00-8000-0000000000ff";
    let cases = [
        get_with_token("/api/remote-agents", &token),
        get_with_token(&format!("/api/remote-agents/{id}"), &token),
        json_with_token("POST", "/api/remote-agents", json!({}), &token, &csrf),
        json_with_token(
            "POST",
            "/api/remote-agents/test-connection",
            json!({}),
            &token,
            &csrf,
        ),
        json_with_token(
            "POST",
            &format!("/api/remote-agents/{id}/handshake"),
            json!({}),
            &token,
            &csrf,
        ),
        json_with_token("PUT", &format!("/api/remote-agents/{id}"), json!({}), &token, &csrf),
        delete_with_token(&format!("/api/remote-agents/{id}"), &token, &csrf),
    ];

    for request in cases {
        let uri = request.uri().to_string();
        let method = request.method().to_string();
        let resp = app.clone().oneshot(request).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "{method} {uri} must no longer be routed"
        );
    }
}

/// Per-conversation token usage was only ever populated by the ACP engine and
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

/// Custom agents and the reachability probe both existed to serve the deleted
/// engines: a "custom agent" was a user-registered third-party CLI, and the
/// probe answered "is that CLI installed and launchable". With one built-in
/// engine there is nothing to register and nothing to probe.
///
/// `GET /api/agents`, `POST /api/agents/refresh`, and
/// `POST /api/agents/provider-health-check` deliberately survive — they describe
/// the built-in engine and check MODEL providers, which is a different question.
#[tokio::test]
async fn removed_custom_agent_and_probe_apis_are_not_registered() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let id = "0190f5fe-7c00-7a00-8000-0000000000ff";
    let cases = [
        json_with_token(
            "POST",
            "/api/agents/health-check",
            json!({ "backend": "nomi" }),
            &token,
            &csrf,
        ),
        json_with_token(
            "PATCH",
            &format!("/api/agents/{id}/enabled"),
            json!({ "enabled": false }),
            &token,
            &csrf,
        ),
        json_with_token("POST", "/api/agents/custom", json!({}), &token, &csrf),
        json_with_token(
            "POST",
            "/api/agents/custom/try-connect",
            json!({}),
            &token,
            &csrf,
        ),
    ];

    for request in cases {
        let uri = request.uri().to_string();
        let method = request.method().to_string();
        let resp = app.clone().oneshot(request).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "{method} {uri} must no longer be routed"
        );
    }
}

/// The surviving `/api/agents*` routes must keep working — this test is the
/// other half of the one above, so a future sweep that over-deletes fails here
/// instead of silently removing the only way to describe the built-in engine.
#[tokio::test]
async fn surviving_agent_apis_are_still_registered() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let resp = app
        .clone()
        .oneshot(get_with_token("/api/agents", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "GET /api/agents must survive");

    let resp = app
        .clone()
        .oneshot(json_with_token(
            "POST",
            "/api/agents/refresh",
            json!({}),
            &token,
            &csrf,
        ))
        .await
        .unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "POST /api/agents/refresh must survive"
    );
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
