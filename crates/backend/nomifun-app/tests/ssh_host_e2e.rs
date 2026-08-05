//! E2E for the SSH host-book routes: owner CRUD round-trip, masked secret in
//! responses, and auth/owner protection.
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
