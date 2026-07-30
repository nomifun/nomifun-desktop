//! Black-box integration tests for the provider-connection routes.
//! Exercises GET/POST/DELETE on `/api/providers/{provider_id}/connections`
//! over HTTP via `oneshot`.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

use nomifun_db::{
    SqliteClientPreferenceRepository, SqliteProviderConnectionRepository,
    SqliteProviderModelRepository, SqliteProviderRepository, SqliteSettingsRepository,
    init_database_memory,
};
use nomifun_system::{
    ClientPrefService, ModelFetchService, ModelProfileService, ProtocolDetectionService,
    ProviderConnectionService, ProviderService, SettingsService, SystemRouterState,
    VersionCheckService, system_routes,
};

const TEST_KEY: [u8; 32] = [0x42; 32];

fn build_state(db: &nomifun_db::Database) -> SystemRouterState {
    let provider_repo = Arc::new(SqliteProviderRepository::new(db.pool().clone()));
    let http_client = reqwest::Client::new();
    SystemRouterState {
        settings_service: SettingsService::new(Arc::new(SqliteSettingsRepository::new(db.pool().clone()))),
        client_pref_service: ClientPrefService::new(Arc::new(SqliteClientPreferenceRepository::new(db.pool().clone()))),
        provider_service: ProviderService::new(
            provider_repo.clone(),
            Arc::new(SqliteProviderModelRepository::new(db.pool().clone())),
            TEST_KEY,
        ),
        provider_connection_service: ProviderConnectionService::new(
            Arc::new(SqliteProviderConnectionRepository::new(db.pool().clone())),
            provider_repo.clone(),
            TEST_KEY,
        ),
        model_fetch_service: ModelFetchService::new(provider_repo.clone(), TEST_KEY, http_client.clone()),
        model_profile_service: ModelProfileService::new(Arc::new(SqliteProviderModelRepository::new(db.pool().clone()))),
        provider_model_service: nomifun_system::ProviderModelService::new(
            Arc::new(SqliteProviderModelRepository::new(db.pool().clone())),
            provider_repo.clone(),
        ),
        managed_model_service: None,
        protocol_detection_service: ProtocolDetectionService::new(http_client.clone()),
        version_check_service: VersionCheckService::new(http_client, "0.1.0".to_owned()),
        data_dir: std::env::temp_dir(),
        work_dir: std::env::temp_dir(),
        work_dir_is_cli_override: false,
    }
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
                "api_key": "sk-primary",
                "models": []
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

    // POST creates the connection; response signals credentials without
    // echoing them.
    let resp = system_routes(build_state(&db))
        .oneshot(json_request(
            "POST",
            &format!("/api/providers/{provider_id}/connections"),
            json!({
                "role": "voice",
                "label": "Voice endpoint",
                "base_url": "https://voice.example.com/v1",
                "auth_scheme": "Bearer",
                "credentials": { "api_key": "sk-voice-secret" }
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

    // POST again without credentials keeps them and updates base_url,
    // preserving the stable connection_id.
    let resp = system_routes(build_state(&db))
        .oneshot(json_request(
            "POST",
            &format!("/api/providers/{provider_id}/connections"),
            json!({
                "role": "voice",
                "base_url": "https://voice2.example.com/v1"
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
async fn reserved_role_and_unknown_provider_status_codes() {
    let db = init_database_memory().await.unwrap();
    let provider_id = create_provider(&db).await;

    // role=default → 400 with the reserved-role message.
    let resp = system_routes(build_state(&db))
        .oneshot(json_request(
            "POST",
            &format!("/api/providers/{provider_id}/connections"),
            json!({
                "role": "default",
                "base_url": "https://voice.example.com/v1",
                "credentials": { "api_key": "sk" }
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

    // Unknown provider → 404 on list and upsert.
    let missing = "0190f5fe-7c00-7a00-8000-000000000099";
    let resp = system_routes(build_state(&db))
        .oneshot(bodyless_request("GET", &format!("/api/providers/{missing}/connections")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let resp = system_routes(build_state(&db))
        .oneshot(json_request(
            "POST",
            &format!("/api/providers/{missing}/connections"),
            json!({
                "role": "voice",
                "base_url": "https://voice.example.com/v1",
                "credentials": { "api_key": "sk" }
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
