//! E2E for the SSH host-book routes: owner CRUD round-trip, masked secret in
//! responses, auth/owner protection, and the live link-status snapshot.
mod common;

use axum::http::StatusCode;
use serde_json::json;
use tower::ServiceExt;

use common::{body_json, build_app, delete_with_token, get_with_token, json_with_token, setup_and_login};

fn password_host_body() -> serde_json::Value {
    json!({
        "name": "prod-web",
        "host": "10.0.3.21",
        "port": 22,
        "username": "deploy",
        "authType": "password",
        "password": "hunter2_supersecret",
        "sudoPassword": "sudo_secret_pw"
    })
}

/// A host whose password auth has no password. The pool refuses it as an unusable
/// credential *before* opening a socket, which is how these HTTP tests get a link
/// into a real, terminal state without needing an sshd.
fn uncredentialed_host_body() -> serde_json::Value {
    json!({
        "name": "no-credential",
        "host": "127.0.0.1",
        "port": 1,
        "username": "nobody",
        "authType": "password"
    })
}

async fn create_host(
    app: &axum::Router,
    body: serde_json::Value,
    token: &str,
    csrf: &str,
) -> String {
    let req = json_with_token("POST", "/api/ssh-hosts", body, token, csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    body_json(resp).await["data"]["sshHostId"]
        .as_str()
        .expect("sshHostId")
        .to_string()
}

#[tokio::test]
async fn unauthenticated_requests_are_rejected() {
    let (app, _services) = build_app().await;
    let req = get_with_token("/api/ssh-hosts", "not-a-real-token");
    let resp = app.oneshot(req).await.unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::OK,
        "unauthenticated list must not succeed"
    );
}

#[tokio::test]
async fn owner_can_crud_and_secret_is_masked() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pw-123456").await;

    // Create
    let req = json_with_token("POST", "/api/ssh-hosts", password_host_body(), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let created = body_json(resp).await;
    assert_eq!(created["success"], true);
    let data = &created["data"];
    let host_id = data["sshHostId"].as_str().expect("sshHostId").to_string();
    // Plaintext must never appear; presence is masked.
    let raw = created.to_string();
    assert!(!raw.contains("hunter2_supersecret"), "password leaked: {raw}");
    assert!(!raw.contains("sudo_secret_pw"), "sudo password leaked: {raw}");
    assert_eq!(data["password"], "***");
    assert_eq!(data["sudoPassword"], "***");
    assert_eq!(data["host"], "10.0.3.21");

    // List shows it
    let req = get_with_token("/api/ssh-hosts", &token);
    let resp = app.clone().oneshot(req).await.unwrap();
    let listed = body_json(resp).await;
    assert_eq!(listed["data"].as_array().unwrap().len(), 1);

    // Update the name only, resending the mask for the password (unchanged)
    let req = json_with_token(
        "PUT",
        &format!("/api/ssh-hosts/{host_id}"),
        json!({ "name": "prod-web-renamed", "password": "***" }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let updated = body_json(resp).await;
    assert_eq!(updated["data"]["name"], "prod-web-renamed");
    assert_eq!(updated["data"]["password"], "***");

    // Delete
    let req = delete_with_token(&format!("/api/ssh-hosts/{host_id}"), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Gone
    let req = get_with_token("/api/ssh-hosts", &token);
    let resp = app.oneshot(req).await.unwrap();
    let listed = body_json(resp).await;
    assert_eq!(listed["data"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn unknown_field_is_rejected() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pw-123456").await;
    let req = json_with_token(
        "POST",
        "/api/ssh-hosts",
        json!({ "name": "x", "host": "h", "username": "u", "authType": "password", "bogus": 1 }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn statuses_snapshot_is_owner_scoped_and_matches_the_link_state() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pw-123456").await;
    let host_id = create_host(&app, uncredentialed_host_body(), &token, &csrf).await;
    let parsed = nomifun_common::SshHostId::parse(host_id.clone()).expect("host id");

    // Nothing has been dialled yet: an empty list, not a 404 and not an error.
    let resp = app
        .clone()
        .oneshot(get_with_token("/api/ssh-hosts/statuses", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let before = body_json(resp).await;
    assert_eq!(before["success"], true);
    assert_eq!(before["data"].as_array().expect("array").len(), 0);

    // One link for the owner, one for somebody else, in the pool the router
    // actually reads. Both dials fail on the missing password, which is what puts
    // them into a state worth reporting.
    let owner = services.authoritative_user_id.to_string();
    let stranger = nomifun_common::UserId::new().as_str().to_string();
    for (user_id, conversation_id) in [
        (owner.as_str(), "conv-owner"),
        (stranger.as_str(), "conv-stranger"),
    ] {
        services
            .ssh_pool
            .acquire(user_id, conversation_id, &parsed, "/")
            .await
            .expect_err("password auth without a password cannot dial");
    }
    assert_eq!(
        services.ssh_pool.active_link_count(),
        2,
        "both failed dials must still leave a link the pool knows about"
    );

    let resp = app
        .clone()
        .oneshot(get_with_token("/api/ssh-hosts/statuses", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let rows = body["data"].as_array().expect("array");
    assert_eq!(
        rows.len(),
        1,
        "another owner's link must be invisible here: {body}"
    );
    let row = &rows[0];
    assert_eq!(row["conversationId"], "conv-owner");
    assert_eq!(row["sshHostId"], host_id);
    // The snapshot is the same projection the realtime event carries, so it must
    // spell out the terminal drop rather than leaving the client to guess.
    assert_eq!(row["state"], "dropped");
    assert_eq!(row["retryable"], false);
    assert_eq!(row["reaped"], serde_json::Value::Null);
    assert!(
        row["detail"].as_str().is_some_and(|d| !d.is_empty()),
        "a dropped link owes the operator a reason: {row}"
    );
    // Whatever the reason says, it must not be the credential itself.
    let raw = body.to_string();
    assert!(!raw.contains("password_encrypted"), "{raw}");
}

#[tokio::test]
async fn statuses_route_does_not_shadow_the_ssh_host_id_capture() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pw-123456").await;
    let host_id = create_host(&app, password_host_body(), &token, &csrf).await;

    // `statuses` is a literal segment sitting exactly where `{ssh_host_id}` also
    // matches. axum prefers the literal, which is only safe as long as no real
    // host id can be spelled `statuses` — a uuid never can. Pinned here because
    // the failure mode is a silent one: the single-host GET would start answering
    // with a list.
    let resp = app
        .clone()
        .oneshot(get_with_token(&format!("/api/ssh-hosts/{host_id}"), &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let one = body_json(resp).await;
    assert_eq!(one["data"]["sshHostId"], host_id);
    assert_eq!(one["data"]["name"], "prod-web");
    assert!(
        one["data"].is_object(),
        "the id capture must still reach the single-host handler: {one}"
    );

    let resp = app
        .clone()
        .oneshot(get_with_token("/api/ssh-hosts/statuses", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let listed = body_json(resp).await;
    assert!(
        listed["data"].is_array(),
        "the literal segment is its own route: {listed}"
    );
}
