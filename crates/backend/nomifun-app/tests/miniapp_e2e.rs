//! E2E for the 小程序 (mini-app) routes: owner CRUD round-trip, responses that
//! never carry the HTML body, owner isolation, the auth-exempt serve channel that
//! the runner/preview iframe loads, the idempotent workspace provision 「继续迭代」
//! calls before it starts an ordinary conversation, and the publish action that
//! promotes the on-disk working copy into the served snapshot.
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

async fn read_app(app: &axum::Router, miniapp_id: &str, token: &str) -> serde_json::Value {
    let resp = app
        .clone()
        .oneshot(get_with_token(
            &format!("/api/miniapps/{miniapp_id}"),
            token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    body_json(resp).await["data"].clone()
}

async fn serve_body(app: &axum::Router, miniapp_id: &str) -> String {
    let resp = app
        .clone()
        .oneshot(get_request(&format!("/api/miniapps/{miniapp_id}/serve")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).expect("utf-8 document")
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
        json_with_token(
            "POST",
            &format!("/api/miniapps/{miniapp_id}/publish"),
            json!({}),
            &stranger_token,
            &stranger_csrf,
        ),
        json_with_token(
            "POST",
            &format!("/api/miniapps/{miniapp_id}/workspace"),
            json!({}),
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

    // And nothing was created on the owner's behalf on the way to those refusals.
    assert!(
        !nomifun_miniapp::miniapp_workspace_dir(&services.work_dir, &miniapp_id).exists(),
        "a non-owner must not be able to mkdir the owner's workspace"
    );

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

/// The whole two-layer story in one pass: the snapshot the runner serves, the
/// working copy a conversation edits, and the one explicit act that crosses
/// between them.
#[tokio::test]
async fn publishing_promotes_the_working_copy_into_the_served_document() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pw-123456").await;
    let miniapp_id = create_app(&app, timer_app_body(), &token, &csrf).await;

    // A freshly solidified app has no directory at all, so there is nothing to
    // publish and nothing to badge.
    let fresh = read_app(&app, &miniapp_id, &token).await;
    assert_eq!(fresh["has_unpublished_changes"], false);
    assert_eq!(fresh["published_at"], serde_json::Value::Null);

    // Entering the iterate flow materializes the working copy — at the path the
    // shared formula names, since that is the only place either layer looks.
    let source_path = provision_workspace(&app, &miniapp_id, &token, &csrf).await;
    let source = std::path::PathBuf::from(&source_path);
    assert_eq!(
        source,
        nomifun_miniapp::miniapp_workspace_dir(&services.work_dir, &miniapp_id)
            .join(nomifun_miniapp::MINIAPP_SOURCE_FILE)
    );
    assert_eq!(
        std::fs::read_to_string(&source).expect("working copy on disk"),
        APP_HTML
    );

    // Byte-identical to the snapshot at this instant: the detail page must not
    // open claiming there is something to publish.
    let materialized = read_app(&app, &miniapp_id, &token).await;
    assert_eq!(
        materialized["has_unpublished_changes"], false,
        "a just-materialized working copy IS the published document: {materialized}"
    );
    let stamped = materialized["published_at"]
        .as_i64()
        .unwrap_or_else(|| panic!("materializing must stamp published_at: {materialized}"));

    // Re-entering is a no-op — this is the path a second 「继续迭代」 takes, and it
    // must never overwrite work in progress.
    assert_eq!(
        provision_workspace(&app, &miniapp_id, &token, &csrf).await,
        source_path
    );

    // Now an editing turn rewrites the file in place. The sleep is not decoration:
    // the flag compares millisecond mtimes, so an edit in the same millisecond as
    // the publish stamp would be a coin flip.
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let next_html = "<!doctype html><html><body><h1>Pomodoro v2</h1></body></html>";
    std::fs::write(&source, next_html).expect("a conversation writes the working copy");

    let iterated = read_app(&app, &miniapp_id, &token).await;
    assert_eq!(
        iterated["has_unpublished_changes"], true,
        "an edited working copy must be reported: {iterated}"
    );
    // Crucially the runner keeps working while the app is being rewritten — the
    // user's live tool does not break because a turn is mid-flight.
    assert_eq!(serve_body(&app, &miniapp_id).await, APP_HTML);

    // Publish.
    let req = json_with_token(
        "POST",
        &format!("/api/miniapps/{miniapp_id}/publish"),
        json!({}),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let published = body_json(resp).await;
    assert_eq!(published["success"], true);
    let data = &published["data"];
    assert_eq!(data["html_size"], next_html.len());
    assert_eq!(
        data["has_unpublished_changes"], false,
        "the publish response must already reflect its own effect: {published}"
    );
    assert!(
        data["published_at"].as_i64().expect("published_at") >= stamped,
        "publishing moves the stamp forward: {published}"
    );
    // The response still carries no body, publish or not.
    assert!(data.get("html").is_none(), "html leaked: {published}");
    assert!(
        !published.to_string().contains("<h1>"),
        "html leaked: {published}"
    );

    // The runner now serves the new document, and a fresh read agrees the flag is
    // clear (it is derived per request, not cached).
    assert_eq!(serve_body(&app, &miniapp_id).await, next_html);
    let settled = read_app(&app, &miniapp_id, &token).await;
    assert_eq!(settled["has_unpublished_changes"], false);
    // The library grid reads the same derived flag, not a stored one.
    let resp = app
        .oneshot(get_with_token("/api/miniapps", &token))
        .await
        .unwrap();
    let listed = body_json(resp).await;
    assert_eq!(listed["data"][0]["has_unpublished_changes"], false);
    assert!(
        listed["data"][0].get("html").is_none(),
        "html leaked: {listed}"
    );
}

/// Nothing to publish is a 400, not a 404 and not a 500: the app exists, the user
/// simply has not iterated on it yet, and the message has to say so.
#[tokio::test]
async fn publishing_without_a_working_copy_is_a_bad_request() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pw-123456").await;
    let miniapp_id = create_app(&app, timer_app_body(), &token, &csrf).await;

    let req = json_with_token(
        "POST",
        &format!("/api/miniapps/{miniapp_id}/publish"),
        json!({}),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let refused = body_json(resp).await.to_string();
    assert!(
        refused.contains("nothing to publish"),
        "the refusal must be actionable: {refused}"
    );

    // The snapshot is untouched, so the app the user already has keeps working.
    assert_eq!(serve_body(&app, &miniapp_id).await, APP_HTML);
}

/// An unknown id on publish is a 404 — the same shape every other owner-scoped
/// verb gives, so a probe cannot tell "absent" from "someone else's".
#[tokio::test]
async fn publishing_an_unknown_app_is_not_found() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pw-123456").await;

    let req = json_with_token(
        "POST",
        "/api/miniapps/0190f5fe-7c00-7a00-8000-000000009992/publish",
        json!({}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}


// ─── Provisioning the app's workspace ─────────────────────────────────────────

async fn provision_workspace(
    app: &axum::Router,
    miniapp_id: &str,
    token: &str,
    csrf: &str,
) -> String {
    let req = json_with_token(
        "POST",
        &format!("/api/miniapps/{miniapp_id}/workspace"),
        json!({}),
        token,
        csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "provision workspace: {body}");
    body["data"]["source_path"]
        .as_str()
        .unwrap_or_else(|| panic!("source_path in {body}"))
        .to_string()
}

/// The whole server side of 「继续迭代」: an absolute source path, materialized on
/// demand, idempotent — and no conversation anywhere in it. The thread that edits
/// the file is an ordinary conversation the client starts afterwards, which is why
/// this route hands back a path and nothing else.
#[tokio::test]
async fn provisioning_the_workspace_returns_an_absolute_source_path_and_is_idempotent() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pw-123456").await;
    let miniapp_id = create_app(&app, timer_app_body(), &token, &csrf).await;
    let dir = nomifun_miniapp::miniapp_workspace_dir(&services.work_dir, &miniapp_id);
    assert!(
        !dir.exists(),
        "a freshly solidified app has no directory until something asks for one"
    );

    let source_path = provision_workspace(&app, &miniapp_id, &token, &csrf).await;
    let source = std::path::Path::new(&source_path);
    // Absolute, because its reader is a model working in some OTHER conversation's
    // workspace, where a relative path names nothing.
    assert!(source.is_absolute(), "{source_path}");
    assert_eq!(
        source.file_name().and_then(|name| name.to_str()),
        Some(nomifun_miniapp::MINIAPP_SOURCE_FILE)
    );
    assert!(
        source.starts_with(nomifun_miniapp::miniapps_root(&services.work_dir)),
        "the source must stay under the mini-app root: {source_path}"
    );
    // Materialized from the published snapshot, so the model has something to read.
    assert_eq!(
        std::fs::read_to_string(source).expect("working copy on disk"),
        APP_HTML
    );
    // Materializing is not iterating: the badge must not light up for a file the
    // user has never touched.
    assert_eq!(
        read_app(&app, &miniapp_id, &token).await["has_unpublished_changes"],
        false
    );

    // Idempotent, and it never overwrites work in progress — this is the path a
    // second 「继续迭代」 takes. The sleep is not decoration: the unpublished flag
    // compares millisecond mtimes.
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let edited = "<!doctype html><title>edited by an ordinary conversation</title>";
    std::fs::write(source, edited).expect("a conversation edits the working copy");
    assert_eq!(
        provision_workspace(&app, &miniapp_id, &token, &csrf).await,
        source_path,
        "the same app must always resolve to the same source path"
    );
    assert_eq!(
        std::fs::read_to_string(source).expect("working copy"),
        edited,
        "re-provisioning must not throw away the user's changes"
    );

    // A vanished directory is put back rather than named: the user relocated their
    // work dir, or something swept the tree, and the model must not be pointed at
    // a file that is not there.
    std::fs::remove_dir_all(&dir).expect("simulate a vanished workspace");
    assert_eq!(
        provision_workspace(&app, &miniapp_id, &token, &csrf).await,
        source_path
    );
    assert_eq!(
        std::fs::read_to_string(source).expect("re-materialized from the snapshot"),
        APP_HTML
    );
}

/// An unknown id is a 404 here too — the same shape every other owner-scoped verb
/// gives, so a probe cannot tell "absent" from "someone else's".
#[tokio::test]
async fn provisioning_the_workspace_of_an_unknown_app_is_not_found() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pw-123456").await;

    let req = json_with_token(
        "POST",
        "/api/miniapps/0190f5fe-7c00-7a00-8000-000000009994/workspace",
        json!({}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
