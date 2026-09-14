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
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
    )
}
async fn ok(router: &Router, method: &str, path: &str, body: Value, key: &str) -> Value {
    let (status, value) = call(router, method, path, body, key).await;
    assert_eq!(status, StatusCode::OK, "{path}: {value}");
    value["data"].clone()
}

fn runtime_draft(editor: &Value, engine: &Value) -> Value {
    let mut draft = editor["draft"].clone();
    draft["document"]["runtime_engine"] = engine.clone();
    draft
}

async fn save_runtime(router: &Router, editor: &Value, engine: &Value) -> Value {
    let draft = runtime_draft(editor, engine);
    let path = format!("/api/agent-presets/{}/revisions", editor["preset"]["preset_id"].as_str().unwrap());
    let saved = ok(router, "POST", &path, json!({
        "expected_current_revision": editor["revision"]["reference"],
        "draft": draft
    }), "save-runtime").await;
    assert_eq!(saved["revision"]["document"]["runtime_engine"], *engine);
    assert_eq!(saved["revision"]["document"]["enabled_capabilities"], editor["draft"]["document"]["enabled_capabilities"]);
    assert!(saved["resolved_snapshot_ref"]["snapshot_digest"].as_str().is_some_and(|digest| digest.len() == 64));
    saved
}

async fn assert_runtime_save_rejected(router: &Router, editor: &Value, engine: &Value, key: &str) {
    let path = format!("/api/agent-presets/{}", editor["preset"]["preset_id"].as_str().unwrap());
    // Save owns compilation now: reject an unavailable Engine without creating
    // a revision or changing the previously saved Agent configuration.
    let (status, error) = call(router, "POST", &format!("{path}/revisions"), json!({
        "expected_current_revision": editor["revision"]["reference"],
        "draft": runtime_draft(editor, engine)
    }), key).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
    assert_eq!(error["code"], "PRESET_REVISION_SAVE_FAILED", "{error}");
    assert!(error["details"]["diagnostics"].as_array().unwrap().iter()
        .any(|diagnostic| diagnostic["code"] == "AGENT_RUNTIME_ENGINE_UNAVAILABLE"), "{error}");
    let unchanged = ok(router, "GET", &format!("{path}/editor"), json!({}), key).await;
    assert_eq!(unchanged["revision"], editor["revision"]);
    assert_eq!(unchanged["draft"], editor["draft"]);
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

async fn assert_coding_product_admission(
    services: &nomifun_app::compatibility::AppServices,
    saved: &Value,
    binding: &nomifun_api_types::RuntimeEngineBinding,
) {
    use nomifun_agent_contracts::{ResolvedCapability, ResolvedSnapshotEnvelope, digest_payload};

    assert_eq!(binding.family_id, "nomifun.coding");
    let reference = &saved["revision"]["reference"];
    let stored: String = sqlx::query_scalar(
        "SELECT snapshot_json FROM nomi_agent_preset_revisions WHERE preset_id = ? AND revision_no = ? AND revision_digest = ?",
    )
    .bind(reference["preset_id"].as_str().unwrap())
    .bind(reference["revision"].as_i64().unwrap())
    .bind(reference["revision_digest"].as_str().unwrap())
    .fetch_one(services.database.pool())
    .await
    .unwrap();
    let mut snapshot: ResolvedSnapshotEnvelope = serde_json::from_str(&stored).unwrap();
    snapshot.validate().expect("saved production snapshot");
    let catalog = services.runtime_engines.catalog().unwrap();
    catalog.validate_snapshot(binding, &snapshot).expect("saved Coding admission");

    // Admission-only fixture, never installed or dispatched. Exercise the real
    // registered Coding policy with a Product ID outside its builtin allowlist.
    let digest = digest_payload(&json!({"type":"object","properties":{},"additionalProperties":false})).unwrap();
    let product: ResolvedCapability = serde_json::from_value(json!({
        "capability":{"id":"coding.admission.product","version":"1.0.0"},
        "source_package":{"id":"coding.admission.fixture","version":"1.0.0"},
        "contribution_id":"capability:coding.admission.product",
        "contribution_lock":{
            "source_kind":"plugin_product_active_release",
            "source_identity":"coding.admission.fixture",
            "plugin_product_id":"coding-admission-product",
            "contribution_id":"capability:coding.admission.product",
            "contract_digest":digest
        },
        "resolved_source":{"source_kind":"managed_local","source_identity":"coding.admission.fixture"},
        "target_artifact_digest":digest,"schema_digest":digest,
        "dependency_path":["coding.admission.product"],"required_runtime_features":[],
        "plugin_product_id":"coding-admission-product",
        "active_release":{"release_id":"coding-admission-release","artifact_id":"coding-admission-artifact",
            "release_digest":digest,"manifest_digest":digest},
        "active_release_epoch":1,"catalog_digest":digest,
        "display_name":"Coding admission fixture","description":"Exact Product action",
        "actions":[{"action_id":"coding.admission.product.invoke",
            "input_schema":format!("schema://coding.admission.product/input@1#{}", digest.as_ref()),
            "output_schema":format!("schema://coding.admission.product/output@1#{}", digest.as_ref()),
            "effect_class":"external_transmit","presentation":"function_tool"}],
        "required_resource_kinds":[],"action_allowlist":["coding.admission.product.invoke"]
    })).unwrap();
    product.validate().expect("complete exact Product capability");
    snapshot.content.capability_allowlist.insert(product.capability.id.clone());
    snapshot.content.enabled_capabilities.push(product);
    snapshot.snapshot_ref.snapshot_digest = digest_payload(&snapshot.content).unwrap();
    snapshot.validate().expect("complete Product snapshot");
    catalog.validate_snapshot(binding, &snapshot)
        .expect("validated Product must join the official Coding allowed set");

    for missing in ["plugin_product_id", "active_release", "active_release_epoch", "catalog_digest"] {
        let mut incomplete = snapshot.clone();
        let product = incomplete.content.enabled_capabilities.last_mut().unwrap();
        match missing {
            "plugin_product_id" => product.plugin_product_id = None,
            "active_release" => product.active_release = None,
            "active_release_epoch" => product.active_release_epoch = None,
            "catalog_digest" => product.catalog_digest = None,
            _ => unreachable!(),
        }
        assert!(product.validate().is_err(), "missing {missing}");
        // Keep the checksum current so rejection cannot be due to a stale hash.
        incomplete.snapshot_ref.snapshot_digest = digest_payload(&incomplete.content).unwrap();
        assert!(catalog.validate_snapshot(binding, &incomplete).is_err(),
            "official Coding admission accepted Product without {missing}");
    }
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
            Arc::new(nomifun_ai_agent::RuntimeEngineSupport::enabled_only(["fs.write".into()])),
        )
        .unwrap();
    let router = nomifun_app::compatibility::create_router(&services).await;
    let descriptor = services.runtime_engines.catalog().unwrap().list().remove(0);
    let late = services.runtime_engines.register(
        descriptor,
        Arc::new(|_, _| Box::pin(async { panic!("late factory must never run") })),
        Arc::new(nomifun_ai_agent::RuntimeEngineSupport::platform()),
    );
    assert!(late.unwrap_err().to_string().contains("closed after host assembly"));
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
            "enabled_capabilities":[{"capability":{"id":"fs.write","version":"1.0.0"}}],
            "skill_bindings":[], "system_role_provider_overrides":{},
            "persona":"Coding test", "instructions":"Use only the selected workspace tools.", "starter_prompts":[]
        }
    }), "preset").await;
    // The workbench saves the runtime into the Agent revision, not the Session DTO.
    assert!(preset["draft"]["document"].get("runtime_engine").is_none());
    let coding_selection = json!({"selector":{"selection":"exact","family_id":coding["family_id"],"build_id":coding["build_id"],"build_digest":coding["build_digest"]},"profile":"coding"});
    let saved = save_runtime(&router, &preset, &coding_selection).await;
    assert_ne!(saved["revision"]["reference"]["revision_digest"], preset["revision"]["reference"]["revision_digest"]);
    let editor_path = format!("/api/agent-presets/{}", preset["preset"]["preset_id"].as_str().unwrap());
    let preset = ok(&router, "GET", &format!("{editor_path}/editor"), json!({}), "reload-agent").await;
    assert_eq!(preset["draft"]["document"]["runtime_engine"], coding_selection);
    let request = json!({"preset_id":preset["preset"]["preset_id"], "title":"Coding production",
        "model":{"provider_id":provider,"model":"coding-fixture"},
        "resource_selections":[{"resource_kind":"workspace","resource_id":"default-workspace"}],
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
    assert_coding_product_admission(
        &services,
        &saved,
        &serde_json::from_value(created["runtime_engine_binding"].clone()).unwrap(),
    ).await;
    for (field, value) in [
        ("family_id", json!("uninstalled.runtime")),
        ("build_digest", json!("b".repeat(64))),
    ] {
        let mut invalid = coding_selection.clone();
        invalid["selector"][field] = value;
        assert_runtime_save_rejected(&router, &preset, &invalid, field).await;
    }
    let mut invalid = coding_selection.clone();
    invalid["profile"] = json!("not-installed");
    assert_runtime_save_rejected(&router, &preset, &invalid, "invalid-profile").await;
    let mut changed = request.clone();
    changed["runtime_engine"] = coding_selection.clone();
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
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let selection_path = format!("/api/agent-sessions/{session}/capability-selection");
    let capabilities_path = format!("/api/agent-sessions/{session}/capabilities");
    let before = ok(&router, "GET", &capabilities_path, json!({}), "capabilities-before").await;
    assert_eq!(before["active_capabilities"], json!(["fs.write"]));
    assert_eq!(before["enabled_capabilities"], json!(["fs.write"]));
    assert_eq!(before["generation"], 0);
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
    let live = ok(&router, "GET", &capabilities_path, json!({}), "capabilities-live").await;
    assert_eq!(live["resolved_snapshot_ref"], before["resolved_snapshot_ref"]);
    assert_eq!(live["active_capabilities"], before["active_capabilities"]);
    assert_eq!(live["enabled_capabilities"], before["enabled_capabilities"]);
    assert_eq!(live["generation"], before["generation"]);
    assert_ne!(live["state_source"], before["state_source"]);
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
    assert!(events.iter().any(|event| event.contains("context_prepared")));
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
    assert_eq!(call(&router, "POST", &fork_path, fork_request, "switch").await.0, StatusCode::UNPROCESSABLE_ENTITY);
    // Changing the Agent affects only future sessions, including model-derived variants.
    let nomi_selection = json!({"selector":{"selection":"channel","family_id":"nomifun.nomi","channel":"stable"},"profile":"default"});
    save_runtime(&router, &preset, &nomi_selection).await;
    let next = ok(&router, "POST", "/api/agent-sessions", request.clone(), "after-agent-edit").await;
    assert_eq!(next["runtime_engine_binding"]["family_id"], "nomifun.nomi");
    assert_eq!(ok(&router, "GET", &conversation_path, json!({}), "old-engine").await["extra"]["runtime_engine_binding"], created["runtime_engine_binding"]);
    assert_eq!(call(&router, "POST", "/api/agent-sessions", request.clone(), "coding-session").await.0, StatusCode::CONFLICT);
    let switch_path = format!("/api/agent-sessions/{session}/preset");
    assert_eq!(call(&router, "PUT", &switch_path, json!({"preset_id":request["preset_id"],"resource_selections":request["resource_selections"]}), "different-agent-engine").await.0, StatusCode::CONFLICT);
    let inherited = ok(&router, "POST", &fork_path, json!({"target_agent_binding":created["agent_binding"],"parent_through_seq":0}), "fork-after-agent-edit").await;
    let inherited_path = format!("/api/conversations/{}", inherited["child_agent_session_id"].as_str().unwrap());
    assert_eq!(ok(&router, "GET", &inherited_path, json!({}), "inherited-old-engine").await["extra"]["runtime_engine_binding"], created["runtime_engine_binding"]);
    // A second independent Agent may select an arbitrary host-registered runtime.
    let mut custom_document = preset["draft"]["document"].clone();
    custom_document["runtime_engine"] = json!({"selector":{"selection":"exact","family_id":"customer.runtime","build_id":"v1","build_digest":"c".repeat(64)},"profile":"workflow"});
    let custom_agent = ok(&router, "POST", "/api/agent-presets", json!({"display_name":"Custom runtime Agent","document":custom_document}), "custom-agent").await;
    // No product branch knows customer.runtime: its bundled policy controls
    // both saved capabilities and effective Session overlays.
    let mut unsupported = custom_document.clone();
    unsupported["enabled_capabilities"] = json!([{"capability":{"id":"fs.read","version":"1.0.0"}}]);
    assert!(!call(&router, "POST", "/api/agent-presets", json!({"display_name":"Unsupported custom Agent","document":unsupported}), "custom-unsupported").await.0.is_success());
    let mut custom = request;
    custom["preset_id"] = custom_agent["preset"]["preset_id"].clone();
    let mut unsupported = custom.clone();
    unsupported["capability_selection"]["enabled_skills"] = json!(["pdf"]);
    assert_eq!(call(&router, "POST", "/api/agent-sessions", unsupported, "custom-skills").await.0, StatusCode::CONFLICT);
    let custom = ok(&router, "POST", "/api/agent-sessions", custom, "custom").await;
    let selection = format!("/api/agent-sessions/{}/capability-selection", custom["agent_session_id"].as_str().unwrap());
    assert_eq!(call(&router, "PUT", &selection, json!({"capability_selection":{"enabled_skills":["pdf"],"excluded_auto_skills":[],"mcp_server_ids":[]}}), "custom-overlay").await.0, StatusCode::CONFLICT);
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
