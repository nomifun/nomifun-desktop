//! MCP E2E tests beyond CRUD: connection test, agent config discovery, OAuth, auth.
//!
//! Covers test-plan sections 2 (connection test error paths), 3 (agent config discovery),
//! 4 (OAuth status), and 6 (authentication).

mod common;

use axum::http::StatusCode;
use nomifun_api_types::McpServerId;
use serde_json::json;
use tower::ServiceExt;

use common::{body_json, build_app, build_local_trust_app, delete_with_token, get_request, get_with_token, json_with_token, setup_and_login};

const CANONICAL_MCP_SERVER_ID: &str = "0190f5fe-7c00-7a00-8abc-012345679998";

fn assert_mcp_server_id(data: &serde_json::Value) -> McpServerId {
    let raw = data["mcp_server_id"]
        .as_str()
        .expect("MCP response must contain a string mcp_server_id");
    let id = McpServerId::parse(raw).expect("mcp_server_id must be a canonical UUIDv7");
    assert_eq!(raw.len(), 36, "mcp_server_id must be a bare UUIDv7");
    assert_eq!(raw, raw.to_ascii_lowercase(), "mcp_server_id must be lowercase");
    assert!(data.get("id").is_none(), "generic id must not be exposed");
    id
}

// ===========================================================================
// CT-3: Connection test — command not found (ENOENT)
// ===========================================================================

#[tokio::test]
async fn connection_test_enoent_command() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/mcp/test-connection",
        json!({
            "name": "enoent-test",
            "transport": {
                "type": "stdio",
                "command": "nonexistent-mcp-command-xyz-12345"
            }
        }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let json = body_json(resp).await;
    assert!(!json["success"].as_bool().unwrap());
    assert_eq!(json["code"], "MCP_COMMAND_NOT_FOUND");
    assert_eq!(json["details"]["command"], "nonexistent-mcp-command-xyz-12345");
    assert!(!json["error"].as_str().unwrap().is_empty());
}

// ===========================================================================
// CT-4: Connection test — unreachable URL
// ===========================================================================

#[tokio::test]
async fn connection_test_unreachable_url() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/mcp/test-connection",
        json!({
            "name": "unreachable-test",
            "transport": {
                "type": "http",
                "url": "http://127.0.0.1:19999/mcp"
            }
        }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);

    let json = body_json(resp).await;
    assert!(!json["success"].as_bool().unwrap());
    assert_eq!(json["code"], "MCP_CONNECTION_FAILED");
    assert_eq!(json["details"]["transport"], "http");
    assert!(!json["error"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn connection_test_accepts_saved_mcp_server_id_and_persists_failure() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let create = json_with_token(
        "POST",
        "/api/mcp/servers",
        json!({
            "name": "saved-id-test",
            "transport": {
                "type": "stdio",
                "command": "nonexistent-mcp-command-saved-id-test"
            }
        }),
        &token,
        &csrf,
    );
    let created = app.clone().oneshot(create).await.unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let created_json = body_json(created).await;
    let mcp_server_id = assert_mcp_server_id(&created_json["data"]);

    let test = json_with_token(
        "POST",
        "/api/mcp/test-connection",
        json!({
            "mcp_server_id": mcp_server_id,
            "name": "saved-id-test",
            "transport": {
                "type": "stdio",
                "command": "nonexistent-mcp-command-saved-id-test"
            }
        }),
        &token,
        &csrf,
    );
    let tested = app.clone().oneshot(test).await.unwrap();
    assert_eq!(tested.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let tested_json = body_json(tested).await;
    assert_eq!(tested_json["code"], "MCP_COMMAND_NOT_FOUND");

    let listed = app
        .clone()
        .oneshot(get_with_token("/api/mcp/servers", &token))
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let listed_json = body_json(listed).await;
    let persisted = listed_json["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|server| server["mcp_server_id"].as_str() == Some(mcp_server_id.as_str()))
        .unwrap();
    assert_eq!(assert_mcp_server_id(persisted), mcp_server_id);
    assert_eq!(persisted["last_test_status"], "error");
}

#[tokio::test]
async fn connection_test_rejects_legacy_mcp_server_id_shapes() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let transport = json!({
        "type": "http",
        "url": "http://127.0.0.1:19999/mcp"
    });

    let invalid_requests = [
        (
            "generic id",
            json!({
                "id": 1,
                "name": "legacy-generic-id-test",
                "transport": transport.clone()
            }),
        ),
        (
            "integer mcp_server_id",
            json!({
                "mcp_server_id": 1,
                "name": "legacy-integer-id-test",
                "transport": transport.clone()
            }),
        ),
        (
            "numeric string mcp_server_id",
            json!({
                "mcp_server_id": "1",
                "name": "legacy-numeric-string-id-test",
                "transport": transport.clone()
            }),
        ),
        (
            "prefixed UUIDv7 mcp_server_id",
            json!({
                "mcp_server_id": format!("mcp_{CANONICAL_MCP_SERVER_ID}"),
                "name": "legacy-prefixed-id-test",
                "transport": transport.clone()
            }),
        ),
        (
            "UUIDv4 mcp_server_id",
            json!({
                "mcp_server_id": "550e8400-e29b-41d4-a716-446655440000",
                "name": "legacy-uuidv4-test",
                "transport": transport.clone()
            }),
        ),
        (
            "uppercase UUIDv7 mcp_server_id",
            json!({
                "mcp_server_id": CANONICAL_MCP_SERVER_ID.to_ascii_uppercase(),
                "name": "legacy-uppercase-id-test",
                "transport": transport.clone()
            }),
        ),
    ];

    for (shape, body) in invalid_requests {
        let req = json_with_token("POST", "/api/mcp/test-connection", body, &token, &csrf);
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{shape} must be rejected");
    }
}

// ===========================================================================
// AS-1: Get agent configs (may return empty in test env)
// ===========================================================================

#[tokio::test]
async fn get_agent_configs() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let resp = app
        .clone()
        .oneshot(get_with_token("/api/mcp/agent-configs", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(json["success"].as_bool().unwrap());
    // In test env, data is an array (may be empty or contain nomifun adapter)
    assert!(json["data"].is_array());
}

// ===========================================================================
// Installation-owned MCP authentication uses saved server identity only.
// ===========================================================================

async fn saved_http_auth_server(app: &axum::Router, token: &str, csrf: &str) -> McpServerId {
    let response = app.clone().oneshot(json_with_token("POST", "/api/mcp/servers", json!({
        "name":"oauth-contract", "transport":{"type":"http","url":"http://127.0.0.1:9/mcp"}
    }), token, csrf)).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_mcp_server_id(&body_json(response).await["data"])
}

#[tokio::test]
async fn auth_status_uses_saved_server_identity_and_is_independent_of_connectivity() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    let id = saved_http_auth_server(&app, &token, &csrf).await;
    let response = app.clone().oneshot(get_with_token(&format!("/api/mcp/servers/{id}/auth"), &token)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let result = body_json(response).await;
    assert_eq!(result["data"]["state"], "anonymous");
    assert_eq!(result["data"]["granted_scopes"], json!([]));
    assert!(result["data"].get("authenticated").is_none());
    assert!(result["data"].get("connected").is_none());
}

#[tokio::test]
async fn auth_operations_reject_unregistered_server_and_unknown_attempts() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    let missing = format!("/api/mcp/servers/{CANONICAL_MCP_SERVER_ID}/auth");
    for request in [
        get_with_token(&missing, &token),
        json_with_token("POST", &format!("{missing}/attempts"), json!({}), &token, &csrf),
        delete_with_token(&missing, &token, &csrf),
    ] {
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(body_json(response).await["code"], "CONNECTION_CHANGED");
    }
    let id = saved_http_auth_server(&app, &token, &csrf).await;
    let attempt = format!("/api/mcp/servers/{id}/auth/attempts/not-an-existing-attempt");
    for request in [get_with_token(&attempt, &token), delete_with_token(&attempt, &token, &csrf)] {
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(body_json(response).await["code"], "CONNECTION_CHANGED");
    }
}

#[tokio::test]
async fn auth_start_rejects_url_owner_callback_and_unknown_client_fields() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    let id = saved_http_auth_server(&app, &token, &csrf).await;
    let path = format!("/api/mcp/servers/{id}/auth/attempts");
    for body in [
        json!({"server_url":"https://other.example.test/mcp"}),
        json!({"owner_id":"another-owner"}),
        json!({"callback_base_url":"https://attacker.example.test"}),
        json!({"client":{"mode":"auto","unknown_secret":"fixture-secret"}}),
        json!({"client":{"mode":"fixture-secret-variant"}}),
        json!({"client":"fixture-private-secret"}),
    ] {
        let response = app.clone().oneshot(json_with_token("POST", &path, body, &token, &csrf)).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let response = body_json(response).await;
        assert_eq!(response["code"], "INVALID_CONFIGURATION");
        assert_eq!(response["details"]["stage"], "authorization");
        assert!(!response["details"]["trace_id"].as_str().unwrap().is_empty());
        for private in ["fixture-secret", "fixture-private-secret", "attacker.example.test", "another-owner"] {
            assert!(!response.to_string().contains(private), "OAuth JSON rejection must not reflect submitted values");
        }
    }
    for id in ["42", "550e8400-e29b-41d4-a716-446655440000"] {
        let response = app.clone().oneshot(get_with_token(&format!("/api/mcp/servers/{id}/auth"), &token)).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn auth_preparation_failure_is_a_pollable_attempt_with_safe_diagnostics() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    let id = saved_http_auth_server(&app, &token, &csrf).await;
    let path = format!("/api/mcp/servers/{id}/auth/attempts");
    // The initial MCP challenge probe targets only a closed loopback port;
    // production metadata validation then rejects plaintext OAuth discovery.
    // No browser, external endpoint, or user authorization is involved.
    let response = app.clone().oneshot(json_with_token("POST", &path, json!({}), &token, &csrf)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let attempt = body_json(response).await["data"].clone();
    assert_eq!(attempt["status"], "failed");
    assert!(attempt["authorization_url"].is_null());
    assert!(!attempt["diagnostic"]["trace_id"].as_str().unwrap().is_empty());
    assert!(attempt["diagnostic"]["stage"].is_string());
    assert_ne!(attempt["diagnostic"]["code"], "INTERNAL");
    let attempt_id = attempt["attempt_id"].as_str().unwrap();
    let poll_path = format!("{path}/{attempt_id}");
    let polled = app.clone().oneshot(get_with_token(&poll_path, &token)).await.unwrap();
    assert_eq!(polled.status(), StatusCode::OK);
    assert_eq!(body_json(polled).await["data"]["status"], "failed");
    let cancelled = app.clone().oneshot(delete_with_token(&poll_path, &token, &csrf)).await.unwrap();
    assert_eq!(cancelled.status(), StatusCode::OK);
}

#[tokio::test]
async fn auth_logout_is_idempotent_for_saved_server_without_grant() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    let id = saved_http_auth_server(&app, &token, &csrf).await;
    let path = format!("/api/mcp/servers/{id}/auth");
    for _ in 0..2 {
        let response = app.clone().oneshot(delete_with_token(&path, &token, &csrf)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    let response = app.clone().oneshot(get_with_token(&path, &token)).await.unwrap();
    assert_eq!(body_json(response).await["data"]["state"], "anonymous");
}

#[tokio::test]
async fn retired_url_based_auth_endpoints_are_absent() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    for request in [
        get_with_token("/api/mcp/oauth/authenticated", &token),
        json_with_token("POST", "/api/mcp/oauth/login", json!({"server_url":"https://example.test"}), &token, &csrf),
        json_with_token("POST", "/api/mcp/oauth/check-status", json!({"server_url":"https://example.test"}), &token, &csrf),
        json_with_token("POST", "/api/mcp/oauth/logout", json!({"server_url":"https://example.test"}), &token, &csrf),
    ] {
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn native_callback_requires_the_local_trust_header_even_for_an_owner_jwt() {
    const LOCAL_SECRET: &str = "mcp-oauth-local-trust-fixture-only";
    let (mut app, services) = build_local_trust_app(LOCAL_SECRET).await;
    let local_request = |path: &str, body: serde_json::Value| {
        axum::http::Request::builder().method("POST").uri(path)
            .header(nomifun_auth::LOCAL_TRUST_HEADER, LOCAL_SECRET)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap())).unwrap()
    };
    let created = app.clone().oneshot(local_request("/api/mcp/servers", json!({
        "name":"native-callback-boundary", "transport":{"type":"http","url":"http://127.0.0.1:9/mcp"}
    }))).await.unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let id = assert_mcp_server_id(&body_json(created).await["data"]);
    let path = format!("/api/mcp/servers/{id}/auth/attempts");
    let native = app.clone().oneshot(local_request(&path, json!({}))).await.unwrap();
    assert_eq!(native.status(), StatusCode::OK);
    let native = body_json(native).await;
    assert_eq!(native["data"]["status"], "failed");
    assert!(matches!(native["data"]["diagnostic"]["code"].as_str(), Some("METADATA_INVALID" | "UPSTREAM_UNAVAILABLE")),
        "a trusted native request must reach provider discovery without requiring a WebUI HTTPS callback: {native}");
    assert_eq!(native["data"]["diagnostic"]["stage"], "discovery");

    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    let browser = app.clone().oneshot(json_with_token("POST", &path, json!({}), &token, &csrf)).await.unwrap();
    assert_eq!(browser.status(), StatusCode::OK);
    let browser = body_json(browser).await;
    assert_eq!(browser["data"]["status"], "failed");
    assert_eq!(browser["data"]["diagnostic"]["code"], "REDIRECT_UNAVAILABLE",
        "an owner JWT is not proof of the desktop shell; it must use the configured HTTPS callback");
    assert_eq!(browser["data"]["diagnostic"]["stage"], "authorization");
    assert!(browser["data"]["authorization_url"].is_null());
}

#[tokio::test]
async fn public_oauth_callback_rejects_missing_or_unknown_state_without_echoing_query_or_selecting_an_owner() {
    let (app, services) = build_app().await;
    for path in [
        "/api/mcp/oauth/callback".to_owned(),
        format!("/api/mcp/oauth/callback?state=unknown-fixture-state&code=fixture-private-code&server_id={CANONICAL_MCP_SERVER_ID}&owner_id=fixture-forged-owner&iss=https%3A%2F%2Funcontacted.example.test&error_description=fixture-private-description"),
    ] {
        let response = app.clone().oneshot(get_request(&path)).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
        let response = body_json(response).await;
        assert_eq!(response["success"], false);
        assert_eq!(response["code"], "STATE_MISMATCH");
        assert_eq!(response["details"]["stage"], "callback");
        assert!(!response["details"]["trace_id"].as_str().unwrap().is_empty());
        let serialized = response.to_string();
        for private in ["unknown-fixture-state", "fixture-private-code", "fixture-forged-owner", "fixture-private-description", CANONICAL_MCP_SERVER_ID, "uncontacted.example.test"] {
            assert!(!serialized.contains(private), "the public callback must not reflect supplied query values");
        }
        assert!(response["details"].get("server_id").is_none());
        assert!(response["details"].get("owner_id").is_none());
    }
    let grants: i64 = nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM mcp_oauth_grants")
        .fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(grants, 0, "an unbound callback cannot create an authorization grant");
}

// ===========================================================================
// AU-1: Unauthenticated access to various MCP endpoints
// ===========================================================================

#[tokio::test]
async fn unauthenticated_get_servers_rejected() {
    let (app, _services) = build_app().await;

    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/api/mcp/servers")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    // CSRF middleware rejects before auth can run
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn unauthenticated_post_server_rejected() {
    let (app, _services) = build_app().await;

    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/api/mcp/servers")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            serde_json::to_vec(&json!({
                "name": "test",
                "transport": { "type": "stdio", "command": "npx" }
            }))
            .unwrap(),
        ))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// AU-3: Valid token accesses MCP routes successfully
#[tokio::test]
async fn authenticated_access_succeeds() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let resp = app
        .clone()
        .oneshot(get_with_token("/api/mcp/servers", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// AU-2: Invalid Bearer token is rejected by auth middleware (403 per API spec)
#[tokio::test]
async fn invalid_token_rejected() {
    let (app, _services) = build_app().await;

    // GET bypasses CSRF → auth middleware sees invalid Bearer → 403
    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/api/mcp/servers")
        .header("authorization", "Bearer invalid-jwt-token-abc123")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
