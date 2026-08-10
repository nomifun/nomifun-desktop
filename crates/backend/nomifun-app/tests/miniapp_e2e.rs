//! E2E for the 小程序 (mini-app) routes: owner CRUD round-trip, responses that
//! never carry the HTML body, owner isolation, and the auth-exempt serve channel
//! that the runner/preview iframe loads.
mod common;

use axum::http::{header, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

use common::{
    body_json, build_app, delete_with_token, get_request, get_with_token, json_with_token,
    setup_and_login,
};

const APP_HTML: &str = "<!doctype html><html><body><h1>Pomodoro</h1><script>var x=1;</script></body></html>";

fn timer_app_body() -> serde_json::Value {
    json!({
        "name": "Pomodoro",
        "description": "25/5 focus timer",
        "icon": "⏱",
        "html": APP_HTML
    })
}

async fn create_app(
    app: &axum::Router,
    body: serde_json::Value,
    token: &str,
    csrf: &str,
) -> String {
    let req = json_with_token("POST", "/api/miniapps", body, token, csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    body_json(resp).await["data"]["miniapp_id"]
        .as_str()
        .expect("miniapp_id")
        .to_string()
}

#[tokio::test]
async fn unauthenticated_management_requests_are_rejected() {
    let (app, _services) = build_app().await;
    let req = get_with_token("/api/miniapps", "not-a-real-token");
    let resp = app.oneshot(req).await.unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::OK,
        "unauthenticated list must not succeed"
    );
}

#[tokio::test]
async fn owner_can_crud_and_responses_never_carry_the_html() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pw-123456").await;

    // Create
    let req = json_with_token("POST", "/api/miniapps", timer_app_body(), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let created = body_json(resp).await;
    assert_eq!(created["success"], true);
    let data = &created["data"];
    let miniapp_id = data["miniapp_id"].as_str().expect("miniapp_id").to_string();
    nomifun_common::MiniAppId::parse(miniapp_id.clone()).expect("bare UUIDv7");
    assert_eq!(data["name"], "Pomodoro");
    assert_eq!(data["description"], "25/5 focus timer");
    assert_eq!(data["icon"], "⏱");
    assert_eq!(data["source_conversation_id"], serde_json::Value::Null);
    assert_eq!(data["html_size"], APP_HTML.len());
    // The body has exactly one consumer, and it is the serve route.
    assert!(data.get("html").is_none(), "html leaked: {created}");
    assert!(
        !created.to_string().contains("<script>"),
        "html leaked: {created}"
    );

    // List shows it, still without the body.
    let req = get_with_token("/api/miniapps", &token);
    let resp = app.clone().oneshot(req).await.unwrap();
    let listed = body_json(resp).await;
    let rows = listed["data"].as_array().expect("array");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].get("html").is_none(), "html leaked: {listed}");

    // Read one
    let req = get_with_token(&format!("/api/miniapps/{miniapp_id}"), &token);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let one = body_json(resp).await;
    assert_eq!(one["data"]["miniapp_id"], miniapp_id);
    assert!(one["data"].get("html").is_none(), "html leaked: {one}");

    // Update the name only; the stored document must survive untouched.
    let req = json_with_token(
        "PUT",
        &format!("/api/miniapps/{miniapp_id}"),
        json!({ "name": "Pomodoro Pro" }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let updated = body_json(resp).await;
    assert_eq!(updated["data"]["name"], "Pomodoro Pro");
    assert_eq!(updated["data"]["html_size"], APP_HTML.len());

    // Re-solidify with a new document.
    let next_html = "<!doctype html><title>v2</title>";
    let req = json_with_token(
        "PUT",
        &format!("/api/miniapps/{miniapp_id}"),
        json!({ "html": next_html }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["data"]["html_size"], next_html.len());

    // Delete answers with `true`, not an empty object.
    let req = delete_with_token(&format!("/api/miniapps/{miniapp_id}"), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let deleted = body_json(resp).await;
    assert_eq!(deleted["data"], true);

    // Gone
    let req = get_with_token("/api/miniapps", &token);
    let resp = app.oneshot(req).await.unwrap();
    let listed = body_json(resp).await;
    assert_eq!(listed["data"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn list_is_sorted_by_most_recently_updated() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pw-123456").await;

    let mut ids = Vec::new();
    for name in ["first", "second", "third"] {
        ids.push(
            create_app(
                &app,
                json!({ "name": name, "html": "<p/>" }),
                &token,
                &csrf,
            )
            .await,
        );
    }
    // Touch the oldest one; it must climb to the top of the library grid.
    // `updated_at` has millisecond resolution and all three rows were created
    // inside the same millisecond, so give the touch a strictly greater stamp
    // instead of trusting the clock (the repository test hand-stamps for the
    // same reason).
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let req = json_with_token(
        "PUT",
        &format!("/api/miniapps/{}", ids[0]),
        json!({ "description": "touched" }),
        &token,
        &csrf,
    );
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::OK
    );

    let resp = app
        .oneshot(get_with_token("/api/miniapps", &token))
        .await
        .unwrap();
    let listed = body_json(resp).await;
    let names: Vec<&str> = listed["data"]
        .as_array()
        .expect("array")
        .iter()
        .map(|row| row["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names[0], "first", "most recently updated first: {listed}");
}

#[tokio::test]
async fn another_owners_app_is_indistinguishable_from_absent() {
    let (mut app, services) = build_app().await;
    let (owner_token, owner_csrf) =
        setup_and_login(&mut app, &services, "admin", "pw-123456").await;
    let miniapp_id = create_app(&app, timer_app_body(), &owner_token, &owner_csrf).await;

    let (stranger_token, stranger_csrf) =
        setup_and_login(&mut app, &services, "stranger", "pw-654321").await;

    // A non-owner is stopped by the instance-owner guard before the row is even
    // looked up, so every verb must refuse — never 200.
    let attempts = vec![
        get_with_token(&format!("/api/miniapps/{miniapp_id}"), &stranger_token),
        get_with_token("/api/miniapps", &stranger_token),
        json_with_token(
            "PUT",
            &format!("/api/miniapps/{miniapp_id}"),
            json!({ "name": "stolen" }),
            &stranger_token,
            &stranger_csrf,
        ),
        delete_with_token(
            &format!("/api/miniapps/{miniapp_id}"),
            &stranger_token,
            &stranger_csrf,
        ),
    ];
    for req in attempts {
        let uri = req.uri().to_string();
        let method = req.method().to_string();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_ne!(
            resp.status(),
            StatusCode::OK,
            "{method} {uri} must not succeed for a non-owner"
        );
    }

    // And the owner's app is still intact.
    let resp = app
        .oneshot(get_with_token(
            &format!("/api/miniapps/{miniapp_id}"),
            &owner_token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["data"]["name"], "Pomodoro");
}

/// The document is served with NO credentials — an `<iframe src>` load carries no
/// trust header, so an authenticated route would blank every mini-app.
#[tokio::test]
async fn serve_channel_returns_the_html_without_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pw-123456").await;
    let miniapp_id = create_app(&app, timer_app_body(), &token, &csrf).await;

    let resp = app
        .oneshot(get_request(&format!("/api/miniapps/{miniapp_id}/serve")))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "public serve must not require auth"
    );
    assert_eq!(
        resp.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    // Revalidate every load: the same id serves a new document after each
    // re-solidify, and an iterating user must see what they just saved.
    assert_eq!(resp.headers()[header::CACHE_CONTROL], "private, no-cache");
    // Frame-denying this response would blank the runner and the preview panel;
    // instead the document is confined by a CSP the middleware owns.
    assert!(
        resp.headers().get("x-frame-options").is_none(),
        "the served document must be embeddable"
    );
    let policy = resp.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .expect("ascii policy");
    // `sandbox` WITHOUT allow-same-origin: an AI-generated document must never
    // run with the deployment's own origin authority — in WebUI mode that origin
    // holds the session cookie and the whole API.
    assert!(
        policy.contains("sandbox allow-scripts allow-forms allow-popups allow-modals"),
        "serve response must sandbox the document: {policy}"
    );
    assert!(
        !policy.contains("allow-same-origin"),
        "allow-same-origin would hand the document the session's origin: {policy}"
    );
    // And only our own origins may frame it.
    assert!(
        policy.contains("frame-ancestors 'self' tauri: http://tauri.localhost https://tauri.localhost"),
        "serve response must restrict framers: {policy}"
    );
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(std::str::from_utf8(&bytes).unwrap(), APP_HTML);
}

/// An unknown id on the serve channel is a clean 404 — NOT 401/403. A 401/403
/// would mean the route is still auth-gated (the very failure mode this split
/// exists to avoid).
#[tokio::test]
async fn serve_channel_missing_id_is_404_not_auth_rejected() {
    let (app, _services) = build_app().await;
    let resp = app
        .oneshot(get_request(
            "/api/miniapps/0190f5fe-7c00-7a00-8000-000000009991/serve",
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "an unknown app must be a clean 404 (auth-exempt), got {}",
        resp.status()
    );
}

#[tokio::test]
async fn invalid_requests_are_refused_with_bad_request() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pw-123456").await;
    let miniapp_id = create_app(&app, timer_app_body(), &token, &csrf).await;

    let cases = vec![
        // An unknown field is a client/server contract drift, not a field to ignore.
        (
            "POST".to_string(),
            "/api/miniapps".to_string(),
            json!({ "name": "x", "html": "<p/>", "bogus": 1 }),
        ),
        // A blank name would render an unclickable card.
        (
            "POST".to_string(),
            "/api/miniapps".to_string(),
            json!({ "name": "   ", "html": "<p/>" }),
        ),
        // An empty app is not an app.
        (
            "POST".to_string(),
            "/api/miniapps".to_string(),
            json!({ "name": "x", "html": "  " }),
        ),
        // Provenance must be a canonical bare UUIDv7, or the column CHECK would
        // surface as a 500 instead of a field error.
        (
            "POST".to_string(),
            "/api/miniapps".to_string(),
            json!({ "name": "x", "html": "<p/>", "source_conversation_id": "conv-1" }),
        ),
        // An update that changes nothing is a client bug; answering 200 hides it.
        (
            "PUT".to_string(),
            format!("/api/miniapps/{miniapp_id}"),
            json!({}),
        ),
    ];
    for (method, uri, body) in cases {
        let req = json_with_token(&method, &uri, body.clone(), &token, &csrf);
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "{method} {uri} {body} must be a 400"
        );
    }
}

#[tokio::test]
async fn a_solidified_app_records_its_source_conversation() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pw-123456").await;

    let create = json_with_token(
        "POST",
        "/api/conversations",
        json!({ "name": "小程序·计时器", "type": "nomi", "extra": { "miniapp": true } }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(create).await.unwrap();
    assert!(resp.status().is_success(), "conversation create");
    let conversation = body_json(resp).await;
    let conversation_id = conversation["data"]["id"]
        .as_str()
        .or_else(|| conversation["data"]["conversation_id"].as_str())
        .unwrap_or_else(|| panic!("conversation id in {conversation}"))
        .to_string();

    let req = json_with_token(
        "POST",
        "/api/miniapps",
        json!({
            "name": "Timer",
            "html": APP_HTML,
            "source_conversation_id": conversation_id
        }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let created = body_json(resp).await;
    assert_eq!(created["data"]["source_conversation_id"], conversation_id);
}
