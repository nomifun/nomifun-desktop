//! Black-box integration tests for the provider-connection routes.
//! Exercises GET/PUT/DELETE on `/api/providers/{provider_id}/connections`
//! over HTTP via `oneshot`.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

use nomifun_db::{
    IProviderModelCapabilityRepository, IProviderModelRepository, IProviderRepository,
    NewProviderModel, NewProviderModelCapability, SqliteProviderModelCapabilityRepository,
    SqliteProviderModelRepository, SqliteProviderRepository, init_database_memory,
};
use nomifun_system::{
    SystemRouterState, VersionCheckService, system_routes,
};

const TEST_KEY: [u8; 32] = [0x42; 32];

fn build_state(db: &nomifun_db::Database) -> SystemRouterState {
    let http_client = reqwest::Client::new();
    common::build_system_state(
        db,
        TEST_KEY,
        http_client.clone(),
        VersionCheckService::new(http_client, "0.1.0".to_owned()),
        None,
        std::env::temp_dir(),
        std::env::temp_dir(),
        false,
    )
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn json_request(method: &str, uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn bodyless_request(method: &str, uri: &str) -> Request<Body> {
    Request::builder().method(method).uri(uri).body(Body::empty()).unwrap()
}

/// Create a provider through the normal route, return its id.
async fn create_provider(db: &nomifun_db::Database) -> String {
    let resp = system_routes(build_state(db))
        .oneshot(json_request(
            "POST",
            "/api/providers",
            json!({
                "platform": "openai",
                "name": "Primary",
                "base_url": "https://api.example.com/v1",
                "auth_scheme": "bearer",
                "credentials": {"api_keys":["sk-primary"]},
                "initial_model": {
                    "model": "seed-chat",
                    "capabilities": [{
                        "task": "chat",
                        "protocol": "openai.chat_text",
                        "connection_role": "default",
                        "provider_params": {}
                    }]
                },
                "connections": []
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let v = body_json(resp).await;
    v["data"]["provider_id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn upsert_list_delete_provider_connection_roundtrip() {
    let db = init_database_memory().await.unwrap();
    let provider_id = create_provider(&db).await;

    // PUT creates the connection; response signals credentials without
    // echoing them.
    let resp = system_routes(build_state(&db))
        .oneshot(json_request(
            "PUT",
            &format!("/api/providers/{provider_id}/connections"),
            json!({
                "role": "voice",
                "label": "Voice endpoint",
                "base_url": "https://voice.example.com/v1",
                "auth_scheme": "Bearer",
                "credentials": { "api_keys": ["sk-voice-secret"] }
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["data"]["role"], "voice");
    assert_eq!(v["data"]["auth_scheme"], "bearer");
    assert_eq!(v["data"]["has_credentials"], true);
    let wire = v.to_string();
    assert!(!wire.contains("sk-voice-secret"), "plaintext leaked over the wire: {wire}");
    assert!(v["data"].get("credentials").is_none());
    let connection_id = v["data"]["connection_id"].as_str().unwrap().to_string();

    // GET lists it.
    let resp = system_routes(build_state(&db))
        .oneshot(bodyless_request(
            "GET",
            &format!("/api/providers/{provider_id}/connections"),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    let listed = v["data"].as_array().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["connection_id"], connection_id.as_str());
    assert_eq!(listed[0]["has_credentials"], true);

    // PUT again without credentials keeps them and updates base_url,
    // preserving the stable connection_id.
    let resp = system_routes(build_state(&db))
        .oneshot(json_request(
            "PUT",
            &format!("/api/providers/{provider_id}/connections"),
            json!({
                "role": "voice",
                "base_url": "https://voice2.example.com/v1",
                "auth_scheme": "bearer"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["data"]["connection_id"], connection_id.as_str());
    assert_eq!(v["data"]["base_url"], "https://voice2.example.com/v1");
    assert_eq!(v["data"]["has_credentials"], true);

    // DELETE removes it; a second DELETE is a no-op success.
    for _ in 0..2 {
        let resp = system_routes(build_state(&db))
            .oneshot(bodyless_request(
                "DELETE",
                &format!("/api/providers/{provider_id}/connections/voice"),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    let resp = system_routes(build_state(&db))
        .oneshot(bodyless_request(
            "GET",
            &format!("/api/providers/{provider_id}/connections"),
        ))
        .await
        .unwrap();
    let v = body_json(resp).await;
    assert!(v["data"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn creating_named_connection_requires_explicit_credentials() {
    let db = init_database_memory().await.unwrap();
    let provider_id = create_provider(&db).await;
    let response = system_routes(build_state(&db))
        .oneshot(json_request(
            "PUT",
            &format!("/api/providers/{provider_id}/connections"),
            json!({
                "role": "voice",
                "base_url": "https://voice.example.com/v1",
                "auth_scheme": "bearer"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let listed = system_routes(build_state(&db))
        .oneshot(bodyless_request(
            "GET",
            &format!("/api/providers/{provider_id}/connections"),
        ))
        .await
        .unwrap();
    assert!(body_json(listed).await["data"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn reserved_role_and_unknown_provider_status_codes() {
    let db = init_database_memory().await.unwrap();
    let provider_id = create_provider(&db).await;

    // role=default -> 400 with the reserved-role message.
    let resp = system_routes(build_state(&db))
        .oneshot(json_request(
            "PUT",
            &format!("/api/providers/{provider_id}/connections"),
            json!({
                "role": "default",
                "base_url": "https://voice.example.com/v1",
                "auth_scheme": "bearer",
                "credentials": { "api_keys": ["sk"] }
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v = body_json(resp).await;
    assert!(
        v["error"]
            .as_str()
            .unwrap()
            .contains("role 'default' is reserved"),
        "unexpected error body: {v}"
    );

    // Unknown provider -> 404 on list and upsert.
    let missing = "0190f5fe-7c00-7a00-8000-000000000099";
    let resp = system_routes(build_state(&db))
        .oneshot(bodyless_request("GET", &format!("/api/providers/{missing}/connections")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let resp = system_routes(build_state(&db))
        .oneshot(json_request(
            "PUT",
            &format!("/api/providers/{missing}/connections"),
            json!({
                "role": "voice",
                "base_url": "https://voice.example.com/v1",
                "auth_scheme": "bearer",
                "credentials": { "api_keys": ["sk"] }
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn upsert_rejects_non_executable_auth_scheme_and_wrong_credential_shape() {
    let db = init_database_memory().await.unwrap();
    let provider_id = create_provider(&db).await;

    for body in [
        json!({
            "role": "voice",
            "base_url": "https://voice.example.com/v1",
            "auth_scheme": "api_key",
            "credentials": {"api_keys": ["sk"]}
        }),
        // The retired singular credential shape must fail closed at save
        // time, before a first invocation can expose a configuration error.
        json!({
            "role": "voice",
            "base_url": "https://voice.example.com/v1",
            "auth_scheme": "bearer",
            "credentials": {"api_key": "sk"}
        }),
        json!({
            "role": "voice",
            "base_url": "https://voice.example.com/v1",
            "auth_scheme": "volc_voice",
            "credentials": {"app_key": "app", "access_key": "access"}
        }),
    ] {
        let resp = system_routes(build_state(&db))
            .oneshot(json_request(
                "PUT",
                &format!("/api/providers/{provider_id}/connections"),
                body,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let value = body_json(resp).await;
        assert_eq!(value["code"], "BAD_REQUEST");
    }
}

#[tokio::test]
async fn delete_referenced_connection_returns_conflict_and_keeps_row() {
    let db = init_database_memory().await.unwrap();
    let provider_id = create_provider(&db).await;

    let resp = system_routes(build_state(&db))
        .oneshot(json_request(
            "PUT",
            &format!("/api/providers/{provider_id}/connections"),
            json!({
                "role": "voice",
                "base_url": "https://voice.example.com/v1",
                "auth_scheme": "bearer",
                "credentials": {"api_keys": ["sk-voice"]}
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let capabilities = [NewProviderModelCapability {
        task: "speech_synthesis",
        traits: "[]",
        protocol: "openai.audio_speech",
        connection_role: "voice",
        provider_params: "{}",
        ..Default::default()
    }];
    let config_revision = SqliteProviderRepository::new(db.pool().clone())
        .find_by_id(&provider_id)
        .await
        .unwrap()
        .unwrap()
        .config_revision;
    SqliteProviderModelRepository::new(db.pool().clone())
        .save(
            &provider_id,
            config_revision,
            &NewProviderModel {
                model: "voice-model",
                enabled: true,
                sort_order: 0,
                description: None,
                capabilities: &capabilities,
            },
        )
        .await
        .unwrap();

    let resp = system_routes(build_state(&db))
        .oneshot(bodyless_request(
            "DELETE",
            &format!("/api/providers/{provider_id}/connections/voice"),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let value = body_json(resp).await;
    assert_eq!(value["code"], "CONFLICT");
    assert!(value["error"].as_str().unwrap().contains("voice-model"));

    let resp = system_routes(build_state(&db))
        .oneshot(bodyless_request(
            "GET",
            &format!("/api/providers/{provider_id}/connections"),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let value = body_json(resp).await;
    assert_eq!(value["data"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn connection_invocation_changes_clear_only_the_referenced_role_health() {
    let db = init_database_memory().await.unwrap();
    let provider_id = create_provider(&db).await;
    let connection_body = json!({
        "role": "voice",
        "label": "Voice endpoint",
        "base_url": "https://voice.example.com/v1",
        "auth_scheme": "bearer",
        "credentials": {"api_keys": ["sk-voice"]},
        "extra": {"region": "one"}
    });
    let created = system_routes(build_state(&db))
        .oneshot(json_request(
            "PUT",
            &format!("/api/providers/{provider_id}/connections"),
            connection_body.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);

    let capabilities = [NewProviderModelCapability {
        task: "speech_synthesis",
        traits: "[]",
        protocol: "openai.audio_speech",
        connection_role: "voice",
        provider_params: "{}",
        ..Default::default()
    }];
    let config_revision = SqliteProviderRepository::new(db.pool().clone())
        .find_by_id(&provider_id)
        .await
        .unwrap()
        .unwrap()
        .config_revision;
    SqliteProviderModelRepository::new(db.pool().clone())
        .save(
            &provider_id,
            config_revision,
            &NewProviderModel {
                model: "voice-health",
                enabled: true,
                sort_order: 0,
                description: None,
                capabilities: &capabilities,
            },
        )
        .await
        .unwrap();
    let capability_repo = SqliteProviderModelCapabilityRepository::new(db.pool().clone());
    let health_revision = SqliteProviderRepository::new(db.pool().clone())
        .find_by_id(&provider_id)
        .await
        .unwrap()
        .unwrap()
        .config_revision;
    capability_repo
        .set_health(
            &provider_id,
            health_revision,
            "voice-health",
            "speech_synthesis",
            Some(r#"{"status":"healthy","latency":5}"#),
        )
        .await
        .unwrap();

    let mut label_only = connection_body.clone();
    label_only["label"] = json!("Renamed voice endpoint");
    let unchanged_transport = system_routes(build_state(&db))
        .oneshot(json_request(
            "PUT",
            &format!("/api/providers/{provider_id}/connections"),
            label_only,
        ))
        .await
        .unwrap();
    assert_eq!(unchanged_transport.status(), StatusCode::OK);
    assert!(
        capability_repo
            .get(&provider_id, "voice-health", "speech_synthesis")
            .await
            .unwrap()
            .unwrap()
            .health
            .is_some()
    );

    let mut changed_extra = connection_body;
    changed_extra["extra"] = json!({"region": "two"});
    let changed_transport = system_routes(build_state(&db))
        .oneshot(json_request(
            "PUT",
            &format!("/api/providers/{provider_id}/connections"),
            changed_extra,
        ))
        .await
        .unwrap();
    assert_eq!(changed_transport.status(), StatusCode::OK);
    let capability = capability_repo
        .get(&provider_id, "voice-health", "speech_synthesis")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(capability.health, None);
    assert_eq!(capability.health_checked_at, None);
}
