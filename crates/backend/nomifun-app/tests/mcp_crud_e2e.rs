//! MCP server configuration CRUD E2E tests.
//!
//! Covers test-plan §1: create/read/update/delete/toggle/batch-import.

mod common;

use axum::http::StatusCode;
use serde_json::json;
use tower::ServiceExt;

use common::{body_json, build_app, delete_with_token, get_with_token, json_with_token, setup_and_login};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn stdio_server_json(name: &str) -> serde_json::Value {
    json!({
        "name": name,
        "description": "test stdio server",
        "transport": {
            "type": "stdio",
            "command": "npx",
            "args": ["-y", "@test/server"]
        }
    })
}

fn http_server_json(name: &str) -> serde_json::Value {
    json!({
        "name": name,
        "transport": {
            "type": "http",
            "url": "https://example.com/mcp"
        }
    })
}

fn sse_server_json(name: &str) -> serde_json::Value {
    json!({
        "name": name,
        "transport": {
            "type": "sse",
            "url": "https://example.com/sse",
            "headers": { "Authorization": "Bearer xxx" }
        }
    })
}

// ===========================================================================
// C-1..C-3: Create different transport types
// ===========================================================================

#[tokio::test]
async fn create_stdio_server() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token("POST", "/api/mcp/servers", stdio_server_json("test-mcp"), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let json = body_json(resp).await;
    assert!(json["success"].as_bool().unwrap());
    let data = &json["data"];
    assert!(data["id"].as_i64().unwrap() > 0);
    assert_eq!(data["name"], "test-mcp");
    assert_eq!(data["description"], "test stdio server");
    assert!(!data["enabled"].as_bool().unwrap());
    assert_eq!(data["transport"]["type"], "stdio");
    assert_eq!(data["transport"]["command"], "npx");
    assert_eq!(data["last_test_status"], "disconnected");
    assert!(!data["builtin"].as_bool().unwrap());
}

#[tokio::test]
async fn create_http_server() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token("POST", "/api/mcp/servers", http_server_json("http-mcp"), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let json = body_json(resp).await;
    assert_eq!(json["data"]["transport"]["type"], "http");
    assert_eq!(json["data"]["transport"]["url"], "https://example.com/mcp");
}

#[tokio::test]
async fn create_sse_server_with_headers() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token("POST", "/api/mcp/servers", sse_server_json("sse-mcp"), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let json = body_json(resp).await;
    assert_eq!(json["data"]["transport"]["type"], "sse");
    assert_eq!(json["data"]["transport"]["headers"]["Authorization"], "Bearer xxx");
}

// ===========================================================================
// C-4: Upsert by name
// ===========================================================================

#[tokio::test]
async fn create_same_name_upserts() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Create initial
    let req = json_with_token(
        "POST",
        "/api/mcp/servers",
        stdio_server_json("upsert-test"),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    let first = body_json(resp).await;
    let first_id = first["data"]["id"].as_i64().unwrap().to_string();

    // Create again with same name — should update, not duplicate
    let req = json_with_token(
        "POST",
        "/api/mcp/servers",
        http_server_json("upsert-test"),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let second = body_json(resp).await;
    assert_eq!(second["data"]["id"].as_i64().unwrap().to_string(), first_id);
    assert_eq!(second["data"]["transport"]["type"], "http");
}

// ===========================================================================
// C-5..C-9: Validation errors
// ===========================================================================

#[tokio::test]
async fn create_missing_name_returns_400() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/mcp/servers",
        json!({ "transport": { "type": "stdio", "command": "npx" } }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_missing_transport_returns_400() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token("POST", "/api/mcp/servers", json!({ "name": "test" }), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_invalid_transport_type_returns_400() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/mcp/servers",
        json!({ "name": "test", "transport": { "type": "invalid", "command": "x" } }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// C-8: stdio transport missing command field
#[tokio::test]
async fn create_stdio_missing_command_returns_400() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/mcp/servers",
        json!({ "name": "test", "transport": { "type": "stdio" } }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// C-9: http/sse transport missing url field
#[tokio::test]
async fn create_http_missing_url_returns_400() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/mcp/servers",
        json!({ "name": "test", "transport": { "type": "http" } }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_sse_missing_url_returns_400() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/mcp/servers",
        json!({ "name": "test", "transport": { "type": "sse" } }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ===========================================================================
// R-1..R-4: Read operations
// ===========================================================================

#[tokio::test]
async fn get_existing_server() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Create
    let req = json_with_token(
        "POST",
        "/api/mcp/servers",
        stdio_server_json("read-test"),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    let id = json["data"]["id"].as_i64().unwrap().to_string();

    // Get by ID
    let resp = app
        .clone()
        .oneshot(get_with_token(&format!("/api/mcp/servers/{id}"), &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["name"], "read-test");
}

#[tokio::test]
async fn get_nonexistent_server_returns_404() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let resp = app
        .clone()
        .oneshot(get_with_token("/api/mcp/servers/nonexistent", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_servers_returns_all() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Create two servers
    for name in ["list-a", "list-b"] {
        let req = json_with_token("POST", "/api/mcp/servers", stdio_server_json(name), &token, &csrf);
        app.clone().oneshot(req).await.unwrap();
    }

    let resp = app
        .clone()
        .oneshot(get_with_token("/api/mcp/servers", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["data"].as_array().unwrap().len() >= 2);
}

#[tokio::test]
async fn list_servers_empty() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let resp = app
        .clone()
        .oneshot(get_with_token("/api/mcp/servers", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"], json!([]));
}

// ===========================================================================
// U-1..U-5: Update operations
// ===========================================================================

#[tokio::test]
async fn update_server_name_is_rejected() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Create
    let req = json_with_token("POST", "/api/mcp/servers", stdio_server_json("old-name"), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    let id = json["data"]["id"].as_i64().unwrap().to_string();

    // Renaming an MCP is not allowed because historical conversations reference its name.
    let req = json_with_token(
        "PUT",
        &format!("/api/mcp/servers/{id}"),
        json!({ "name": "new-name" }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let json = body_json(resp).await;
    assert_eq!(json["success"], false);
}

#[tokio::test]
async fn update_server_transport() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Create as stdio
    let req = json_with_token(
        "POST",
        "/api/mcp/servers",
        stdio_server_json("transport-test"),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    let id = json["data"]["id"].as_i64().unwrap().to_string();

    // Update to http
    let req = json_with_token(
        "PUT",
        &format!("/api/mcp/servers/{id}"),
        json!({ "transport": { "type": "http", "url": "https://new.url" } }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["transport"]["type"], "http");
    assert_eq!(json["data"]["transport"]["url"], "https://new.url");
}

#[tokio::test]
async fn update_server_description() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/mcp/servers",
        stdio_server_json("desc-test"),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    let id = json["data"]["id"].as_i64().unwrap().to_string();

    let req = json_with_token(
        "PUT",
        &format!("/api/mcp/servers/{id}"),
        json!({ "description": "new description" }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["description"], "new description");
}

#[tokio::test]
async fn update_nonexistent_server_returns_404() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "PUT",
        "/api/mcp/servers/nonexistent",
        json!({ "name": "x" }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn update_name_to_existing_is_rejected() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Create A and B
    let req = json_with_token("POST", "/api/mcp/servers", stdio_server_json("server-a"), &token, &csrf);
    app.clone().oneshot(req).await.unwrap();

    let req = json_with_token("POST", "/api/mcp/servers", stdio_server_json("server-b"), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    let b_id = json["data"]["id"].as_i64().unwrap().to_string();

    // Renaming is rejected before name conflict handling.
    let req = json_with_token(
        "PUT",
        &format!("/api/mcp/servers/{b_id}"),
        json!({ "name": "server-a" }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ===========================================================================
// D-1..D-3: Delete operations
// ===========================================================================

#[tokio::test]
async fn delete_server() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Create
    let req = json_with_token(
        "POST",
        "/api/mcp/servers",
        stdio_server_json("delete-me"),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    let id = json["data"]["id"].as_i64().unwrap().to_string();

    // Delete
    let req = delete_with_token(&format!("/api/mcp/servers/{id}"), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify gone
    let resp = app
        .clone()
        .oneshot(get_with_token(&format!("/api/mcp/servers/{id}"), &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_nonexistent_server_returns_404() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = delete_with_token("/api/mcp/servers/nonexistent", &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ===========================================================================
// T-1..T-3: Toggle
// ===========================================================================

#[tokio::test]
async fn toggle_server_enables_then_disables() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Create (starts disabled)
    let req = json_with_token(
        "POST",
        "/api/mcp/servers",
        stdio_server_json("toggle-test"),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    let id = json["data"]["id"].as_i64().unwrap().to_string();
    assert!(!json["data"]["enabled"].as_bool().unwrap());

    // Toggle → enabled
    let req = json_with_token(
        "POST",
        &format!("/api/mcp/servers/{id}/toggle"),
        json!({}),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json["data"]["enabled"].as_bool().unwrap());

    // Toggle → disabled
    let req = json_with_token(
        "POST",
        &format!("/api/mcp/servers/{id}/toggle"),
        json!({}),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    assert!(!json["data"]["enabled"].as_bool().unwrap());
}

#[tokio::test]
async fn toggle_nonexistent_server_returns_404() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token("POST", "/api/mcp/servers/nonexistent/toggle", json!({}), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ===========================================================================
// B-1..B-3: Batch import
// ===========================================================================

#[tokio::test]
async fn batch_import_creates_multiple() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/mcp/servers/import",
        json!({
            "servers": [
                { "name": "import-a", "transport": { "type": "stdio", "command": "npx" } },
                { "name": "import-b", "transport": { "type": "http", "url": "https://example.com" } },
                { "name": "import-c", "transport": { "type": "sse", "url": "https://example.com/sse" } }
            ]
        }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn batch_import_upserts_existing() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Create one first
    let req = json_with_token("POST", "/api/mcp/servers", stdio_server_json("existing"), &token, &csrf);
    app.clone().oneshot(req).await.unwrap();

    // Batch import with one existing and one new
    let req = json_with_token(
        "POST",
        "/api/mcp/servers/import",
        json!({
            "servers": [
                { "name": "existing", "transport": { "type": "http", "url": "https://updated.com" } },
                { "name": "brand-new", "transport": { "type": "stdio", "command": "node" } }
            ]
        }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"].as_array().unwrap().len(), 2);

    // Verify total count is 2 (not 3)
    let resp = app
        .clone()
        .oneshot(get_with_token("/api/mcp/servers", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["data"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn batch_import_empty_list() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/mcp/servers/import",
        json!({ "servers": [] }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"], json!([]));
}

// B-4: Batch import with invalid config rejects the whole request
#[tokio::test]
async fn batch_import_with_invalid_config_returns_400() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/mcp/servers/import",
        json!({
            "servers": [
                { "name": "valid", "transport": { "type": "stdio", "command": "npx" } },
                { "name": "invalid", "transport": { "type": "unknown" } }
            ]
        }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ===========================================================================
// AU-1: Auth required (CSRF middleware returns 403 before auth checks)
// ===========================================================================

#[tokio::test]
async fn unauthenticated_access_is_rejected() {
    let (app, _services) = build_app().await;

    // GET without token — CSRF middleware rejects before auth can run
    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/api/mcp/servers")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
