//! Office artifact authorization, lifecycle and snapshot HTTP contracts.

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::json;
use tower::ServiceExt;

use common::{body_json, get_request, json_with_token, setup_and_login};

use nomifun_app::AppConfig;
use nomifun_app::compatibility::{AppServices, build_module_states, create_router_with_states};
use nomifun_office::{OfficePreviewService, OfficeRouterState, SnapshotService};

// ── Helpers ──────────────────────────────────────────────────────────

async fn build_office_app() -> (axum::Router, AppServices, tempfile::TempDir) {
    let default_roots = vec![
        std::env::temp_dir(),
        dirs::home_dir().unwrap_or_else(std::env::temp_dir),
    ];
    build_office_app_with_roots(default_roots).await
}

async fn build_office_app_with_roots(
    allowed_roots: Vec<std::path::PathBuf>,
) -> (axum::Router, AppServices, tempfile::TempDir) {
    let tmp = tempfile::TempDir::new().unwrap();
    let data_dir = tmp.path().to_path_buf();

    let db = nomifun_db::init_database_memory().await.unwrap();
    let config = AppConfig {
        data_dir: data_dir.clone(),
        work_dir: data_dir,
        ..Default::default()
    };
    let services = AppServices::from_config(db, &config).await.unwrap();
    let (mut states, _) = build_module_states(&services).await;

    states.office = build_test_office_state(tmp.path(), allowed_roots);

    let router = create_router_with_states(&services, states);
    (router, services, tmp)
}

fn build_test_office_state(
    data_dir: &std::path::Path,
    allowed_roots: Vec<std::path::PathBuf>,
) -> OfficeRouterState {
    use nomifun_office::{OfficeEngine, OfficeEngineSupplier, PreviewLimits};
    struct UnavailableEngine;
    #[async_trait::async_trait]
    impl OfficeEngineSupplier for UnavailableEngine {
        async fn resolve(&self) -> Result<OfficeEngine, nomifun_api_types::OfficePreviewError> {
            Err(nomifun_api_types::OfficePreviewError {
                code: "ENGINE_UNAVAILABLE".into(),
                message: "Preview component unavailable".into(),
                retryable: false,
            })
        }
    }
    OfficeRouterState {
        preview_service: Arc::new(OfficePreviewService::with_supplier(
            data_dir,
            Arc::new(UnavailableEngine),
            PreviewLimits::default(),
        )),
        snapshot_service: Arc::new(SnapshotService::new(data_dir)),
        allowed_roots,
    }
}

fn write_office_fixture(path: &std::path::Path, part: &str) {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for name in ["[Content_Types].xml", part] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"<root/>").unwrap();
    }
    zip.finish().unwrap();
}

fn snapshot_target() -> serde_json::Value {
    json!({"content_type": "markdown", "file_path": "/a.md"})
}

// ── AU-1/AU-2: Unauthenticated requests ─────────────────────────────

#[tokio::test]
async fn au1_unauthenticated_preview_start_returns_403() {
    let (app, _services, _tmp) = build_office_app().await;
    let req = common::get_request("/api/office-preview/start");
    let resp = app.oneshot(req).await.unwrap();
    assert!(
        resp.status() == StatusCode::UNAUTHORIZED || resp.status() == StatusCode::FORBIDDEN,
        "expected 401 or 403, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn au2_unauthenticated_all_office_endpoints() {
    let endpoints = [
        "/api/office-preview/start",
        "/api/office-preview/start",
        "/api/office-preview/start",
        "/api/preview-history/list",
        "/api/preview-history/save",
    ];

    for endpoint in endpoints {
        let (app, _services, _tmp) = build_office_app().await;
        let body = json!({});
        let req = axum::http::Request::builder()
            .method("POST")
            .uri(endpoint)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert!(
            resp.status() == StatusCode::UNAUTHORIZED || resp.status() == StatusCode::FORBIDDEN,
            "endpoint {endpoint}: expected 401 or 403, got {}",
            resp.status()
        );
    }
}

// ── WP-4: Word preview start (officecli not available) ───────────────

#[tokio::test]
async fn wp4_word_preview_officecli_not_available() {
    let (mut app, services, tmp) = build_office_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pass123").await;

    let file_path = tmp.path().join("test.docx");
    write_office_fixture(&file_path, "word/document.xml");

    let body = json!({"request_id":nomifun_common::generate_id(), "doc_type":"word", "file_path": file_path.to_str().unwrap()});
    let req = json_with_token("POST", "/api/office-preview/start", body, &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["state"], "queued");
    let request_id = json["data"]["request_id"].as_str().unwrap();
    let terminal = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let request = json_with_token(
                "GET",
                &format!("/api/office-preview/status/{request_id}"),
                json!({}),
                &token,
                &csrf,
            );
            let result = body_json(app.clone().oneshot(request).await.unwrap()).await;
            if result["data"]["state"] == "error" {
                break result;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(terminal["data"]["error"]["code"], "ENGINE_UNAVAILABLE");
    assert_eq!(terminal["data"]["error"]["retryable"], false);
}

#[tokio::test]
async fn wp5_word_preview_with_workspace_accepts_non_sandbox_path() {
    let sandbox = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let file_path = outside.path().join("demo.docx");
    write_office_fixture(&file_path, "word/document.xml");

    let (mut app, services, _tmp) =
        build_office_app_with_roots(vec![sandbox.path().to_path_buf()]).await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pass123").await;

    let body = json!({
        "request_id":nomifun_common::generate_id(), "doc_type":"word",
        "file_path": file_path.to_str().unwrap(),
        "workspace": outside.path().to_str().unwrap()
    });
    let req = json_with_token("POST", "/api/office-preview/start", body, &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["state"], "queued");
}

#[tokio::test]
async fn wp6_word_preview_without_workspace_rejects_non_sandbox_path() {
    let sandbox = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let file_path = outside.path().join("demo.docx");
    write_office_fixture(&file_path, "word/document.xml");

    let (mut app, services, _tmp) =
        build_office_app_with_roots(vec![sandbox.path().to_path_buf()]).await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pass123").await;

    let body = json!({
        "request_id":nomifun_common::generate_id(), "doc_type":"word",
        "file_path": file_path.to_str().unwrap()
    });
    let req = json_with_token("POST", "/api/office-preview/start", body, &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let json = body_json(resp).await;
    assert_eq!(json["code"], "PATH_OUTSIDE_SANDBOX");
}

#[tokio::test]
async fn ep1_excel_preview_with_workspace_accepts_non_sandbox_path() {
    let sandbox = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let file_path = outside.path().join("demo.xlsx");
    write_office_fixture(&file_path, "xl/workbook.xml");

    let (mut app, services, _tmp) =
        build_office_app_with_roots(vec![sandbox.path().to_path_buf()]).await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pass123").await;

    let body = json!({
        "request_id":nomifun_common::generate_id(), "doc_type":"excel",
        "file_path": file_path.to_str().unwrap(),
        "workspace": outside.path().to_str().unwrap()
    });
    let req = json_with_token("POST", "/api/office-preview/start", body, &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["state"], "queued");
}

#[tokio::test]
async fn pp1_ppt_preview_with_workspace_accepts_non_sandbox_path() {
    let sandbox = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let file_path = outside.path().join("demo.pptx");
    write_office_fixture(&file_path, "ppt/presentation.xml");

    let (mut app, services, _tmp) =
        build_office_app_with_roots(vec![sandbox.path().to_path_buf()]).await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pass123").await;

    let body = json!({
        "request_id":nomifun_common::generate_id(), "doc_type":"ppt",
        "file_path": file_path.to_str().unwrap(),
        "workspace": outside.path().to_str().unwrap()
    });
    let req = json_with_token("POST", "/api/office-preview/start", body, &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["state"], "queued");
}

// ── SH-1: Save snapshot ─────────────────────────────────────────────

#[tokio::test]
async fn sh1_save_snapshot() {
    let (mut app, services, _tmp) = build_office_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pass123").await;

    let body = json!({
        "target": snapshot_target(),
        "content": "# Hello World"
    });
    let req = json_with_token("POST", "/api/preview-history/save", body, &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["success"], true);

    let data = &json["data"];
    assert!(data["snapshot_id"].is_string());
    assert!(!data["snapshot_id"].as_str().unwrap().is_empty());
    assert!(data.get("id").is_none());
    assert!(data["created_at"].is_number());
    assert_eq!(data["size"], 13); // "# Hello World".len()
    assert_eq!(data["content_type"], "markdown");
}

// ── SH-2: List snapshots ────────────────────────────────────────────

#[tokio::test]
async fn sh2_list_snapshots() {
    let (mut app, services, _tmp) = build_office_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pass123").await;

    for i in 0..3 {
        let body = json!({
            "target": snapshot_target(),
            "content": format!("content {i}")
        });
        let req = json_with_token("POST", "/api/preview-history/save", body, &token, &csrf);
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    let body = json!({"target": snapshot_target()});
    let req = json_with_token("POST", "/api/preview-history/list", body, &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["success"], true);

    let snapshots = json["data"].as_array().unwrap();
    assert_eq!(snapshots.len(), 3);
}

// ── SH-3: Get snapshot content ──────────────────────────────────────

#[tokio::test]
async fn sh3_get_snapshot_content() {
    let (mut app, services, _tmp) = build_office_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pass123").await;

    let save_body = json!({
        "target": snapshot_target(),
        "content": "# Hello"
    });
    let req = json_with_token(
        "POST",
        "/api/preview-history/save",
        save_body,
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    let save_json = body_json(resp).await;
    let snapshot_id = save_json["data"]["snapshot_id"].as_str().unwrap();

    let get_body = json!({
        "target": snapshot_target(),
        "snapshot_id": snapshot_id
    });
    let req = json_with_token(
        "POST",
        "/api/preview-history/get-content",
        get_body,
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["content"], "# Hello");
    assert_eq!(json["data"]["snapshot"]["snapshot_id"], snapshot_id);
    assert!(json["data"]["snapshot"].get("id").is_none());
}

// ── SH-4: Get nonexistent snapshot ──────────────────────────────────

#[tokio::test]
async fn sh4_get_nonexistent_snapshot() {
    let (mut app, services, _tmp) = build_office_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pass123").await;

    let body = json!({
        "target": snapshot_target(),
        "snapshot_id": "0190f5fe-7c00-7a00-8000-000000000099"
    });
    let req = json_with_token(
        "POST",
        "/api/preview-history/get-content",
        body,
        &token,
        &csrf,
    );
    let resp = app.clone().oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["success"], true);
    assert!(json["data"].is_null());
}

// ── SH-5: Snapshot trimming at 50 limit ─────────────────────────────

#[tokio::test]
async fn sh5_snapshot_trim_at_limit() {
    let (mut app, services, _tmp) = build_office_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pass123").await;

    for i in 0..52 {
        let body = json!({
            "target": snapshot_target(),
            "content": format!("snap {i}")
        });
        let req = json_with_token("POST", "/api/preview-history/save", body, &token, &csrf);
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    let body = json!({"target": snapshot_target()});
    let req = json_with_token("POST", "/api/preview-history/list", body, &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();

    let json = body_json(resp).await;
    let snapshots = json["data"].as_array().unwrap();
    assert!(
        snapshots.len() <= 50,
        "expected at most 50 snapshots, got {}",
        snapshots.len()
    );
}

// ── SH-6: Different targets are isolated ────────────────────────────

#[tokio::test]
async fn sh6_different_targets_isolated() {
    let (mut app, services, _tmp) = build_office_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pass123").await;

    let target_a = json!({"content_type": "markdown", "file_path": "/a.md"});
    let target_b = json!({"content_type": "code", "file_path": "/b.rs"});

    let body_a = json!({"target": target_a, "content": "AAA"});
    let req = json_with_token("POST", "/api/preview-history/save", body_a, &token, &csrf);
    app.clone().oneshot(req).await.unwrap();

    let body_b = json!({"target": target_b, "content": "BBB"});
    let req = json_with_token("POST", "/api/preview-history/save", body_b, &token, &csrf);
    app.clone().oneshot(req).await.unwrap();

    let list_a = json!({"target": target_a});
    let req = json_with_token("POST", "/api/preview-history/list", list_a, &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    let json_a = body_json(resp).await;
    assert_eq!(json_a["data"].as_array().unwrap().len(), 1);

    let list_b = json!({"target": target_b});
    let req = json_with_token("POST", "/api/preview-history/list", list_b, &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    let json_b = body_json(resp).await;
    assert_eq!(json_b["data"].as_array().unwrap().len(), 1);
}

// ── SH-7: Target with multiple fields produces different hash ───────

#[tokio::test]
async fn sh7_target_field_combination_different_hash() {
    let (mut app, services, _tmp) = build_office_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pass123").await;

    let target_simple = json!({"content_type": "markdown", "file_path": "/a.md"});
    let target_complex = json!({
        "content_type": "markdown",
        "file_path": "/a.md",
        "workspace": "/ws",
        "conversation_id": "0190f5fe-7c00-7a00-8000-000000000016"
    });

    let body = json!({"target": target_simple, "content": "simple"});
    let req = json_with_token("POST", "/api/preview-history/save", body, &token, &csrf);
    app.clone().oneshot(req).await.unwrap();

    let body = json!({"target": target_complex, "content": "complex"});
    let req = json_with_token("POST", "/api/preview-history/save", body, &token, &csrf);
    app.clone().oneshot(req).await.unwrap();

    let list = json!({"target": target_simple});
    let req = json_with_token("POST", "/api/preview-history/list", list, &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    assert_eq!(
        json["data"].as_array().unwrap().len(),
        1,
        "simple target should only have 1 snapshot"
    );

    let list = json!({"target": target_complex});
    let req = json_with_token("POST", "/api/preview-history/list", list, &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    let json = body_json(resp).await;
    assert_eq!(
        json["data"].as_array().unwrap().len(),
        1,
        "complex target should only have 1 snapshot"
    );
}

// Capability reads never accept port numbers or inactive opaque tokens.
#[tokio::test]
async fn artifact_capabilities_require_an_active_grant() {
    for capability in ["8080".to_owned(), "a".repeat(64)] {
        let (app, _services, _tmp) = build_office_app().await;
        let request = get_request(&format!(
            "/api/office-preview-artifacts/{capability}/?request_id={}&nonce={}",
            uuid::Uuid::now_v7(),
            uuid::Uuid::now_v7()
        ));
        assert_eq!(
            app.oneshot(request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
}

#[tokio::test]
async fn stop_before_start_prevents_resurrection() {
    let (mut app, services, tmp) = build_office_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "pass123").await;
    let request_id = nomifun_common::generate_id();
    let stop = json_with_token(
        "POST",
        "/api/office-preview/stop",
        json!({"request_id":request_id}),
        &token,
        &csrf,
    );
    assert_eq!(
        app.clone().oneshot(stop).await.unwrap().status(),
        StatusCode::OK
    );
    let file = tmp.path().join("test.docx");
    write_office_fixture(&file, "word/document.xml");
    let start = json_with_token(
        "POST",
        "/api/office-preview/start",
        json!({"request_id":request_id,"doc_type":"word","file_path":file}),
        &token,
        &csrf,
    );
    let response = body_json(app.oneshot(start).await.unwrap()).await;
    assert_eq!(response["data"]["error"]["code"], "CANCELLED");
}
