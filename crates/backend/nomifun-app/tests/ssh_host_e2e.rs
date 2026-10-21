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
async fn ssh_session_launch_freezes_only_exec_for_an_owned_host_without_connecting() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "fixture-password").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let host_id = create_host(&app, json!({
        "name":"SSH launch fixture", "host":"127.0.0.1", "port":port,
        "username":"fixture", "authType":"password"
    }), &token, &csrf).await;
    let response = app.clone().oneshot(json_with_token("POST", "/api/providers", json!({
        "platform":"openai", "name":"No-request SSH launch model",
        "base_url":"https://example.invalid/v1", "auth_scheme":"bearer",
        "credentials":{"api_keys":["fixture-only"]}, "enabled":true,
        "initial_model":{"model":"ssh-launch-model", "enabled":true, "capabilities":[{
            "task":"chat", "traits":[], "protocol":"openai.chat_text",
            "connection_role":"default", "context_limit":128000
        }]}
    }), &token, &csrf)).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let provider = body_json(response).await;
    let model = json!({"provider_id":provider["data"]["provider_id"], "model":"ssh-launch-model"});
    let key = uuid::Uuid::now_v7().to_string();
    let request = || {
        let mut request = json_with_token("POST", "/api/agent-sessions/ssh",
            json!({"ssh_host_id":host_id, "model":model}), &token, &csrf);
        request.headers_mut().insert("idempotency-key", key.parse().unwrap());
        request
    };
    let response = app.clone().oneshot(request()).await.unwrap();
    let status = response.status();
    let opened = body_json(response).await;
    assert_eq!(status, StatusCode::OK, "{opened}");
    let session = opened["data"]["agent_session_id"].as_str().unwrap();
    let response = app.clone().oneshot(request()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["data"], opened["data"], "same-key creation reuses the canonical receipt");
    let binding = &opened["data"]["agent_binding"];
    assert!(binding["typed_resource_bindings"].as_array().unwrap().iter().any(|resource|
        resource["resource_kind"] == "ssh_host" && resource["resource_id"] == host_id
            && resource["owner_id"] == services.authoritative_user_id.as_ref()));
    let response = app.clone().oneshot(get_with_token(
        &format!("/api/agent-sessions/{session}/projection"), &token)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let projection = body_json(response).await;
    assert_eq!(projection["data"]["agent_snapshot"]["enabled_capability_actions"]["ssh"], json!(["ssh/exec"]));
    let store = nomifun_agent_session::AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
    let session_id = nomifun_agent_contracts::AgentSessionId::from(session.to_owned());
    let canonical = store.get_live_session(&session_id).await.unwrap();
    assert_eq!(serde_json::to_value(canonical.agent_binding).unwrap(), *binding);
    assert!(store.head(&session_id).await.unwrap().active_turn_id.is_none());
    assert_eq!(services.ssh_pool.active_link_count(), 0);
    assert!(tokio::time::timeout(std::time::Duration::from_millis(20), listener.accept()).await.is_err(),
        "preparing the SSH Session must not connect to the selected host");
    let response = app.clone().oneshot(get_with_token("/api/agent-preset-templates", &token)).await.unwrap();
    let library = body_json(response).await;
    assert_eq!(library["data"]["user_presets"], json!([]), "SSH preparation creates no visible personal Agent");
    assert!(library["data"]["official_presets"]["chat.minimal"]["current_stable_revision"].is_null());

    let foreign_owner = nomifun_common::UserId::new();
    let foreign = services.ssh_pool.host_service().create(foreign_owner.as_str(),
        serde_json::from_value(json!({"name":"Other owner's host", "host":"127.0.0.1", "port":port,
            "username":"fixture", "authType":"password"})).unwrap()).await.unwrap();
    let response = app.clone().oneshot(json_with_token("POST", "/api/agent-sessions/ssh",
        json!({"ssh_host_id":foreign.ssh_host_id, "model":model}), &token, &csrf)).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND, "host ownership is checked before creating a Session");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions").fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(count, 1);
    services.cleanup_after_startup_failure(anyhow::anyhow!("SSH Session launch fixture ended")).await;
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
