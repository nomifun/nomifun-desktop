//! Integration tests for `POST /api/system/work-dir` — durably binding a
//! changed root to a one-shot fresh dataset before the next boot.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

use nomifun_db::{
    SqliteClientPreferenceRepository, SqliteProviderRepository, SqliteSettingsRepository, init_database_memory,
};
use nomifun_system::{
    ClientPrefService, ModelFetchService, ProtocolDetectionService, ProviderService, SettingsService,
    SystemRouterState, VersionCheckService, system_routes,
};

const TEST_KEY: [u8; 32] = [0x42; 32];

fn unique_data_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("nomifun-wdroute-{tag}-{}", nomifun_common::now_ms()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn setup(data_dir: std::path::PathBuf) -> axum::Router {
    seed_finalized_single_root_dataset(&data_dir);
    let work_dir = data_dir.clone();
    setup_with_work(data_dir, work_dir).await
}

fn seed_finalized_single_root_dataset(data_dir: &std::path::Path) {
    if data_dir
        .join(nomifun_common::factory_reset::V3_DATASET_RECEIPT_FILE)
        .exists()
    {
        return;
    }
    let generation = "0190f5fe-7c00-7a00-8000-0000000000f0";
    std::fs::write(
        data_dir.join("nomifun-backend.db"),
        b"receipt probe sentinel",
    )
    .unwrap();
    std::fs::write(
        data_dir.join("storage-generation"),
        generation,
    )
    .unwrap();
    nomifun_common::factory_reset::write_v3_dataset_receipt_for_work_dir(
        data_dir,
        data_dir,
        generation,
    )
    .unwrap();
}

async fn setup_with_work(
    data_dir: std::path::PathBuf,
    work_dir: std::path::PathBuf,
) -> axum::Router {
    setup_with_work_and_cli_override(data_dir, work_dir, false).await
}

async fn setup_with_work_and_cli_override(
    data_dir: std::path::PathBuf,
    work_dir: std::path::PathBuf,
    work_dir_is_cli_override: bool,
) -> axum::Router {
    let db = init_database_memory().await.unwrap();
    let provider_repo = Arc::new(SqliteProviderRepository::new(db.pool().clone()));
    let http_client = reqwest::Client::new();
    let state = SystemRouterState {
        settings_service: SettingsService::new(Arc::new(SqliteSettingsRepository::new(db.pool().clone()))),
        client_pref_service: ClientPrefService::new(Arc::new(SqliteClientPreferenceRepository::new(db.pool().clone()))),
        provider_service: ProviderService::new(provider_repo.clone(), TEST_KEY),
        model_fetch_service: ModelFetchService::new(provider_repo, TEST_KEY, http_client.clone()),
        model_profile_service: nomifun_system::ModelProfileService::new(std::sync::Arc::new(
            nomifun_db::SqliteModelProfileRepository::new(db.pool().clone()),
        )),
        managed_model_service: None,
        protocol_detection_service: ProtocolDetectionService::new(http_client.clone()),
        version_check_service: VersionCheckService::new(http_client, "1.0.0".to_owned()),
        work_dir,
        work_dir_is_cli_override,
        data_dir,
    };
    system_routes(state)
}

fn post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn explicit_factory_reset_binds_the_live_work_root_without_parsing_a_damaged_receipt() {
    let data_dir = unique_data_dir("factory-reset-damaged-receipt");
    let work_dir = unique_data_dir("factory-reset-live-work");
    std::fs::write(data_dir.join("dataset-v3.json"), b"damaged receipt")
        .unwrap();
    let app = setup_with_work(data_dir.clone(), work_dir.clone()).await;

    let resp = app
        .oneshot(post(
            "/api/system/factory-reset",
            serde_json::Value::Null,
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        nomifun_common::factory_reset::requested_v3_reset_work_dir(&data_dir)
            .unwrap(),
        Some(std::fs::canonicalize(&work_dir).unwrap())
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    let _ = std::fs::remove_dir_all(&work_dir);
}

#[tokio::test]
async fn valid_work_dir_change_creates_target_and_arms_one_shot_reset() {
    let data_dir = unique_data_dir("valid");
    let target_root = unique_data_dir("valid-target");
    let target = target_root.join("chosen-workspace"); // absolute, does not exist yet
    let app = setup(data_dir.clone()).await;

    let resp = app
        .oneshot(post("/api/system/work-dir", json!({ "work_dir": target.to_str().unwrap() })))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["success"], true);
    // The durable request, rather than dir-config, chooses the target on the
    // next boot. The config is committed only after the locked reset applies.
    assert!(target.is_dir(), "target work dir should have been created");
    let canonical_target = std::fs::canonicalize(&target).unwrap();
    assert_eq!(
        nomifun_common::factory_reset::requested_v3_reset_work_dir(&data_dir)
            .unwrap()
            .as_deref(),
        Some(canonical_target.as_path())
    );
    assert!(
        nomifun_common::dir_config::persisted_work_dir(&data_dir).is_none()
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    let _ = std::fs::remove_dir_all(&target_root);
}

#[tokio::test]
async fn finalized_dataset_change_arms_one_shot_reset_instead_of_rebinding_receipt() {
    let data_dir = unique_data_dir("finalized-change");
    let current = unique_data_dir("finalized-current");
    let target_root = unique_data_dir("finalized-target");
    let target = target_root.join("new-workspace");
    std::fs::create_dir_all(&current).unwrap();
    std::fs::write(data_dir.join("nomifun-backend.db"), b"receipt probe sentinel")
        .unwrap();
    let generation = "0190f5fe-7c00-7a00-8000-000000000001";
    std::fs::write(data_dir.join("storage-generation"), generation).unwrap();
    nomifun_common::factory_reset::write_v3_dataset_receipt_for_work_dir(
        &data_dir,
        &current,
        generation,
    )
    .unwrap();
    let app = setup_with_work(data_dir.clone(), current.clone()).await;

    let resp = app
        .oneshot(post(
            "/api/system/work-dir",
            json!({ "work_dir": target.to_str().unwrap() }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert!(target.is_dir());
    let canonical_target = std::fs::canonicalize(&target).unwrap();
    assert!(
        nomifun_common::dir_config::persisted_work_dir(&data_dir).is_none(),
        "target config is committed during locked pre-boot reset preparation"
    );
    assert_eq!(
        nomifun_common::factory_reset::requested_v3_reset_work_dir(&data_dir)
            .unwrap()
            .as_deref(),
        Some(canonical_target.as_path())
    );
    assert_eq!(
        nomifun_common::factory_reset::inspect_v3_dataset_receipt(
            &data_dir,
            &current,
        )
        .unwrap(),
        nomifun_common::factory_reset::DatasetReceiptStatus::Current,
        "the running generation must not be rebound in place"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    let _ = std::fs::remove_dir_all(&current);
    let _ = std::fs::remove_dir_all(&target_root);
}

#[tokio::test]
async fn missing_finalized_receipt_rejects_change_without_creating_target() {
    let data_dir = unique_data_dir("missing-receipt");
    let target_root = unique_data_dir("missing-receipt-target");
    let target = target_root.join("new-workspace");
    let app =
        setup_with_work(data_dir.clone(), data_dir.clone()).await;

    let resp = app
        .oneshot(post(
            "/api/system/work-dir",
            json!({ "work_dir": target.to_str().unwrap() }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CONFLICT);
    assert!(!target.exists());
    assert!(
        !data_dir
            .join(nomifun_common::factory_reset::V3_DATASET_RESET_REQUEST_FILE)
            .exists()
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    let _ = std::fs::remove_dir_all(&target_root);
}

#[tokio::test]
async fn pending_change_cannot_be_retargeted_before_restart() {
    let data_dir = unique_data_dir("pending-retarget");
    let first_root = unique_data_dir("pending-retarget-first");
    let second_root = unique_data_dir("pending-retarget-second");
    let first = first_root.join("workspace");
    let second = second_root.join("workspace");
    let app = setup(data_dir.clone()).await;

    let first_resp = app
        .clone()
        .oneshot(post(
            "/api/system/work-dir",
            json!({ "work_dir": first.to_str().unwrap() }),
        ))
        .await
        .unwrap();
    assert_eq!(first_resp.status(), StatusCode::OK);

    let second_resp = app
        .oneshot(post(
            "/api/system/work-dir",
            json!({ "work_dir": second.to_str().unwrap() }),
        ))
        .await
        .unwrap();
    assert_eq!(second_resp.status(), StatusCode::CONFLICT);
    assert_eq!(
        nomifun_common::factory_reset::requested_v3_reset_work_dir(&data_dir)
            .unwrap()
            .as_deref(),
        Some(std::fs::canonicalize(&first).unwrap().as_path())
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    let _ = std::fs::remove_dir_all(&first_root);
    let _ = std::fs::remove_dir_all(&second_root);
}

#[tokio::test]
async fn cli_work_dir_override_rejects_settings_change() {
    let data_dir = unique_data_dir("cli-override");
    let target_root = unique_data_dir("cli-override-target");
    let target = target_root.join("workspace");
    seed_finalized_single_root_dataset(&data_dir);
    let app = setup_with_work_and_cli_override(
        data_dir.clone(),
        data_dir.clone(),
        true,
    )
    .await;

    let resp = app
        .oneshot(post(
            "/api/system/work-dir",
            json!({ "work_dir": target.to_str().unwrap() }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CONFLICT);
    assert!(!target.exists());
    assert!(
        !data_dir
            .join(nomifun_common::factory_reset::V3_DATASET_RESET_REQUEST_FILE)
            .exists()
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    let _ = std::fs::remove_dir_all(&target_root);
}

#[tokio::test]
async fn rejects_work_dir_nested_inside_data_dir() {
    let data_dir = unique_data_dir("nested");
    let target = data_dir.join("nested-workspace");
    let app = setup(data_dir.clone()).await;

    let resp = app
        .oneshot(post(
            "/api/system/work-dir",
            json!({ "work_dir": target.to_str().unwrap() }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(nomifun_common::dir_config::persisted_work_dir(&data_dir).is_none());
    assert!(
        !data_dir
            .join(nomifun_common::factory_reset::V3_DATASET_RESET_REQUEST_FILE)
            .exists()
    );

    let _ = std::fs::remove_dir_all(&data_dir);
}

#[tokio::test]
async fn rejects_work_dir_that_contains_data_dir() {
    let root = unique_data_dir("ancestor-root");
    let data_dir = root.join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let app = setup(data_dir.clone()).await;

    let resp = app
        .oneshot(post(
            "/api/system/work-dir",
            json!({ "work_dir": root.to_str().unwrap() }),
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(nomifun_common::dir_config::persisted_work_dir(&data_dir).is_none());
    assert!(
        !data_dir
            .join(nomifun_common::factory_reset::V3_DATASET_RESET_REQUEST_FILE)
            .exists()
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn rejects_relative_work_dir() {
    let data_dir = unique_data_dir("relative");
    let app = setup(data_dir.clone()).await;

    let resp = app
        .oneshot(post("/api/system/work-dir", json!({ "work_dir": "relative/path" })))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(nomifun_common::dir_config::persisted_work_dir(&data_dir).is_none());

    let _ = std::fs::remove_dir_all(&data_dir);
}

#[tokio::test]
async fn rejects_blank_work_dir() {
    let data_dir = unique_data_dir("blank");
    let app = setup(data_dir.clone()).await;

    let resp = app
        .oneshot(post("/api/system/work-dir", json!({ "work_dir": "   " })))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(nomifun_common::dir_config::persisted_work_dir(&data_dir).is_none());

    let _ = std::fs::remove_dir_all(&data_dir);
}

#[tokio::test]
async fn rejects_work_dir_with_edge_whitespace_segment() {
    let data_dir = unique_data_dir("ws-space");
    // A path segment that begins/ends with whitespace ("bad ") — the repo refuses
    // these for conversation workspaces (workspace_path_has_edge_whitespace_segment),
    // so the work dir gatekeeper must reject it up front (deterministically, with
    // the dedicated error code) instead of relying on the OS to fail create_dir_all.
    let bad = data_dir.join("bad ").join("inner");
    let app = setup(data_dir.clone()).await;

    let resp = app
        .oneshot(post("/api/system/work-dir", json!({ "work_dir": bad.to_str().unwrap() })))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["code"], "WORKSPACE_PATH_EDGE_WHITESPACE_UNSUPPORTED");
    assert!(nomifun_common::dir_config::persisted_work_dir(&data_dir).is_none());

    let _ = std::fs::remove_dir_all(&data_dir);
}
