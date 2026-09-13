//! Real product owner/registry/Broker/Kernel; only the HTTP provider is mocked.
mod common;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use nomifun_db::sqlx;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tower::ServiceExt;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

const TRUST: &str = "coding-production-local-trust";
async fn call(
    router: &Router,
    method: &str,
    path: &str,
    body: Value,
    key: &str,
) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("x-nomi-local-trust", TRUST)
                .header("content-type", "application/json")
                .header("idempotency-key", key)
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| panic!("non-JSON: {}", String::from_utf8_lossy(&bytes))),
    )
}
async fn ok(router: &Router, method: &str, path: &str, body: Value, key: &str) -> Value {
    let (status, value) = call(router, method, path, body, key).await;
    assert_eq!(status, StatusCode::OK, "{path}: {value}");
    value["data"].clone()
}
async fn wait_finished(pool: &nomifun_db::SqlitePool, session: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let status: Option<String> =
                sqlx::query_scalar("SELECT status FROM conversations WHERE conversation_id = ?")
                    .bind(session)
                    .fetch_one(pool)
                    .await
                    .unwrap();
            if status.as_deref() == Some("finished") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("production turn reached its durable terminal");
}

#[tokio::test]
async fn coding_engine_runs_real_workspace_tool_and_resumes_on_the_default_route() {
    let mock = MockServer::start().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    Mock::given(method("POST")).respond_with(move |_: &wiremock::Request| {
        let step = counter.fetch_add(1, Ordering::SeqCst);
        let args = json!({"path":"engine-proof.txt","content":"written by Coding through Kernel"}).to_string();
        let delta = if step == 0 { json!({"tool_calls":[{"index":0,"id":"write-proof","type":"function","function":{"name":"write_file","arguments":args}}]}) }
            else { json!({"content":"Coding production proof completed."}) };
        let finish = if step == 0 { "tool_calls" } else { "stop" };
        let frame = json!({"id":format!("round-{step}"),"object":"chat.completion.chunk","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
        let terminal = json!({"id":format!("round-{step}"),"object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":finish}]});
        ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
            .set_body_string(format!("data: {frame}\n\ndata: {terminal}\n\ndata: [DONE]\n\n"))
    }).mount(&mock).await;
    let root = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database_memory().await.unwrap();
    let services = nomifun_app::compatibility::AppServices::from_config(
        database,
        &nomifun_app::AppConfig {
            data_dir: root.path().join("data"),
            work_dir: root.path().join("work"),
            auth_policy: nomifun_app::AuthPolicy::TrustLocalToken,
            local_trust_secret: Some(Arc::from(TRUST)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let custom_calls = Arc::new(AtomicUsize::new(0));
    let custom_counter = custom_calls.clone();
    services
        .runtime_engines
        .register(
            nomifun_api_types::RuntimeEngineDescriptor {
                family_id: "customer.runtime".into(),
                build_id: "v1".into(),
                build_digest: "c".repeat(64),
                display_name: "Customer runtime".into(),
                host_contract_version: 1,
                supported_profiles: vec!["workflow".into()],
            },
            Arc::new(move |options, binding| {
                assert_eq!(binding.family_id, "customer.runtime");
                assert_eq!(
                    options.extra["runtime_engine_binding"]["profile"],
                    "workflow"
                );
                custom_counter.fetch_add(1, Ordering::SeqCst);
                Box::pin(async {
                    Err(nomifun_common::AppError::Conflict(
                        "fixture custom runtime refused construction".into(),
                    ))
                })
            }),
        )
        .unwrap();
    let router = nomifun_app::compatibility::create_router(&services).await;
    let provider = nomifun_common::generate_id();
    let encrypted = nomifun_common::encrypt_string(
        &json!({"api_keys":["test-only-coding"]}).to_string(),
        &services.encryption_key,
    )
    .unwrap();
    sqlx::query("INSERT INTO providers (provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, enabled, created_at, updated_at) VALUES (?, 'openai', 'Coding fixture', ?, 'bearer', ?, 1, 1, 1)")
        .bind(&provider).bind(mock.uri()).bind(encrypted).execute(services.database.pool()).await.unwrap();
    common::seed_openai_chat_model(services.database.pool(), &provider, "coding-fixture").await;
    sqlx::query("UPDATE provider_model_capabilities SET traits = ? WHERE provider_id = ? AND model = 'coding-fixture'")
        .bind(json!(["streaming", "function_calling"]).to_string()).bind(&provider)
        .execute(services.database.pool()).await.unwrap();
    let catalog = ok(&router, "GET", "/api/runtime-engines", json!({}), "catalog").await;
    let coding = catalog
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["family_id"] == "nomifun.coding")
        .unwrap();
    assert!(
        catalog
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["family_id"] == "nomifun.nomi")
    );
    let preset = ok(&router, "POST", "/api/agent-presets", json!({
        "display_name":"Coding workspace proof", "document": {
            "schema_version":"1.0.0", "model_route_refs":{}, "chat_route_records":{},
            "initial_capabilities":[{"capability":{"id":"fs.write","version":"1.0.0"}}],
            "on_demand_capabilities":[], "skill_bindings":[], "system_role_provider_overrides":{},
            "persona":"Coding test", "instructions":"Use only the selected workspace tools.", "starter_prompts":[]
        }
    }), "preset").await;
    let request = json!({"preset_id":preset["preset"]["preset_id"], "title":"Coding production",
        "model":{"provider_id":provider,"model":"coding-fixture"},
        "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"}],
        "runtime_engine":{"selector":{"selection":"exact","family_id":coding["family_id"],"build_id":coding["build_id"],"build_digest":coding["build_digest"]},"profile":"coding"},
        "capability_selection":{"enabled_skills":[],"excluded_auto_skills":[],"mcp_server_ids":[]}
    });
    let created = ok(
        &router,
        "POST",
        "/api/agent-sessions",
        request.clone(),
        "coding-session",
    )
    .await;
    let replay = ok(
        &router,
        "POST",
        "/api/agent-sessions",
        request.clone(),
        "coding-session",
    )
    .await;
    assert_eq!(created["agent_session_id"], replay["agent_session_id"]);
    let session = created["agent_session_id"].as_str().unwrap();
    assert_eq!(
        created["runtime_engine_binding"]["family_id"],
        "nomifun.coding"
    );
    for (field, value) in [
        ("family_id", json!("uninstalled.runtime")),
        ("build_digest", json!("b".repeat(64))),
    ] {
        let mut invalid = request.clone();
        invalid["runtime_engine"]["selector"][field] = value;
        assert!(
            !call(&router, "POST", "/api/agent-sessions", invalid, field)
                .await
                .0
                .is_success()
        );
    }
    let mut invalid = request.clone();
    invalid["runtime_engine"]["profile"] = json!("not-installed");
    assert!(
        !call(&router, "POST", "/api/agent-sessions", invalid, "profile")
            .await
            .0
            .is_success()
    );
    let mut changed = request.clone();
    changed.as_object_mut().unwrap().remove("runtime_engine");
    assert_eq!(
        call(
            &router,
            "POST",
            "/api/agent-sessions",
            changed,
            "coding-session"
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let selection_path = format!("/api/agent-sessions/{session}/capability-selection");
    assert_eq!(call(&router, "PUT", &selection_path, json!({"capability_selection":{"enabled_skills":["pdf"],"excluded_auto_skills":[],"mcp_server_ids":[]}}), "unsupported-skills").await.0, StatusCode::CONFLICT);
    let conversation_path = format!("/api/conversations/{session}");
    let conversation = ok(&router, "GET", &conversation_path, json!({}), "read").await;
    let proof_path = std::path::PathBuf::from(conversation["extra"]["workspace"].as_str().unwrap())
        .join("engine-proof.txt");
    let (status, _) = call(
        &router,
        "PATCH",
        &conversation_path,
        json!({"extra":{"runtime_engine_binding":null}}),
        "tamper",
    )
    .await;
    assert!(
        !status.is_success(),
        "public extra cannot remove immutable binding"
    );
    assert!(sqlx::query("UPDATE conversations SET extra = json_remove(extra, '$.runtime_engine_binding') WHERE conversation_id = ?")
        .bind(session).execute(services.database.pool()).await.is_err());
    let turn_path = format!("/api/agent-sessions/{session}/turns");
    ok(
        &router,
        "POST",
        &turn_path,
        json!({"input":{"text":"Write the proof file."},"idempotency_key":"first"}),
        "first",
    )
    .await;
    wait_finished(services.database.pool(), session).await;
    let receipts: Vec<(Option<bool>, Option<String>)> = sqlx::query_as("SELECT result_ok, result_error FROM conversation_delivery_receipts WHERE conversation_id = ? AND kind = 'turn'")
        .bind(session).fetch_all(services.database.pool()).await.unwrap();
    let debug_events: Vec<String> = sqlx::query_scalar(
        "SELECT event_json FROM conversation_runtime_events WHERE conversation_id = ? ORDER BY id",
    )
    .bind(session)
    .fetch_all(services.database.pool())
    .await
    .unwrap();
    assert!(
        receipts.iter().all(|(ok, _)| *ok == Some(true)),
        "{receipts:?}; events={debug_events:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&proof_path).unwrap(),
        "written by Coding through Kernel"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let events: Vec<String> = sqlx::query_scalar(
        "SELECT event_json FROM conversation_runtime_events WHERE conversation_id = ? ORDER BY id",
    )
    .bind(session)
    .fetch_all(services.database.pool())
    .await
    .unwrap();
    assert!(
        events.iter().any(|event| event.contains("tool_completed")),
        "{events:?}"
    );
    assert!(events.last().unwrap().contains("turn_completed"));
    assert!(
        events
            .iter()
            .any(|event| event.contains("host_tool_settled"))
    );
    let claimed: i64 = sqlx::query_scalar("SELECT count(*) FROM conversation_runtime_events WHERE conversation_id = ? AND model_claimed = 1")
        .bind(session).fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(claimed, 2);
    services
        .agent_runtime_registry
        .terminate_and_wait_result(session, None)
        .await
        .unwrap();
    ok(
        &router,
        "POST",
        &turn_path,
        json!({"input":{"text":"What did you write?"},"idempotency_key":"second"}),
        "second",
    )
    .await;
    wait_finished(services.database.pool(), session).await;
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let requests = mock.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[2].body).unwrap();
    assert!(
        body["messages"]
            .to_string()
            .contains("Write the proof file.")
    );
    assert!(body["messages"].to_string().contains("engine-proof.txt"));
    let persisted = ok(&router, "GET", &conversation_path, json!({}), "restored").await;
    assert_eq!(
        persisted["extra"]["runtime_engine_binding"],
        created["runtime_engine_binding"]
    );
    let fork_path = format!("/api/agent-sessions/{session}/forks");
    let fork_request = json!({"target_agent_binding":created["agent_binding"],"parent_through_seq":0,"title":"Inherited Coding"});
    let child = ok(&router, "POST", &fork_path, fork_request.clone(), "inherit").await;
    let child_path = format!(
        "/api/conversations/{}",
        child["child_agent_session_id"].as_str().unwrap()
    );
    assert_eq!(
        ok(&router, "GET", &child_path, json!({}), "child").await["extra"]["runtime_engine_binding"],
        created["runtime_engine_binding"]
    );
    let mut fork_request = fork_request;
    fork_request["runtime_engine"] = json!({"selector":{"selection":"channel","family_id":"nomifun.nomi","channel":"stable"},"profile":"default"});
    let child = ok(&router, "POST", &fork_path, fork_request, "switch").await;
    let child_path = format!(
        "/api/conversations/{}",
        child["child_agent_session_id"].as_str().unwrap()
    );
    assert_eq!(
        ok(&router, "GET", &child_path, json!({}), "child").await["extra"]["runtime_engine_binding"]
            ["family_id"],
        "nomifun.nomi"
    );
    let mut custom = request;
    custom["runtime_engine"] = json!({"selector":{"selection":"exact","family_id":"customer.runtime","build_id":"v1","build_digest":"c".repeat(64)},"profile":"workflow"});
    let custom = ok(&router, "POST", "/api/agent-sessions", custom, "custom").await;
    assert_eq!(
        custom["runtime_engine_binding"]["family_id"],
        "customer.runtime"
    );
    let path = format!(
        "/api/agent-sessions/{}/turns",
        custom["agent_session_id"].as_str().unwrap()
    );
    let dispatch = call(
        &router,
        "POST",
        &path,
        json!({"input":{"text":"Custom dispatch"},"idempotency_key":"custom-turn"}),
        "custom-turn",
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while custom_calls.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("custom factory was not dispatched: {dispatch:?}"));
    wait_finished(
        services.database.pool(),
        custom["agent_session_id"].as_str().unwrap(),
    )
    .await;
    assert_eq!(custom_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "custom factory refusal must not fall back to a built-in model"
    );
    services
        .agent_runtime_registry
        .terminate_and_wait_result(session, None)
        .await
        .unwrap();
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}
