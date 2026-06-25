//! Integration tests for POST /api/terminals/register-knowledge.

mod common;

use axum::http::StatusCode;
use serde_json::json;
use tower::ServiceExt;

use common::{body_json, build_app, json_with_token, setup_and_login};

#[tokio::test]
async fn register_knowledge_claude_writes_mcp_json() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().to_str().unwrap().to_owned();

    let req = json_with_token(
        "POST",
        "/api/terminals/register-knowledge",
        json!({ "cwd": cwd, "family": "claude" }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["scope"], "project");
    assert!(json["data"]["written_path"].as_str().unwrap().ends_with(".mcp.json"));

    // Verify file was actually written
    let content = std::fs::read_to_string(tmp.path().join(".mcp.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert!(parsed["mcpServers"]["nomifun-knowledge"]["command"].is_string());
    assert_eq!(
        parsed["mcpServers"]["nomifun-knowledge"]["args"],
        json!(["mcp-knowledge-stdio"])
    );
    // Must NOT contain token or port
    let lower = content.to_lowercase();
    assert!(!lower.contains("token"));
    assert!(!lower.contains("\"port\""));
}

#[tokio::test]
async fn register_knowledge_invalid_family_returns_400() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/terminals/register-knowledge",
        json!({ "cwd": "/tmp", "family": "invalid-cli" }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let json = body_json(resp).await;
    assert_eq!(json["success"], false);
    assert!(json["error"].as_str().unwrap().contains("invalid family"));
}

#[tokio::test]
async fn register_knowledge_gemini_creates_dir_and_file() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let tmp = tempfile::tempdir().unwrap();
    let cwd = tmp.path().to_str().unwrap().to_owned();

    let req = json_with_token(
        "POST",
        "/api/terminals/register-knowledge",
        json!({ "cwd": cwd, "family": "gemini" }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["data"]["scope"], "project");

    // Verify .gemini/settings.json was written
    let path = tmp.path().join(".gemini/settings.json");
    assert!(path.exists());
    let content = std::fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert!(parsed["mcpServers"]["nomifun-knowledge"]["command"].is_string());
}
