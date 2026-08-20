//! Provider CRUD and model fetch tests with auth.

mod common;

use axum::http::StatusCode;
use nomifun_common::ProviderId;
use serde_json::json;
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{body_json, build_app, delete_with_token, get_with_token, json_with_token, setup_and_login};

// ===========================================================================
// Provider CRUD
// ===========================================================================

#[tokio::test]
async fn provider_full_crud_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // 1. List — empty
    let resp = app
        .clone()
        .oneshot(get_with_token("/api/providers", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let providers = json["data"].as_array().unwrap();
    assert_eq!(providers.len(), 1);
    ProviderId::parse(providers[0]["provider_id"].as_str().unwrap()).unwrap();
    assert_eq!(providers[0]["platform"], "nomifun-free-model");

    // 2. Create
    let req = json_with_token(
        "POST",
        "/api/providers",
        json!({
            "platform": "anthropic",
            "name": "Anthropic",
            "base_url": "https://api.anthropic.com",
            "auth_scheme": "header_key:x-api-key",
            "credentials": {"api_keys": ["sk-ant-api03-test1234"]},
            "initial_model": {
                "model": "claude-test",
                "capabilities": [{
                    "task": "chat",
                    "protocol": "anthropic.messages",
                    "connection_role": "default",
                    "output_limit": 8192,
                    "provider_params": {}
                }]
            },
            "connections": []
        }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    let id = json["data"]["provider_id"].as_str().unwrap().to_string();
    assert_eq!(json["data"]["platform"], "anthropic");
    assert_eq!(json["data"]["name"], "Anthropic");
    assert_eq!(json["data"]["has_credentials"], true);
    assert!(json["data"].get("credentials").is_none());
    assert!(json["data"].get("api_key").is_none());

    // 3. List — should contain one
    let resp = app
        .clone()
        .oneshot(get_with_token("/api/providers", &token))
        .await
        .unwrap();
    let json = body_json(resp).await;
    let providers = json["data"].as_array().unwrap();
    assert_eq!(providers.len(), 2);
    assert!(
        providers
            .iter()
            .any(|provider| provider["platform"] == "nomifun-free-model")
    );
    assert!(
        providers
            .iter()
            .any(|provider| provider["provider_id"].as_str() == Some(id.as_str()))
    );

    // 4. Update
    let req = json_with_token(
        "PUT",
        &format!("/api/providers/{id}"),
        json!({"name": "Updated Name", "enabled": false}),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["data"]["name"], "Updated Name");
    assert!(!json["data"]["enabled"].as_bool().unwrap());

    // 5. Delete
    let resp = app
        .clone()
        .oneshot(delete_with_token(&format!("/api/providers/{id}"), &token, &csrf))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 6. Verify deleted
    let resp = app.oneshot(get_with_token("/api/providers", &token)).await.unwrap();
    let json = body_json(resp).await;
    let providers = json["data"].as_array().unwrap();
    assert_eq!(providers.len(), 1);
    ProviderId::parse(providers[0]["provider_id"].as_str().unwrap()).unwrap();
    assert_eq!(providers[0]["platform"], "nomifun-free-model");
}

#[tokio::test]
async fn provider_create_validation_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    // Missing platform
    let req = json_with_token(
        "POST",
        "/api/providers",
        json!({
            "name": "Test",
            "base_url": "https://api.example.com",
            "credentials": {"api_keys": ["sk-test"]}
        }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Invalid URL
    let req = json_with_token(
        "POST",
        "/api/providers",
        json!({
            "platform": "openai",
            "name": "Test",
            "base_url": "not-a-url",
            "auth_scheme": "bearer",
            "credentials": {"api_keys": ["sk-test"]},
            "initial_model": {
                "model": "gpt-test",
                "capabilities": [{
                    "task": "chat",
                    "protocol": "openai.chat_text",
                    "connection_role": "default",
                    "provider_params": {}
                }]
            },
            "connections": []
        }),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn provider_update_nonexistent_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let provider_id = ProviderId::new().into_string();
    let req = json_with_token(
        "PUT",
        &format!("/api/providers/{provider_id}"),
        json!({"name": "X"}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn provider_delete_nonexistent_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let provider_id = ProviderId::new().into_string();
    let resp = app
        .oneshot(delete_with_token(
            &format!("/api/providers/{provider_id}"),
            &token,
            &csrf,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ===========================================================================
// Model fetch
// ===========================================================================

#[tokio::test]
async fn model_fetch_openai_with_auth() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id": "gpt-4o"}, {"id": "gpt-4o-mini"}]
        })))
        .mount(&mock_server)
        .await;

    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = json_with_token(
        "POST",
        "/api/providers",
        json!({
            "platform": "openai",
            "name": "OpenAI Mock",
            "base_url": mock_server.uri(),
            "auth_scheme": "bearer",
            "credentials": {"api_keys": ["test-api-key"]},
            "initial_model": {
                "model": "gpt-test",
                "capabilities": [{
                    "task": "chat",
                    "protocol": "openai.chat_text",
                    "connection_role": "default",
                    "provider_params": {}
                }]
            },
            "connections": []
        }),
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json = body_json(resp).await;
    let id = json["data"]["provider_id"].as_str().unwrap().to_string();

    let req = json_with_token(
        "POST",
        &format!("/api/providers/{id}/models"),
        json!({"try_fix": false}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    let models = json["data"]["models"].as_array().unwrap();
    assert_eq!(models.len(), 2);
    // v3 shape: fetched models are objects carrying id (+ optional name).
    assert_eq!(models[0]["id"], "gpt-4o");
}

#[tokio::test]
async fn model_fetch_nonexistent_provider_with_auth() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let provider_id = ProviderId::new().into_string();
    let req = json_with_token(
        "POST",
        &format!("/api/providers/{provider_id}/models"),
        json!({}),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
