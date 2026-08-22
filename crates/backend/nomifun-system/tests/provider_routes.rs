//! Black-box tests for aggregate provider graph management.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use nomifun_common::ProviderId;
use nomifun_db::{
    IProviderModelCapabilityRepository, IProviderRepository,
    SqliteProviderModelCapabilityRepository, SqliteProviderRepository, UpdateProviderParams,
    init_database_memory,
};
use nomifun_system::{SystemRouterState, VersionCheckService, system_routes};

const TEST_KEY: [u8; 32] = [0x42; 32];

fn build_state(db: &nomifun_db::Database) -> SystemRouterState {
    let http = reqwest::Client::new();
    common::build_system_state(
        db,
        TEST_KEY,
        http.clone(),
        VersionCheckService::new(http, "0.1.0".into()),
        None,
        std::env::temp_dir(),
        std::env::temp_dir(),
        false,
    )
}

fn request(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    let builder = Request::builder().method(method).uri(uri);
    match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn create_body(name: &str) -> Value {
    json!({
        "platform": "openai",
        "name": name,
        "base_url": "https://api.openai.example/v1",
        "auth_scheme": "bearer",
        "credentials": {"api_keys":["sk-primary"]},
        "enabled": true,
        "initial_model": {
            "model": "gpt-test",
            "description": "initial chat model",
            "capabilities": [{
                "task": "chat",
                "traits": ["function_calling", "streaming"],
                "protocol": "openai.chat_text",
                "connection_role": "default",
                "provider_params": {}
            }]
        },
        "connections": []
    })
}

#[tokio::test]
async fn empty_list_and_aggregate_create_projection() {
    let db = init_database_memory().await.unwrap();
    let empty = system_routes(build_state(&db))
        .oneshot(request("GET", "/api/providers", None))
        .await
        .unwrap();
    assert_eq!(empty.status(), StatusCode::OK);
    assert!(body_json(empty).await["data"].as_array().unwrap().is_empty());

    let created = system_routes(build_state(&db))
        .oneshot(request("POST", "/api/providers", Some(create_body("Primary"))))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = body_json(created).await;
    let provider = &created["data"];
    ProviderId::parse(provider["provider_id"].as_str().unwrap()).unwrap();
    assert_eq!(provider["has_credentials"], true);
    assert!(provider.get("credentials").is_none());
    assert!(provider.get("api_key").is_none());
    assert_eq!(provider["auth_scheme"], "bearer");
    assert_eq!(provider["models"].as_array().unwrap().len(), 1);
    assert_eq!(provider["models"][0]["model"], "gpt-test");
    assert_eq!(provider["models"][0]["capabilities"][0]["task"], "chat");

    let listed = system_routes(build_state(&db))
        .oneshot(request("GET", "/api/providers", None))
        .await
        .unwrap();
    let listed = body_json(listed).await;
    assert_eq!(listed["data"].as_array().unwrap().len(), 1);
    assert_eq!(listed["data"][0]["models"][0]["model"], "gpt-test");
}

#[tokio::test]
async fn aggregate_create_commits_named_connection_and_capability_together() {
    let db = init_database_memory().await.unwrap();
    let body = json!({
        "platform": "openai",
        "name": "Voice provider",
        "base_url": "https://api.example/v1",
        "auth_scheme": "bearer",
        "credentials": {"api_keys":["sk-default"]},
        "initial_model": {
            "model": "voice/model",
            "capabilities": [{
                "task": "speech_synthesis",
                "protocol": "openai.audio_speech",
                "connection_role": "voice",
                "endpoint": "/audio/speech",
                "provider_params": {"voice": "alloy"}
            }]
        },
        "connections": [{
            "role": "voice",
            "label": "Voice endpoint",
            "base_url": "https://voice.example/v1",
            "auth_scheme": "header_key:x-api-key",
            "credentials": {"api_keys": ["voice-secret"]},
            "extra": {}
        }]
    });
    let created = system_routes(build_state(&db))
        .oneshot(request("POST", "/api/providers", Some(body)))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = body_json(created).await;
    let provider_id = created["data"]["provider_id"].as_str().unwrap();

    let connections = system_routes(build_state(&db))
        .oneshot(request(
            "GET",
            &format!("/api/providers/{provider_id}/connections"),
            None,
        ))
        .await
        .unwrap();
    let connections = body_json(connections).await;
    assert_eq!(connections["data"].as_array().unwrap().len(), 1);
    assert_eq!(connections["data"][0]["role"], "voice");
    assert_eq!(connections["data"][0]["has_credentials"], true);
}

#[tokio::test]
async fn aggregate_create_validation_is_atomic() {
    let db = init_database_memory().await.unwrap();
    let mut body = create_body("Broken");
    body["initial_model"]["capabilities"][0]["connection_role"] = json!("missing");
    let response = system_routes(build_state(&db))
        .oneshot(request("POST", "/api/providers", Some(body)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let listed = system_routes(build_state(&db))
        .oneshot(request("GET", "/api/providers", None))
        .await
        .unwrap();
    assert!(body_json(listed).await["data"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn aggregate_named_connection_requires_explicit_credentials() {
    let db = init_database_memory().await.unwrap();
    let mut body = create_body("Missing child credentials");
    body["connections"] = json!([{
        "role":"voice",
        "base_url":"https://voice.example/v1",
        "auth_scheme":"bearer"
    }]);
    let response = system_routes(build_state(&db))
        .oneshot(request("POST", "/api/providers", Some(body)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        SqliteProviderRepository::new(db.pool().clone())
            .list()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn cleared_migration_credentials_remain_listable_and_can_be_reentered() {
    let db = init_database_memory().await.unwrap();
    let created = system_routes(build_state(&db))
        .oneshot(request("POST", "/api/providers", Some(create_body("Migrated"))))
        .await
        .unwrap();
    let provider_id = body_json(created).await["data"]["provider_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let repo = SqliteProviderRepository::new(db.pool().clone());
    let revision = repo
        .find_by_id(&provider_id)
        .await
        .unwrap()
        .unwrap()
        .config_revision;
    repo.update(
        &provider_id,
        revision,
        UpdateProviderParams {
            credentials_encrypted: Some(""),
            ..Default::default()
        },
    )
        .await
        .unwrap();

    let listed = system_routes(build_state(&db))
        .oneshot(request("GET", "/api/providers", None))
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let listed = body_json(listed).await;
    assert_eq!(listed["data"][0]["has_credentials"], false);

    let reentered = system_routes(build_state(&db))
        .oneshot(request(
            "PUT",
            &format!("/api/providers/{provider_id}"),
            Some(json!({"credentials":{"api_keys":["replacement"]}})),
        ))
        .await
        .unwrap();
    assert_eq!(reentered.status(), StatusCode::OK);
    assert_eq!(body_json(reentered).await["data"]["has_credentials"], true);
}

#[tokio::test]
async fn aggregate_create_rejects_provider_params_the_protocol_cannot_encode() {
    let db = init_database_memory().await.unwrap();
    let mut body = create_body("Invalid multipart params");
    let capability = &mut body["initial_model"]["capabilities"][0];
    capability["task"] = json!("image_edit");
    capability["traits"] = json!([]);
    capability["protocol"] = json!("openai.images");
    capability["endpoint"] = json!("/images/edits");
    capability["provider_params"] = json!({"future":{"nested":true}});

    let response = system_routes(build_state(&db))
        .oneshot(request("POST", "/api/providers", Some(body)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error = body_json(response).await;
    assert!(
        error.to_string().contains("cannot losslessly encode"),
        "unexpected error: {error}"
    );

    let listed = system_routes(build_state(&db))
        .oneshot(request("GET", "/api/providers", None))
        .await
        .unwrap();
    assert!(body_json(listed).await["data"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn update_keeps_platform_identity_and_updates_connection_defaults() {
    let db = init_database_memory().await.unwrap();
    let created = system_routes(build_state(&db))
        .oneshot(request("POST", "/api/providers", Some(create_body("Before"))))
        .await
        .unwrap();
    let provider_id = body_json(created).await["data"]["provider_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let updated = system_routes(build_state(&db))
        .oneshot(request(
            "PUT",
            &format!("/api/providers/{provider_id}"),
            Some(json!({
                "name": "After",
                "base_url": "https://gateway.example/v1",
                "auth_scheme": "bearer",
                "credentials": {"api_keys":["new-secret"]},
                "sort_order": 7
            })),
        ))
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);
    let updated = body_json(updated).await;
    assert_eq!(updated["data"]["platform"], "openai");
    assert_eq!(updated["data"]["name"], "After");
    assert_eq!(updated["data"]["auth_scheme"], "bearer");
    assert_eq!(updated["data"]["has_credentials"], true);
    assert!(updated["data"].get("credentials").is_none());
    assert_eq!(updated["data"]["sort_order"], 7);

    let platform_change = system_routes(build_state(&db))
        .oneshot(request(
            "PUT",
            &format!("/api/providers/{provider_id}"),
            Some(json!({"platform": "anthropic"})),
        ))
        .await
        .unwrap();
    assert_eq!(platform_change.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn non_bearer_auth_survives_create_list_and_metadata_only_update() {
    let db = init_database_memory().await.unwrap();
    let mut body = create_body("Gemini auth persistence");
    body["platform"] = json!("gemini");
    body["base_url"] = json!("https://generativelanguage.googleapis.com");
    body["auth_scheme"] = json!("header_key:x-goog-api-key");
    body["initial_model"]["model"] = json!("gemini-contract");
    body["initial_model"]["capabilities"][0]["protocol"] = json!("gemini.generate_text");
    body["initial_model"]["capabilities"][0]["endpoint"] =
        json!("/v1beta/models/{model}:streamGenerateContent?alt=sse");

    let created = system_routes(build_state(&db))
        .oneshot(request("POST", "/api/providers", Some(body)))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = body_json(created).await;
    let provider_id = created["data"]["provider_id"].as_str().unwrap();
    assert_eq!(created["data"]["auth_scheme"], "header_key:x-goog-api-key");

    let updated = system_routes(build_state(&db))
        .oneshot(request(
            "PUT",
            &format!("/api/providers/{provider_id}"),
            Some(json!({"name": "Gemini renamed"})),
        ))
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);
    assert_eq!(
        body_json(updated).await["data"]["auth_scheme"],
        "header_key:x-goog-api-key"
    );

    let listed = system_routes(build_state(&db))
        .oneshot(request("GET", "/api/providers", None))
        .await
        .unwrap();
    assert_eq!(
        body_json(listed).await["data"][0]["auth_scheme"],
        "header_key:x-goog-api-key"
    );
}

#[tokio::test]
async fn invocation_changes_clear_default_capability_health_but_metadata_edits_do_not() {
    let db = init_database_memory().await.unwrap();
    let created = system_routes(build_state(&db))
        .oneshot(request("POST", "/api/providers", Some(create_body("Before"))))
        .await
        .unwrap();
    let provider_id = body_json(created).await["data"]["provider_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let capability_repo = SqliteProviderModelCapabilityRepository::new(db.pool().clone());
    let config_revision = SqliteProviderRepository::new(db.pool().clone())
        .find_by_id(&provider_id)
        .await
        .unwrap()
        .unwrap()
        .config_revision;
    capability_repo
        .set_health(
            &provider_id,
            config_revision,
            "gpt-test",
            "chat",
            Some(r#"{"status":"healthy","latency":7}"#),
        )
        .await
        .unwrap();

    // Display metadata and resubmitting the same plaintext secret are true
    // no-ops for invocation, so the observation remains valid.
    let metadata_only = system_routes(build_state(&db))
        .oneshot(request(
            "PUT",
            &format!("/api/providers/{provider_id}"),
            Some(json!({
                "name": "Renamed",
                "credentials": {"api_keys":["sk-primary"]}
            })),
        ))
        .await
        .unwrap();
    assert_eq!(metadata_only.status(), StatusCode::OK);
    assert!(
        capability_repo
            .get(&provider_id, "gpt-test", "chat")
            .await
            .unwrap()
            .unwrap()
            .health
            .is_some()
    );

    let transport_change = system_routes(build_state(&db))
        .oneshot(request(
            "PUT",
            &format!("/api/providers/{provider_id}"),
            Some(json!({"base_url": "https://gateway.example/v1"})),
        ))
        .await
        .unwrap();
    assert_eq!(transport_change.status(), StatusCode::OK);
    let capability = capability_repo
        .get(&provider_id, "gpt-test", "chat")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(capability.health, None);
    assert_eq!(capability.health_checked_at, None);
}

#[tokio::test]
async fn clone_copies_graph_then_delete_removes_source() {
    let db = init_database_memory().await.unwrap();
    let created = system_routes(build_state(&db))
        .oneshot(request("POST", "/api/providers", Some(create_body("Source"))))
        .await
        .unwrap();
    let source_id = body_json(created).await["data"]["provider_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let cloned = system_routes(build_state(&db))
        .oneshot(request(
            "POST",
            &format!("/api/providers/{source_id}/clone"),
            Some(json!({"name": "Copy"})),
        ))
        .await
        .unwrap();
    assert_eq!(cloned.status(), StatusCode::CREATED);
    let cloned = body_json(cloned).await;
    assert_ne!(cloned["data"]["provider_id"], source_id);
    assert_eq!(cloned["data"]["name"], "Copy");
    assert_eq!(cloned["data"]["models"][0]["model"], "gpt-test");
    assert_eq!(cloned["data"]["models"][0]["capabilities"][0]["task"], "chat");

    let deleted = system_routes(build_state(&db))
        .oneshot(request(
            "DELETE",
            &format!("/api/providers/{source_id}"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::OK);

    let listed = system_routes(build_state(&db))
        .oneshot(request("GET", "/api/providers", None))
        .await
        .unwrap();
    let listed = body_json(listed).await;
    assert_eq!(listed["data"].as_array().unwrap().len(), 1);
    assert_eq!(listed["data"][0]["name"], "Copy");
}

#[tokio::test]
async fn duplicate_id_is_rejected() {
    let db = init_database_memory().await.unwrap();
    let provider_id = ProviderId::new().into_string();
    let mut body = create_body("One");
    body["provider_id"] = json!(provider_id);
    let first = system_routes(build_state(&db))
        .oneshot(request("POST", "/api/providers", Some(body.clone())))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::CREATED);

    let duplicate = system_routes(build_state(&db))
        .oneshot(request("POST", "/api/providers", Some(body)))
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn bedrock_requires_explicit_sdk_auth_and_sdk_capability() {
    let db = init_database_memory().await.unwrap();
    let body = json!({
        "platform": "bedrock",
        "name": "Bedrock",
        "base_url": "",
        "auth_scheme": "bedrock",
        "credentials": {
            "access_key_id": "AKIA_ROUTE_TEST",
            "secret_access_key": "bedrock-route-secret",
            "session_token": "bedrock-route-session"
        },
        "bedrock_config": {
            "auth_method": "accessKey",
            "region": "us-east-1"
        },
        "initial_model": {
            "model": "anthropic.claude-test",
            "capabilities": [{
                "task": "chat",
                "protocol": "bedrock.anthropic_messages",
                "connection_role": "default",
                "output_limit": 8192,
                "provider_params": {}
            }]
        },
        "connections": []
    });
    let response = system_routes(build_state(&db))
        .oneshot(request("POST", "/api/providers", Some(body)))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let created = body_json(response).await;
    assert_eq!(created["data"]["has_credentials"], true);
    assert_eq!(created["data"]["bedrock_config"]["auth_method"], "accessKey");
    let serialized = created.to_string();
    for secret in [
        "AKIA_ROUTE_TEST",
        "bedrock-route-secret",
        "bedrock-route-session",
        "secret_access_key",
        "session_token",
    ] {
        assert!(
            !serialized.contains(secret),
            "Bedrock write-only credential leaked in response: {secret}"
        );
    }
}
