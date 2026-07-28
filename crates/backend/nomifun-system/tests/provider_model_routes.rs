//! Black-box integration tests for the row-level model catalog routes
//! (`/api/provider-models*`). Exercises create -> list/filter -> update ->
//! delete over HTTP via `oneshot`, including projection consistency with
//! `GET /api/providers`.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

use nomifun_db::{
    SqliteClientPreferenceRepository, SqliteProviderModelRepository, SqliteProviderRepository,
    SqliteSettingsRepository, init_database_memory,
};
use nomifun_system::{
    ClientPrefService, ModelFetchService, ModelProfileService, ProtocolDetectionService,
    ProviderService, SettingsService, SystemRouterState, VersionCheckService, system_routes,
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
        provider_connection_service: nomifun_system::ProviderConnectionService::new(
            Arc::new(nomifun_db::SqliteProviderConnectionRepository::new(db.pool().clone())),
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

fn get_request(uri: &str) -> Request<Body> {
    Request::builder().method("GET").uri(uri).body(Body::empty()).unwrap()
}

/// Create a provider on the given platform with no models; return its id.
async fn create_provider(db: &nomifun_db::Database, platform: &str, name: &str) -> String {
    let resp = system_routes(build_state(db))
        .oneshot(json_request(
            "POST",
            "/api/providers",
            json!({
                "platform": platform,
                "name": name,
                "base_url": "https://api.example.test/v1",
                "api_key": "sk-test",
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
async fn create_list_update_delete_provider_model() {
    let db = init_database_memory().await.unwrap();
    let provider_id = create_provider(&db, "stepfun", "StepFun").await;

    // Create with no explicit tasks: heuristic seeds speech_recognition,
    // source=inferred, 201 Created (mirrors POST /api/providers).
    let resp = system_routes(build_state(&db))
        .oneshot(json_request(
            "POST",
            "/api/provider-models",
            json!({ "provider_id": provider_id, "model": "step-asr" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let v = body_json(resp).await;
    assert_eq!(v["data"]["model"], "step-asr");
    assert_eq!(v["data"]["tasks"], json!(["speech_recognition"]));
    assert_eq!(v["data"]["source"], "inferred");
    assert_eq!(v["data"]["enabled"], true);
    assert_eq!(v["data"]["sort_order"], 0);

    // The row is immediately part of the provider projection.
    let resp = system_routes(build_state(&db)).oneshot(get_request("/api/providers")).await.unwrap();
    let v = body_json(resp).await;
    let provider = &v["data"].as_array().unwrap()[0];
    assert_eq!(provider["models"], json!(["step-asr"]));

    // Partial update: description only; tasks/source untouched. Explicit null
    // for context_limit is a no-op clear on an already-NULL column.
    let resp = system_routes(build_state(&db))
        .oneshot(json_request(
            "POST",
            "/api/provider-models/update",
            json!({
                "provider_id": provider_id,
                "model": "step-asr",
                "description": "speech to text",
                "context_limit": null
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["data"]["description"], "speech to text");
    assert_eq!(v["data"]["tasks"], json!(["speech_recognition"]));
    assert_eq!(v["data"]["source"], "inferred");
    assert!(v["data"].get("context_limit").is_none());

    // Tasks update flips source to user.
    let resp = system_routes(build_state(&db))
        .oneshot(json_request(
            "POST",
            "/api/provider-models/update",
            json!({ "provider_id": provider_id, "model": "step-asr", "tasks": ["chat"] }),
        ))
        .await
        .unwrap();
    let v = body_json(resp).await;
    assert_eq!(v["data"]["tasks"], json!(["chat"]));
    assert_eq!(v["data"]["source"], "user");

    // Delete; the provider projection loses the model.
    let resp = system_routes(build_state(&db))
        .oneshot(json_request(
            "POST",
            "/api/provider-models/delete",
            json!({ "provider_id": provider_id, "model": "step-asr" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = system_routes(build_state(&db)).oneshot(get_request("/api/providers")).await.unwrap();
    let v = body_json(resp).await;
    let provider = &v["data"].as_array().unwrap()[0];
    assert_eq!(provider["models"], json!([]));

    let resp = system_routes(build_state(&db)).oneshot(get_request("/api/provider-models")).await.unwrap();
    let v = body_json(resp).await;
    assert!(v["data"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn list_filters_by_provider_id_query() {
    let db = init_database_memory().await.unwrap();
    let openai_id = create_provider(&db, "openai", "OpenAI").await;
    let deepseek_id = create_provider(&db, "deepseek", "DeepSeek").await;

    for (provider_id, model) in [(&openai_id, "gpt-4o"), (&deepseek_id, "deepseek-chat")] {
        let resp = system_routes(build_state(&db))
            .oneshot(json_request(
                "POST",
                "/api/provider-models",
                json!({ "provider_id": provider_id, "model": model }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    // Unfiltered: both rows.
    let resp = system_routes(build_state(&db)).oneshot(get_request("/api/provider-models")).await.unwrap();
    let v = body_json(resp).await;
    assert_eq!(v["data"].as_array().unwrap().len(), 2);

    // Filtered: only the requested provider's row.
    let resp = system_routes(build_state(&db))
        .oneshot(get_request(&format!("/api/provider-models?provider_id={openai_id}")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    let rows = v["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["model"], "gpt-4o");
    assert_eq!(rows[0]["provider_id"], openai_id.as_str());
}

#[tokio::test]
async fn error_paths_conflict_and_not_found() {
    let db = init_database_memory().await.unwrap();
    let provider_id = create_provider(&db, "openai", "OpenAI").await;

    // Explicit-tasks create → source=user.
    let resp = system_routes(build_state(&db))
        .oneshot(json_request(
            "POST",
            "/api/provider-models",
            json!({ "provider_id": provider_id, "model": "gpt-4o", "tasks": ["chat"] }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let v = body_json(resp).await;
    assert_eq!(v["data"]["source"], "user");

    // Duplicate create → 409.
    let resp = system_routes(build_state(&db))
        .oneshot(json_request(
            "POST",
            "/api/provider-models",
            json!({ "provider_id": provider_id, "model": "gpt-4o" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // Create under a missing provider → 404.
    let ghost = nomifun_common::ProviderId::new().into_string();
    let resp = system_routes(build_state(&db))
        .oneshot(json_request(
            "POST",
            "/api/provider-models",
            json!({ "provider_id": ghost, "model": "gpt-4o" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Update of a missing row → 404.
    let resp = system_routes(build_state(&db))
        .oneshot(json_request(
            "POST",
            "/api/provider-models/update",
            json!({ "provider_id": provider_id, "model": "ghost", "enabled": false }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Delete of a missing row → 404.
    let resp = system_routes(build_state(&db))
        .oneshot(json_request(
            "POST",
            "/api/provider-models/delete",
            json!({ "provider_id": provider_id, "model": "ghost" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Unknown body field → 400 (deny_unknown_fields).
    let resp = system_routes(build_state(&db))
        .oneshot(json_request(
            "POST",
            "/api/provider-models",
            json!({ "provider_id": provider_id, "model": "gpt-4o-mini", "bogus": 1 }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
