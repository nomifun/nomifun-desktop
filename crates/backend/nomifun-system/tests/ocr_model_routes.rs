use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use nomifun_db::{
    SqliteClientPreferenceRepository, SqliteModelProfileRepository, SqliteProviderRepository,
    SqliteSettingsRepository, init_database_memory,
};
use nomifun_system::{
    ClientPrefService, ModelFetchService, ModelProfileService, OcrModelService,
    PP_OCRV6_SMALL_MODEL_ID, ProtocolDetectionService, ProviderService, SettingsService,
    SystemRouterState, VersionCheckService, system_routes,
};
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt;

const TEST_KEY: [u8; 32] = [0x71; 32];

async fn setup() -> (axum::Router, TempDir) {
    let temp = TempDir::new().unwrap();
    let db = init_database_memory().await.unwrap();
    let provider_repo = Arc::new(SqliteProviderRepository::new(db.pool().clone()));
    let http = reqwest::Client::new();
    let ocr = OcrModelService::new(temp.path()).await.unwrap();
    let state = SystemRouterState {
        settings_service: SettingsService::new(Arc::new(SqliteSettingsRepository::new(
            db.pool().clone(),
        ))),
        client_pref_service: ClientPrefService::new(Arc::new(
            SqliteClientPreferenceRepository::new(db.pool().clone()),
        )),
        provider_service: ProviderService::new(provider_repo.clone(), TEST_KEY),
        model_fetch_service: ModelFetchService::new(provider_repo, TEST_KEY, http.clone()),
        model_profile_service: ModelProfileService::new(Arc::new(
            SqliteModelProfileRepository::new(db.pool().clone()),
        )),
        managed_model_service: None,
        local_model_service: None,
        ocr_model_service: Some(ocr),
        image_model_service: None,
        protocol_detection_service: ProtocolDetectionService::new(http.clone()),
        version_check_service: VersionCheckService::new(http, "0.1.0".into()),
        data_dir: temp.path().to_path_buf(),
    };
    (system_routes(state), temp)
}

fn request(method: &str, uri: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn catalog_and_status_are_safe_and_do_not_download_at_boot() {
    let (app, temp) = setup().await;
    let response = app
        .clone()
        .oneshot(request("GET", "/api/model-services/local/ocr/catalog"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["data"][0]["id"], PP_OCRV6_SMALL_MODEL_ID);
    assert_eq!(body["data"][0]["downloadSizeBytes"], 31_191_354);
    assert_eq!(body["data"][0]["license"], "Apache-2.0");
    assert_eq!(body["data"][0]["components"][0], "detector");
    assert!(body["data"][0].get("downloadUrl").is_none());
    assert!(body["data"][0].get("revision").is_none());
    assert!(body["data"][0].get("sha256").is_none());
    assert!(body["data"][0].get("localPath").is_none());

    let response = app
        .oneshot(request("GET", "/api/model-services/local/ocr/status"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["data"]["artifactsReady"], false);
    assert_eq!(body["data"]["inferenceReady"], false);
    assert_eq!(body["data"]["models"][0]["installPhase"], "not_installed");

    let model_dir = temp
        .path()
        .join("local-ai/ocr")
        .join(PP_OCRV6_SMALL_MODEL_ID);
    assert_eq!(std::fs::read_dir(model_dir).unwrap().count(), 0);
}

#[tokio::test]
async fn invalid_lifecycle_mutations_fail_without_network_work() {
    let (app, _temp) = setup().await;
    let unknown = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/model-services/local/ocr/models/not-in-catalog/install",
        ))
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);

    let pause = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/model-services/local/ocr/models/pp-ocrv6-small-onnx/pause",
        ))
        .await
        .unwrap();
    assert_eq!(pause.status(), StatusCode::CONFLICT);

    let resume = app
        .clone()
        .oneshot(request(
            "POST",
            "/api/model-services/local/ocr/models/pp-ocrv6-small-onnx/resume",
        ))
        .await
        .unwrap();
    assert_eq!(resume.status(), StatusCode::CONFLICT);

    let delete = app
        .oneshot(request(
            "DELETE",
            "/api/model-services/local/ocr/models/pp-ocrv6-small-onnx",
        ))
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::OK);
    let body = json_body(delete).await;
    assert_eq!(body["data"]["models"][0]["installPhase"], "not_installed");
}
