//! Opt-in real text-model -> media-service regression. Credentials arrive as
//! two lines on stdin (Step Plan, then Agnes), never command arguments or logs.
//! All configuration, Sessions and generated files belong to an isolated root.

use std::io::BufRead as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use nomifun_app::compatibility::{AppServices, build_module_states, create_router_with_states};
use nomifun_app::{AppConfig, AuthPolicy};
use nomifun_db::{CreateProviderParams, IClientPreferenceRepository, IProviderModelRepository, IProviderRepository, NewProviderModel, NewProviderModelCapability, SqliteClientPreferenceRepository, SqliteProviderModelRepository, SqliteProviderRepository};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use zeroize::Zeroizing;

const TRUST: &str = "live-text-model-creation";
const CHAT: &str = "step-3.7-flash";

async fn request(router: &axum::Router, method: Method, path: &str, body: Option<Value>) -> Value {
    let mut builder = Request::builder().method(method).uri(path).header("x-nomi-local-trust", TRUST);
    let body = match body {
        Some(value) => { builder = builder.header("content-type", "application/json"); Body::from(value.to_string()) },
        None => Body::empty(),
    };
    let response = router.clone().oneshot(builder.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value: Value = serde_json::from_slice(&bytes).expect("API JSON");
    assert!(status.is_success() && value["success"] == true, "API {path}: HTTP {status}, code={}", value["code"]);
    value["data"].clone()
}

async fn provider(services: &AppServices, platform: &str, url: &str, model: &str, task: &str, protocol: &str, key: &str, sort: i64) -> String {
    let id = Uuid::now_v7().to_string();
    let credentials = Zeroizing::new(json!({"api_keys":[key]}).to_string());
    let encrypted = nomifun_common::encrypt_string(&credentials, &services.encryption_key).unwrap();
    SqliteProviderRepository::new(services.database.pool().clone()).create(
        CreateProviderParams { provider_id: Some(&id), platform, name: "Isolated live media route", base_url: url, auth_scheme: "bearer", credentials_encrypted: &encrypted, enabled: true, bedrock_config: None, sort_order: Some(sort) },
        &NewProviderModel { model, enabled: true, sort_order: 0, description: None, capabilities: &[NewProviderModelCapability { task, protocol, traits: "[]", connection_role: "default", provider_params: "{}", ..Default::default() }] }, &[],
    ).await.unwrap();
    id
}

async fn verify_image(router: &axum::Router, pool: &nomifun_db::SqlitePool, preset: &str, expected_provider: &str, image_model: &str) {
    let session = request(router, Method::POST, "/api/agent-sessions", Some(json!({
        "preset_id":preset, "title":"Text model media regression",
        "resource_selections":[
            {"resource_kind":"computer","resource_id":"local-desktop"},
            {"resource_kind":"process_session","resource_id":"managed-process-session"},
            {"resource_kind":"project_memory","resource_id":"default-project-memory"},
            {"resource_kind":"scheduler","resource_id":"installation-scheduler"},
            {"resource_kind":"workspace","resource_id":"default-workspace"}
        ]
    }))).await;
    let sid = session["agent_session_id"].as_str().unwrap();
    let receipt = request(router, Method::POST, &format!("/api/agent-sessions/{sid}/turns"), Some(json!({
        "input":{"content":"生成一张小猫咪图片"}, "idempotency_key":Uuid::now_v7().to_string()
    }))).await;
    let operation = receipt["operation_id"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(240);
    loop {
        let state: String = nomifun_db::sqlx::query_scalar("SELECT state FROM agent_turns WHERE operation_id=?").bind(operation).fetch_one(pool).await.unwrap();
        if state == "completed" { break; }
        assert!(!matches!(state.as_str(), "failed" | "canceled"), "live text Turn reached {state}");
        assert!(Instant::now() < deadline, "live text Turn timeout");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let source: String = nomifun_db::sqlx::query_scalar("SELECT source_message_id FROM agent_turns WHERE operation_id=?").bind(operation).fetch_one(pool).await.unwrap();
    let denied: i64 = nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='tool/result-recorded' AND inline_json LIKE '%frozen Session execution ceiling%'").bind(sid).fetch_one(pool).await.unwrap();
    assert_eq!(denied, 0, "an exposed media tool must remain inside its frozen ceiling");
    let deadline = Instant::now() + Duration::from_secs(240);
    let asset = loop {
        let page = request(router, Method::GET, &format!("/api/agent-sessions/{sid}/creation-tasks"), None).await;
        let tasks = page["items"].as_array().unwrap();
        if let Some(task) = tasks.first() {
            assert_eq!(tasks.len(), 1, "one user request creates one task");
            assert_eq!(task["provider_id"], expected_provider);
            assert_eq!(task["model"], image_model);
            assert_eq!(task["owner"]["conversation_id"], sid);
            assert_eq!(task["owner"]["message_id"], source);
            match task["status"].as_str() {
                Some("succeeded") => break task["result_asset_ids"][0].as_str().unwrap().to_owned(),
                Some("failed" | "canceled") => panic!("live image task {}: kind={}", task["status"], task["error"]["kind"]),
                _ => {},
            }
        }
        assert!(Instant::now() < deadline, "live image task timeout; task_count={}", tasks.len());
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    let response = router.clone().oneshot(Request::builder().uri(format!("/api/creative-studio/files/{asset}")).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let image = image::load_from_memory(&bytes).expect("live generated image must decode");
    assert!(image.width() > 0 && image.height() > 0);
    eprintln!("LIVE_TEXT_MEDIA chat={CHAT} image={image_model} status=succeeded width={} height={}", image.width(), image.height());
}

#[tokio::test(flavor="multi_thread", worker_threads=4)]
#[ignore = "requires Step Plan and Agnes keys on stdin; makes two real image generation calls"]
async fn general_step_plan_text_model_generates_stepfun_and_agnes_images() {
    let mut step_key = Zeroizing::new(String::new());
    let mut agnes_key = Zeroizing::new(String::new());
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    input.read_line(&mut step_key).expect("Step Plan credential on stdin");
    input.read_line(&mut agnes_key).expect("Agnes credential on stdin");
    drop(input);
    assert!(!step_key.trim().is_empty() && !agnes_key.trim().is_empty(), "two credentials required");
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("work")).unwrap();
    let mut services = AppServices::from_config(nomifun_db::init_database_memory().await.unwrap(), &AppConfig {
        data_dir: root.path().join("data"), work_dir: root.path().join("work"),
        auth_policy: AuthPolicy::TrustLocalToken, local_trust_secret: Some(Arc::from(TRUST)), ..Default::default()
    }).await.unwrap();
    services.attached_chrome = Some(nomifun_app::AttachedChromeProviderService::new());
    let pool = services.database.pool().clone();
    let step = provider(&services, "stepfun-plan", "https://api.stepfun.com/step_plan/v1", CHAT, "chat", "openai.chat_text", step_key.trim(), -10).await;
    let step_stored = SqliteProviderRepository::new(pool.clone()).find_by_id(&step).await.unwrap().unwrap();
    SqliteProviderModelRepository::new(pool.clone()).save(&step, step_stored.config_revision, &NewProviderModel {
        model:"step-image-edit-2", enabled:true, sort_order:0, description:None,
        capabilities:&[NewProviderModelCapability {task:"image_generation", protocol:"stepfun.images", traits:"[]", connection_role:"default", provider_params:"{}", ..Default::default()}],
    }).await.unwrap();
    let agnes = provider(&services, "agnes", "https://apihub.agnes-ai.com/v1", "agnes-image-2.1-flash", "image_generation", "agnes.images", agnes_key.trim(), 10).await;
    let chat_default = json!({"provider_id":step,"model":CHAT}).to_string();
    let prefs = SqliteClientPreferenceRepository::new(pool.clone());
    prefs.upsert_batch(&[("nomi.defaultModel", &chat_default)]).await.unwrap();
    let (states, _channels) = build_module_states(&services).await;
    let router = create_router_with_states(&services, states);
    let editor = request(&router, Method::POST, "/api/agent-presets/from-template/assistant.general", Some(json!({
        "display_name":"Live General media regression", "reuse_existing":false,
        "model":{"provider_id":step,"model":CHAT}, "model_route_refs":{},"chat_route_records":{}
    }))).await;
    let preset = editor["preset"]["preset_id"].as_str().unwrap();
    for (id, model) in [(&step, "step-image-edit-2"), (&agnes, "agnes-image-2.1-flash")] {
        let selected = json!({"provider_id":id,"model":model}).to_string();
        prefs.upsert_batch(&[("models.default.imageGeneration", &selected)]).await.unwrap();
        verify_image(&router, &pool, preset, id, model).await;
    }
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}
