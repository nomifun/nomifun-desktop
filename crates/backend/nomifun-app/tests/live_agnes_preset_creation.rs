//! Opt-in real-provider smoke for the canonical general Agent Session and
//! Creation Action with two image routes and no image default. Pass an Agnes
//! API key as one line on stdin. The credential is only kept in memory and
//! the test uses an isolated temporary database.

use std::io::BufRead as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use nomifun_app::compatibility::{AppServices, build_module_states, create_router_with_states};
use nomifun_app::{AppConfig, AuthPolicy};
use nomifun_db::{
    CreateProviderParams, IClientPreferenceRepository, IProviderModelRepository,
    IProviderRepository, NewProviderModel, NewProviderModelCapability,
    SqliteClientPreferenceRepository, SqliteProviderModelRepository,
    SqliteProviderRepository,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use zeroize::Zeroizing;

const TRUST: &str = "live-agnes-preset-creation";
const CHAT_MODEL: &str = "agnes-2.5-flash";
const IMAGE_MODEL: &str = "agnes-image-2.1-flash";

async fn request(router: &axum::Router, method: Method, path: &str, body: Option<Value>) -> Value {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("x-nomi-local-trust", TRUST);
    let body = if let Some(value) = body {
        builder = builder.header("content-type", "application/json");
        Body::from(serde_json::to_vec(&value).expect("serialize test request"))
    } else {
        Body::empty()
    };
    let response = router.clone().oneshot(builder.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value: Value = serde_json::from_slice(&bytes).expect("JSON API response");
    assert!(status.is_success(), "{path}: HTTP {status}, code={}", value["code"]);
    assert_eq!(value["success"], true, "{path}: unexpected API envelope");
    value["data"].clone()
}

async fn wait_for_turn(pool: &nomifun_db::SqlitePool, operation_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let state: Option<(String, Option<String>)> = nomifun_db::sqlx::query_as(
            "SELECT state, error_json FROM agent_turns WHERE operation_id = ?",
        )
        .bind(operation_id)
        .fetch_optional(pool)
        .await
        .unwrap();
        match state {
            Some((state, _)) if state == "completed" => return,
            Some((state, error)) if state == "failed" || state == "canceled" => {
                let detail = error
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<Value>(value).ok())
                    .map(|value| {
                        let keys = value.as_object().map(|object| object.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
                        let code = value["code"].as_str()
                            .or_else(|| value["error_code"].as_str())
                            .or_else(|| value["error"]["code"].as_str())
                            .unwrap_or("UNKNOWN");
                        format!("code={code} keys={keys:?}")
                    })
                    .unwrap_or_else(|| "code=UNKNOWN".into());
                panic!("Agent turn {state}: {detail}");
            }
            _ => {}
        }
        assert!(Instant::now() < deadline, "Agent turn did not settle");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn assert_chat_reply(pool: &nomifun_db::SqlitePool, session_id: &str) {
    let rows: Vec<String> = nomifun_db::sqlx::query_scalar(
        "SELECT projection_json FROM agent_messages WHERE session_id = ? AND presentation_intent = 'message'",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .unwrap();
    assert!(rows.iter().any(|row| {
        serde_json::from_str::<Value>(row)
            .ok()
            .and_then(|value| value["content"].as_str().map(str::to_owned))
            .is_some_and(|content| content.contains("AGNES_PRESET_OK"))
    }), "Agent did not produce the requested chat reply");
}

fn resource_selections(template: &str, companion_id: &str, customer_id: &str) -> Value {
    let resource = |kind: &str, id: &str| json!({"resource_kind": kind, "resource_id": id});
    match template {
        "chat.minimal" => json!([]),
        "assistant.general" => json!([
            resource("computer", "local-desktop"),
            resource("process_session", "managed-process-session"),
            resource("project_memory", "default-project-memory"),
            resource("scheduler", "installation-scheduler"),
            resource("workspace", "default-workspace"),
        ]),
        "coding.codex" => json!([
            resource("process_session", "managed-process-session"),
            resource("project_memory", "default-project-memory"),
            resource("workspace", "default-workspace"),
        ]),
        "companion.default" => json!([
            resource("companion", companion_id),
            resource("companion_memory", companion_id),
            resource("scheduler", "installation-scheduler"),
        ]),
        "customer-service.default" => json!([resource("customer", customer_id)]),
        "creative-studio.default" => json!([
            resource("asset_library", "creative-studio-assets"),
            resource("process_session", "managed-process-session"),
            resource("project_memory", "default-project-memory"),
            resource("workspace", "default-workspace"),
        ]),
        other => panic!("unknown official template {other}"),
    }
}

async fn verify_general_image(router: &axum::Router, pool: &nomifun_db::SqlitePool, provider_id: &str, session_id: &str) {
    let turn = request(
        router,
        Method::POST,
        &format!("/api/agent-sessions/{session_id}/turns"),
        Some(json!({
            "input": {"content": "生成一只小猫咪图片"},
            "idempotency_key": Uuid::now_v7().to_string(),
        })),
    )
    .await;
    wait_for_turn(pool, turn["operation_id"].as_str().expect("accepted image turn")).await;
    eprintln!("AGNES_LIVE_AGENT_TURN image=completed");

    let deadline = Instant::now() + Duration::from_secs(180);
    let asset_id = loop {
        let page = request(
            router,
            Method::GET,
            &format!("/api/agent-sessions/{session_id}/creation-tasks"),
            None,
        )
        .await;
        let tasks = page["items"].as_array().expect("Creation task page");
        if let Some(task) = tasks.first() {
            assert_eq!(task["provider_id"], provider_id);
            assert_eq!(task["model"], IMAGE_MODEL);
            match task["status"].as_str() {
                Some("succeeded") => {
                    assert_eq!(tasks.len(), 1, "one explicit image request must create one task");
                    break task["result_asset_ids"][0]
                        .as_str()
                        .expect("generated asset ID")
                        .to_owned();
                }
                Some("failed" | "canceled") => {
                    panic!("Creation task failed: code={}", task["error"]["code"]);
                }
                _ => {}
            }
        }
        assert!(Instant::now() < deadline, "Creation task did not succeed");
        tokio::time::sleep(Duration::from_millis(500)).await;
    };

    let file = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/creative-studio/files/{asset_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(file.status(), StatusCode::OK, "generated file must be readable");
    let bytes = file.into_body().collect().await.unwrap().to_bytes();
    let image = image::load_from_memory(&bytes).expect("generated image must decode");
    assert!(image.width() > 0 && image.height() > 0);
    eprintln!("AGNES_LIVE_IMAGE_RESULT status=succeeded width={} height={}", image.width(), image.height());
}

#[tokio::test]
#[ignore = "requires an Agnes API key supplied on stdin and makes billable real model calls"]
async fn official_presets_chat_and_general_agent_auto_routes_image_without_default() {
    let mut line = Zeroizing::new(String::new());
    std::io::stdin().lock().read_line(&mut line).expect("read Agnes key from stdin");
    let credential = line.trim();
    assert!(!credential.is_empty(), "pass the Agnes API key on stdin");

    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("work")).unwrap();
    let database = nomifun_db::init_database_memory().await.unwrap();
    let mut services = AppServices::from_config(
        database,
        &AppConfig {
            data_dir: root.path().join("data"),
            work_dir: root.path().join("work"),
            auth_policy: AuthPolicy::TrustLocalToken,
            local_trust_secret: Some(Arc::from(TRUST)),
            ..AppConfig::default()
        },
    )
    .await
    .unwrap();
    services.attached_chrome = Some(nomifun_app::AttachedChromeProviderService::new());
    let pool = services.database.pool().clone();
    let provider_id = Uuid::now_v7().to_string();
    let credentials = Zeroizing::new(json!({"api_keys": [credential]}).to_string());
    let encrypted = nomifun_common::encrypt_string(&credentials, &services.encryption_key).unwrap();
    let providers = SqliteProviderRepository::new(pool.clone());
    let provider = providers
        .create(
            CreateProviderParams {
                provider_id: Some(&provider_id),
                platform: "agnes",
                name: "Agnes live Agent smoke",
                base_url: "https://apihub.agnes-ai.com/v1",
                auth_scheme: "bearer",
                credentials_encrypted: &encrypted,
                enabled: true,
                bedrock_config: None,
                sort_order: Some(-10),
            },
            &NewProviderModel {
                model: CHAT_MODEL,
                enabled: true,
                sort_order: 0,
                description: None,
                capabilities: &[NewProviderModelCapability {
                    task: "chat",
                    traits: "[]",
                    protocol: "openai.chat_text",
                    connection_role: "default",
                    provider_params: "{}",
                    ..Default::default()
                }],
            },
            &[],
        )
        .await
        .unwrap();
    SqliteProviderModelRepository::new(pool.clone())
        .save(
            &provider_id,
            provider.0.config_revision,
            &NewProviderModel {
                model: IMAGE_MODEL,
                enabled: true,
                sort_order: 0,
                description: None,
                capabilities: &[NewProviderModelCapability {
                    task: "image_generation",
                    traits: "[]",
                    protocol: "agnes.images",
                    connection_role: "default",
                    provider_params: "{}",
                    ..Default::default()
                }],
            },
        )
        .await
        .unwrap();
    let alternate_provider_id = Uuid::now_v7().to_string();
    providers
        .create(
            CreateProviderParams {
                provider_id: Some(&alternate_provider_id),
                platform: "agnes",
                name: "Agnes alternate image route",
                base_url: "https://apihub.agnes-ai.com/v1",
                auth_scheme: "bearer",
                credentials_encrypted: &encrypted,
                enabled: true,
                bedrock_config: None,
                sort_order: Some(10),
            },
            &NewProviderModel {
                model: IMAGE_MODEL,
                enabled: true,
                sort_order: 0,
                description: None,
                capabilities: &[NewProviderModelCapability {
                    task: "image_generation",
                    traits: "[]",
                    protocol: "agnes.images",
                    connection_role: "default",
                    provider_params: "{}",
                    ..Default::default()
                }],
            },
            &[],
        )
        .await
        .unwrap();
    let chat_default = json!({"provider_id": provider_id, "model": CHAT_MODEL}).to_string();
    SqliteClientPreferenceRepository::new(pool.clone())
        .upsert_batch(&[("nomi.defaultModel", &chat_default)])
        .await
        .unwrap();
    let image_defaults: i64 = nomifun_db::sqlx::query_scalar(
        "SELECT COUNT(*) FROM client_preferences WHERE key = 'models.default.imageGeneration'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let image_routes: i64 = nomifun_db::sqlx::query_scalar(
        "SELECT COUNT(*) FROM provider_model_capabilities WHERE task = 'image_generation'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(image_defaults, 0, "the live route must run without an image default");
    assert_eq!(image_routes, 2, "the live route must choose between multiple image routes");

    let (states, _channels) = build_module_states(&services).await;
    let router = create_router_with_states(&services, states);
    let companion = request(
        &router,
        Method::POST,
        "/api/companion/companions",
        Some(json!({"name": "Agnes test companion", "character": "ink"})),
    )
    .await;
    let companion_id = companion["companion_id"].as_str().expect("test companion ID");
    let customer = request(
        &router,
        Method::POST,
        "/api/customer-service/agents",
        Some(json!({
            "name": "Agnes test customer", "provider_id": provider_id, "model": CHAT_MODEL,
        })),
    )
    .await;
    let customer_id = customer["cs_agent_id"].as_str().expect("test customer Agent ID");
    for template in [
        "assistant.general",
        "chat.minimal",
        "coding.codex",
        "companion.default",
        "customer-service.default",
        "creative-studio.default",
    ] {
        let editor = request(
            &router,
            Method::POST,
            &format!("/api/agent-presets/from-template/{template}"),
            Some(json!({
                "model": {"provider_id": provider_id, "model": CHAT_MODEL},
                "reuse_existing": false,
                "display_name": "Agnes live preset",
                "model_route_refs": {},
                "chat_route_records": {},
            })),
        )
        .await;
        let preset_id = editor["preset"]["preset_id"].as_str().expect("saved preset ID");
        let session = request(
            &router,
            Method::POST,
            "/api/agent-sessions",
            Some(json!({
                "preset_id": preset_id,
                "title": format!("Agnes live {template}"),
                "resource_selections": resource_selections(template, companion_id, customer_id),
            })),
        )
        .await;
        let session_id = session["agent_session_id"].as_str().expect("opened session ID");
        assert_eq!(session["state"], "ready");
        let turn = request(
            &router,
            Method::POST,
            &format!("/api/agent-sessions/{session_id}/turns"),
            Some(json!({
                "input": {"content": "请只回复 AGNES_PRESET_OK，不要调用工具。"},
                "idempotency_key": Uuid::now_v7().to_string(),
            })),
        )
        .await;
        let operation_id = turn["operation_id"].as_str().expect("accepted turn");
        wait_for_turn(&pool, operation_id).await;
        assert_chat_reply(&pool, session_id).await;
        eprintln!("AGNES_LIVE_PRESET_TURN {template}=completed");
        if template == "assistant.general" {
            verify_general_image(&router, &pool, &provider_id, session_id).await;
        }
    }

    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}
