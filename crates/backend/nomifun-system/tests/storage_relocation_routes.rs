//! Online storage relocation reviews and schedules an offline move without
//! resetting the current dataset or changing any effective root.

mod common;

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use nomifun_common::{factory_reset, paths, storage_relocation};
use nomifun_db::{Database, ISettingsRepository, SqliteSettingsRepository, init_database};
use nomifun_system::{VersionCheckService, system_routes};

const TEST_KEY: [u8; 32] = [0x42; 32];
const GENERATION: &str = "0190f5fe-7c00-7a00-8000-0000000000f0";
const BASE: &str = "/api/system/storage-relocation";

struct Fixture {
    app: axum::Router,
    db: Database,
    data: PathBuf,
    work: PathBuf,
    target: PathBuf,
    control: PathBuf,
    root: tempfile::TempDir,
}

impl Fixture {
    async fn new(cli_override: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let work = root.path().join("work");
        let target = root.path().join("destination");
        std::fs::create_dir_all(work.join("conversations")).unwrap();
        let db = init_database(&data.join("nomifun-backend.db")).await.unwrap();
        let settings = SqliteSettingsRepository::new(db.pool().clone());
        settings.upsert_settings(Some("zh-CN"), Some(false), None).await.unwrap();
        std::fs::write(data.join("storage-generation"), GENERATION).unwrap();
        std::fs::write(work.join("conversations/history-attachment.txt"), b"existing attachment").unwrap();
        factory_reset::write_v3_dataset_receipt_for_work_dir(&data, &work, GENERATION).unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let app = system_routes(common::build_system_state(&db, TEST_KEY, client.clone(),
            VersionCheckService::new(client, "1.0.0".into()), data.clone(), work.clone(), cli_override));
        let control = storage_relocation::control_dir_for_root(&data).unwrap();
        Self { app, db, data, work, target, control, root }
    }

    async fn send(&self, method: &str, suffix: &str, payload: Value) -> (StatusCode, Value) {
        let response = self.app.clone().oneshot(request(method, &format!("{BASE}/{suffix}"), payload)).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    async fn prepare(&self) -> Value {
        let (status, body) = self.send("POST", "prepare", json!({ "target_dir": self.target })).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["data"].clone()
    }

    async fn assert_source_intact(&self) {
        assert_eq!(std::fs::read_to_string(self.data.join("storage-generation")).unwrap(), GENERATION);
        assert_eq!(std::fs::read(self.work.join("conversations/history-attachment.txt")).unwrap(), b"existing attachment");
        assert_eq!(factory_reset::inspect_v3_dataset_receipt(&self.data, &self.work).unwrap(), factory_reset::DatasetReceiptStatus::Current);
        assert!(factory_reset::requested_v3_reset_work_dir(&self.data).unwrap().is_none());
        let settings = SqliteSettingsRepository::new(self.db.pool().clone()).get_settings().await.unwrap().unwrap();
        assert_eq!(settings.language, "zh-CN");
        assert!(!settings.notification_enabled);
        assert!(!self.target.exists(), "online preparation/start/cancel must never copy the dataset");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Only remove the new hash scope this exact temporary source created.
        // Never recurse through the shared configuration parent or an alias.
        let expected_parent = dirs::config_dir().unwrap().join("NomiFun/storage");
        let source_is_temporary = self.data.starts_with(self.root.path());
        let isolated_scope = self.control.parent().is_some_and(|parent| paths::paths_equivalent(parent, &expected_parent))
            && self.control.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(source_is_temporary && isolated_scope, "test cleanup must stay within its own temporary dataset scope");
        if self.control.exists() { std::fs::remove_dir_all(&self.control).unwrap(); }
    }
}

fn request(method: &str, uri: &str, payload: Value) -> Request<Body> {
    Request::builder().method(method).uri(uri).header("content-type", "application/json")
        .body(if method == "GET" { Body::empty() } else { Body::from(serde_json::to_vec(&payload).unwrap()) }).unwrap()
}

#[tokio::test]
async fn prepare_start_status_and_cancel_preserve_live_data_and_generation() {
    let fixture = Fixture::new(false).await;
    let (status, idle) = fixture.send("GET", "status", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(idle["data"]["phase"], "idle");
    let prepared = fixture.prepare().await;
    assert_eq!(prepared["phase"], "prepared");
    assert!(!prepared["restart_required"].as_bool().unwrap());
    assert!(prepared["total_bytes"].as_u64().unwrap() > 0);
    assert!(prepared["file_count"].as_u64().unwrap() > 0);
    assert!(prepared["blockers"].as_array().unwrap().is_empty());
    assert!(paths::stored_path_matches(prepared["current_dir"].as_str().unwrap(), &fixture.data));
    fixture.assert_source_intact().await;

    let operation = prepared["operation_id"].as_str().unwrap();
    let (_, restored) = fixture.send("GET", "status", Value::Null).await;
    assert_eq!(restored["data"]["operation_id"], operation, "status must discover the first root's persisted plan");
    let (status, started) = fixture.send("POST", "start", json!({ "operation_id": operation })).await;
    assert_eq!(status, StatusCode::OK, "{started}");
    assert_eq!(started["data"]["phase"], "awaiting_restart");
    assert_eq!(started["data"]["restart_required"], true);
    assert!(paths::stored_path_matches(started["data"]["current_dir"].as_str().unwrap(), &fixture.data));
    let (_, restored) = fixture.send("GET", "status", Value::Null).await;
    assert_eq!(restored["data"]["phase"], "awaiting_restart");
    fixture.assert_source_intact().await;

    let (status, canceled) = fixture.send("POST", "cancel", json!({ "operation_id": operation })).await;
    assert_eq!(status, StatusCode::OK, "{canceled}");
    assert_eq!(canceled["data"]["phase"], "canceled");
    assert_eq!(canceled["data"]["restart_required"], false);
    let (_, restored) = fixture.send("GET", "status", Value::Null).await;
    assert_eq!(restored["data"]["phase"], "canceled");
    fixture.assert_source_intact().await;
    fixture.db.close().await;
}

#[tokio::test]
async fn startup_override_is_a_reviewable_blocker_without_a_persisted_plan() {
    let fixture = Fixture::new(true).await;
    let prepared = fixture.prepare().await;
    assert!(prepared["operation_id"].is_null());
    assert_eq!(prepared["blockers"][0]["code"], "STARTUP_OVERRIDE");
    assert!(!fixture.control.exists());
    fixture.assert_source_intact().await;
    fixture.db.close().await;
}

#[tokio::test]
async fn occupied_target_is_rejected_without_overwriting_foreign_files() {
    let fixture = Fixture::new(false).await;
    std::fs::create_dir(&fixture.target).unwrap();
    let foreign = fixture.target.join("important.txt");
    std::fs::write(&foreign, b"foreign data").unwrap();
    let (status, body) = fixture.send("POST", "prepare", json!({ "target_dir": fixture.target })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(std::fs::read(foreign).unwrap(), b"foreign data");
    assert!(storage_relocation::pending_storage_relocation(&fixture.control).unwrap().is_none());
    assert_eq!(std::fs::read_to_string(fixture.data.join("storage-generation")).unwrap(), GENERATION);
    fixture.db.close().await;
}

#[tokio::test]
async fn stale_operations_cannot_start_cancel_or_retarget_the_reviewed_plan() {
    let fixture = Fixture::new(false).await;
    let prepared = fixture.prepare().await;
    let operation = prepared["operation_id"].as_str().unwrap();
    for command in ["start", "cancel"] {
        let (status, body) = fixture.send("POST", command, json!({ "operation_id": "stale-operation" })).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
    }
    let different = fixture.root.path().join("other-target");
    let (status, _) = fixture.send("POST", "prepare", json!({ "target_dir": different })).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (_, restored) = fixture.send("GET", "status", Value::Null).await;
    assert_eq!(restored["data"]["operation_id"], operation);
    assert_eq!(restored["data"]["phase"], "prepared");
    fixture.assert_source_intact().await;
    fixture.db.close().await;
}

#[tokio::test]
async fn retired_work_dir_endpoint_does_not_arm_a_reset() {
    let fixture = Fixture::new(false).await;
    let response = fixture.app.clone().oneshot(request("POST", "/api/system/work-dir", json!({ "work_dir": fixture.target }))).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    fixture.assert_source_intact().await;
    fixture.db.close().await;
}

#[tokio::test]
async fn changed_live_work_root_cannot_arm_the_reviewed_plan() {
    let mut fixture = Fixture::new(false).await;
    let prepared = fixture.prepare().await;
    let operation = prepared["operation_id"].as_str().unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    fixture.app = system_routes(common::build_system_state(&fixture.db, TEST_KEY, client.clone(),
        VersionCheckService::new(client, "1.0.0".into()), fixture.data.clone(),
        fixture.root.path().join("different-live-work"), false));
    let (status, body) = fixture.send("POST", "start", json!({ "operation_id": operation })).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let pending = storage_relocation::pending_storage_relocation(&fixture.control).unwrap().unwrap();
    assert_eq!(pending.operation_id, operation);
    assert_eq!(pending.phase, storage_relocation::RelocationPhase::Prepared,
        "a rejected source binding must not arm or mutate the reviewed plan");
    fixture.assert_source_intact().await;
    fixture.db.close().await;
}

#[tokio::test]
async fn explicit_factory_reset_binds_live_work_root_without_parsing_damaged_receipt() {
    let fixture = Fixture::new(false).await;
    std::fs::write(fixture.data.join(factory_reset::V3_DATASET_RECEIPT_FILE), b"damaged receipt").unwrap();
    let response = fixture.app.clone().oneshot(request("POST", "/api/system/factory-reset", Value::Null)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let requested = factory_reset::requested_v3_reset_work_dir(&fixture.data).unwrap().unwrap();
    assert!(paths::paths_equivalent(&requested, &fixture.work));
    fixture.db.close().await;
}
