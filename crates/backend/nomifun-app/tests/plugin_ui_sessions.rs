//! A real installed UI release calls the production Session owner, not a test port.
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

#[path = "common/mod.rs"]
mod common;

#[path = "plugin_ui_binding.rs"]
mod ui_binding;

#[path = "plugin_ui_admission.rs"]
mod admission;

const TRUST: &str = "plugin-ui-session-test";

// Each host mode runs in its own process: never mutate process-global env
// while other Tokio/libtest workers might be composing an application.
fn in_agent_ui_host(test: &str, enabled: bool) -> bool {
    const CHILD: &str = "NOMIFUN_TEST_AGENT_UI_CHILD";
    const OPT_IN: &str = "NOMIFUN_ALLOW_EXPERIMENTAL_AGENT_UI";
    if std::env::var(CHILD).as_deref() == Ok(test) {
        assert_eq!(std::env::var(OPT_IN).as_deref() == Ok("1"), enabled);
        return true;
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", test, "--nocapture"]).env(CHILD, test);
    if enabled {
        command.env(OPT_IN, "1");
    } else {
        command.env_remove(OPT_IN);
    }
    let status = command.status().expect("run isolated Agent UI host test");
    assert!(status.success(), "Agent UI host test failed: {test}");
    false
}

async fn request(
    router: &axum::Router,
    method: &str,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("x-nomi-local-trust", TRUST)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes))),
    )
}

async fn post(router: &axum::Router, path: &str, body: Value) -> Value {
    let (status, result) = request(router, "POST", path, body).await;
    assert!(status.is_success(), "{path}: {status} {result}");
    result["data"].clone()
}

async fn install_ui(router: &axum::Router) -> Value {
    let draft = post(router, "/api/plugins/runtimes/import/inspect", json!({
        "filename": "agent-view.html",
        "content": "<!doctype html><html><head><title>Agent view</title></head><body><main>Agent</main></body></html>"
    })).await;
    post(
        router,
        &format!("/api/plugins/drafts/{}/save", draft["id"].as_str().unwrap()),
        json!({"expected_revision": draft["revision"]}),
    )
    .await["plugin"]
        .clone()
}

fn bridge_body(surface: &Value, command: Value) -> Value {
    json!({
        "surface_capability": surface["surface_capability"],
        "active_release_epoch": surface["active_release_epoch"],
        "expected_release_digest": surface["expected_release_digest"],
        "request": {"call_id": uuid::Uuid::now_v7().to_string(),
            "target": {"target": "agent_session", "request": command}}
    })
}

fn storage_body(surface: &Value, request: Value) -> Value {
    let mut value = bridge_body(surface, Value::Null);
    value["request"]["target"] = json!({"target": "host_kv", "request": request});
    value
}

async fn get(router: &axum::Router, path: &str) -> Value {
    let (status, body) = request(router, "GET", path, Value::Null).await;
    assert!(status.is_success(), "{path}: {status} {body}");
    body["data"].clone()
}

// Use the public authoring/build/publish path, not a manufactured Catalog entry.
async fn publish_agent_view(router: &axum::Router, plugin_id: &str) -> Value {
    publish_agent_view_manifest(router, plugin_id, json!({"agent_view": {
        "name": "My Agent view", "description": "Explicit Session presentation"
    }}), false).await
}

async fn publish_agent_view_manifest(
    router: &axum::Router, plugin_id: &str, manifest: Value, acknowledge_test_warning: bool,
) -> Value {
    let base = format!("/api/plugins/runtimes/{plugin_id}");
    let w = get(router, &format!("{base}/workshop")).await;
    let w = post(router, &format!("{base}/source/edit"), json!({
        "plugin_id": plugin_id, "expected_product_revision": w["plugin"]["product_revision"],
        "project_id": w["project_id"], "expected_project_revision": w["project_revision"],
        "expected_build_generation": w["build_generation"],
        "expected_source_snapshot_digest": w["source_snapshot_digest"],
        "path": "nomifun.plugin.json", "content": manifest.to_string()
    })).await;
    let w = post(router, &format!("{base}/build"), json!({
        "plugin_id": plugin_id, "expected_product_revision": w["plugin"]["product_revision"],
        "project_id": w["project_id"], "expected_project_revision": w["project_revision"],
        "expected_build_generation": w["build_generation"],
        "expected_source_snapshot_digest": w["source_snapshot_digest"],
        "expected_dependency_lock_digest": w["dependency_lock_digest"]
    })).await;
    post(router, &format!("{base}/publish"), json!({
        "plugin_id": plugin_id, "expected_product_revision": w["plugin"]["product_revision"],
        "expected_pointer_revision": w["plugin"]["releases"]["pointer_revision"],
        "expected_active_release_epoch": w["plugin"]["releases"]["active_release_epoch"],
        "ready_release_id": w["plugin"]["releases"]["ready"]["release_id"],
        "expected_ready_release_digest": w["plugin"]["releases"]["ready"]["release_digest"],
        "expected_active_release_digest": w["plugin"]["releases"]["active"]["release_digest"],
        "acknowledge_test_warning": acknowledge_test_warning
    })).await["plugin"].clone()
}

fn latest_user_input(expected: &'static str) -> impl wiremock::Match {
    move |request: &wiremock::Request| {
        request.body_json::<Value>().ok().is_some_and(|body| {
            body["messages"].as_array().and_then(|messages| messages.last())
                .is_some_and(|message| message["role"] == "user" && message["content"] == expected)
        })
    }
}

#[tokio::test]
async fn installed_ui_uses_owned_session_commands_and_never_replays_a_turn_on_reopen() {
    if !in_agent_ui_host("installed_ui_uses_owned_session_commands_and_never_replays_a_turn_on_reopen", true) {
        return;
    }
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let upstream = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/step_plan/v1/chat/completions"))
        .and(latest_user_input("hello from plugin UI"))
        .respond_with(wiremock::ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(concat!(
                r#"data: {"choices":[{"index":0,"delta":{"role":"assistant","content":"PLUGIN_UI_REPLY"},"finish_reason":null}]}"#, "\n\n",
                r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#, "\n\n",
                "data: [DONE]\n\n")))
        .expect(3).mount(&upstream).await;
    let provider = post(&router, "/api/providers", json!({
        "platform": "stepfun-plan", "name": "Plugin UI model",
        "base_url": format!("{}/step_plan/v1", upstream.uri()),
        "auth_scheme": "bearer", "credentials": {"api_keys": ["test-only"]},
        "enabled": true, "initial_model": {"model": "step-3.7-flash", "enabled": true,
            "capabilities": [{"task": "chat", "traits": ["function_calling", "streaming"],
                "protocol": "openai.chat_text", "connection_role": "default", "provider_params": {}}]}
    })).await;
    let preset = post(
        &router,
        "/api/agent-presets/from-template/chat.minimal",
        json!({
            "display_name": "Plugin UI test", "reuse_existing": true,
            "model": {"provider_id": provider["provider_id"], "model": "step-3.7-flash"}
        }),
    )
    .await;
    let session = post(
        &router,
        "/api/agent-sessions",
        json!({
            "preset_id": preset["preset"]["preset_id"], "title": "Plugin UI session"
        }),
    )
    .await;
    let session_id = session["agent_session_id"].as_str().unwrap();
    let plugin = install_ui(&router).await;
    let plugin_id = plugin["plugin_id"].as_str().unwrap();
    assert_eq!(get(&router, "/api/agent-catalog/ui/agent-session").await, json!([]), "HTML alone must not opt into Agent presentation");
    let plugin = publish_agent_view(&router, plugin_id).await;
    let views = get(&router, "/api/agent-catalog/ui/agent-session").await;
    assert_eq!(views.as_array().unwrap().len(), 1);
    assert_eq!(views[0]["plugin_id"], plugin_id);
    assert_eq!(views[0]["expected_release_digest"], plugin["releases"]["active"]["release_digest"]);
    let capability = views[0]["capability"].clone();
    let agent_catalog = get(&router, "/api/agent-catalog").await;
    assert!(!agent_catalog["capabilities"].as_array().unwrap().iter().any(|item| item["capability"] == capability), "a UI contribution must never become an Agent Tool");
    let open_path = format!("/api/plugins/runtimes/{plugin_id}/surface/open");
    let bridge_path = format!("/api/plugins/runtimes/{plugin_id}/surface/bridge");
    let grant = json!({"plugin_id": plugin_id, "agent_session": {
        "agent_session_id": session_id,
        "expected_release_digest": plugin["releases"]["active"]["release_digest"],
        "ui_capability": capability
    }});
    let observe = json!({"operation": "observe", "after_seq": 0, "limit": 100});

    let standalone = post(&router, &open_path, json!({"plugin_id": plugin_id})).await;
    let (status, _) = request(
        &router,
        "POST",
        &bridge_path,
        bridge_body(&standalone, observe.clone()),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "standalone view must not acquire Session authority"
    );
    let surface = post(&router, &open_path, grant.clone()).await;
    for invalid in [json!({"id": "not-a-ui-capability", "version": "1.0.0"}), json!({"id": capability["id"], "version": "99.0.0"})] {
        let mut wrong = grant.clone();
        wrong["agent_session"]["ui_capability"] = invalid;
        assert_eq!(request(&router, "POST", &open_path, wrong).await.0, StatusCode::BAD_REQUEST);
    }
    let observed = post(
        &router,
        &bridge_path,
        bridge_body(&surface, observe.clone()),
    )
    .await;
    assert_eq!(observed["session"]["agent_session_id"], session_id);
    assert_eq!(observed["messages"], json!([]));
    assert!(
        upstream.received_requests().await.unwrap().is_empty(),
        "opening/observing must not start a turn"
    );

    // Being an owned Conversation is insufficient: it must carry the canonical
    // AgentSession metadata produced by the real Session application service.
    let plain_id = uuid::Uuid::now_v7().to_string();
    let owner_id = nomifun_db::installation_owner_id(services.database.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO conversations (conversation_id, user_id, name, type, created_at, updated_at) VALUES (?, ?, 'Not an AgentSession', 'nomi', 1, 1)")
        .bind(&plain_id).bind(&owner_id).execute(services.database.pool()).await.unwrap();
    let mut plain_grant = grant.clone();
    plain_grant["agent_session"]["agent_session_id"] = json!(plain_id);
    let (status, plain_error) = request(&router, "POST", &open_path, plain_grant).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(plain_error["code"], "NOMI_CORE_AGENT_SESSION_NOT_FOUND");

    let mut stale = grant.clone();
    stale["agent_session"]["expected_release_digest"] = json!("0".repeat(64));
    assert_eq!(
        request(&router, "POST", &open_path, stale).await.0,
        StatusCode::BAD_REQUEST
    );
    // Failed consent must leave the previous valid view intact.
    post(
        &router,
        &bridge_path,
        bridge_body(&surface, observe.clone()),
    )
    .await;
    let mut missing = grant.clone();
    missing["agent_session"]["agent_session_id"] = json!(uuid::Uuid::now_v7().to_string());
    assert_eq!(
        request(&router, "POST", &open_path, missing).await.0,
        StatusCode::NOT_FOUND
    );
    let mut spoofed = observe.clone();
    spoofed["agent_session_id"] = json!(uuid::Uuid::now_v7().to_string());
    assert_eq!(
        request(
            &router,
            "POST",
            &bridge_path,
            bridge_body(&surface, spoofed)
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (status, invalid) = request(
        &router,
        "POST",
        &bridge_path,
        bridge_body(
            &surface,
            json!({"operation": "turn", "input": {}, "idempotency_key": "invalid"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        invalid["code"], "NOMI_CORE_INVALID_REQUEST",
        "Session error codes must survive the bridge"
    );

    let turn = json!({"operation": "turn", "input": {"content": "hello from plugin UI"}, "idempotency_key": "same-intent"});
    let sent = post(&router, &bridge_path, bridge_body(&surface, turn.clone())).await;
    assert_eq!(sent["agent_session_id"], session_id);
    // Close the view immediately after admission; it neither owns nor cancels the turn.
    post(
        &router,
        &format!("/api/plugins/runtimes/{plugin_id}/surface/close"),
        json!({
            "plugin_id": plugin_id, "surface_session_id": surface["surface_session_id"],
            "surface_capability": surface["surface_capability"]
        }),
    )
    .await;
    assert!(
        !request(
            &router,
            "POST",
            &bridge_path,
            bridge_body(&surface, observe.clone())
        )
        .await
        .0
        .is_success()
    );
    let reopened = post(&router, &open_path, grant.clone()).await;
    let history = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let page = post(
                &router,
                &bridge_path,
                bridge_body(&reopened, observe.clone()),
            )
            .await;
            if page["messages"].to_string().contains("PLUGIN_UI_REPLY") {
                break page;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("real Nomi reply must become visible through plugin history");
    assert_eq!(
        history["events"],
        json!([]),
        "message history must not pretend to replay token events"
    );
    assert!(history["messages"].as_array().unwrap().iter().any(|message|
        message["message_type"] == "text" && message["projection"]["content"] == "PLUGIN_UI_REPLY"));
    let replay = post(&router, &bridge_path, bridge_body(&reopened, turn)).await;
    assert_eq!(replay["operation_id"], sent["operation_id"]);
    assert_eq!(upstream.received_requests().await.unwrap().len(), 1);
    let (status, public_history) = request(
        &router,
        "GET",
        &format!("/api/agent-sessions/{session_id}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{public_history}");
    assert_eq!(
        public_history["data"]["messages"], history["messages"],
        "plugin and built-in read the same projection"
    );
    // Exercise cancellation of a real in-flight Nomi model request as well as
    // observation/replay; a test-only Session port cannot establish this.
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/step_plan/v1/chat/completions"))
        .and(latest_user_input("cancel this request"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                // Longer than the cancellation assertion deadline: natural
                // completion cannot make a broken cancellation path pass.
                .set_delay(std::time::Duration::from_secs(30))
                .set_body_string("data: [DONE]\n\n"),
        )
        .expect(1)
        .mount(&upstream)
        .await;
    post(&router, &bridge_path, bridge_body(&reopened, json!({
        "operation": "turn", "input": {"content": "cancel this request"}, "idempotency_key": "cancel-intent"
    }))).await;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while upstream.received_requests().await.unwrap().len() < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("second turn must actually reach the model before cancellation");
    let active = post(&router, &bridge_path, bridge_body(&reopened, observe.clone())).await;
    assert_eq!(active["head"]["status"], "running", "cancel must act on an active turn");
    post(
        &router,
        &bridge_path,
        bridge_body(&reopened, json!({"operation": "cancel"})),
    )
    .await;

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let page = post(
                &router,
                &bridge_path,
                bridge_body(&reopened, observe.clone()),
            )
            .await;
            if page["head"]["status"] != "running" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("cancel must stop the Session's active turn");

    // Exercise the production event-bus observer and WS manager queue, not a
    // synthetic plugin event. This is not a browser/WebSocket handshake test.
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let connection = services.ws_manager.add_client(owner_id.clone(), TRUST.into(), tx);
    post(&router, &bridge_path, bridge_body(&reopened, json!({
        "operation": "turn", "input": {"content": "hello from plugin UI"}, "idempotency_key": "stream-intent"
    }))).await;
    let event = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while let Some(message) = rx.recv().await {
            let nomifun_realtime::WsOutbound::Text(text) = message else { continue; };
            let message: Value = serde_json::from_str(&text).unwrap();
            if message["name"] == "plugin.agent-session.stream" && message["data"]["event"].to_string().contains("PLUGIN_UI_REPLY") {
                return message["data"].clone();
            }
        }
        panic!("WS transport closed before plugin stream delivery");
    }).await.expect("real Nomi events must reach the scoped plugin WS projection");
    assert_eq!(event["plugin_id"], plugin_id);
    assert_eq!(event["surface_session_id"], reopened["surface_session_id"]);
    assert_eq!(event["surface_generation"], reopened["surface_generation"]);
    assert_eq!(event["event"]["conversation_id"], session_id);
    services.ws_manager.remove_client(connection);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let page = post(&router, &bridge_path, bridge_body(&reopened, observe.clone())).await;
            if page["head"]["status"] != "running" { break; }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }).await.expect("stream turn must complete before teardown");

    // An ordinary reopen clears, rather than inherits, the old Session grant.
    let normal = post(&router, &open_path, json!({"plugin_id": plugin_id})).await;
    assert_eq!(
        request(&router, "POST", &bridge_path, bridge_body(&normal, observe))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let scope: Option<String> = sqlx::query_scalar(
        "SELECT conversation_id FROM plugin_surface_sessions WHERE plugin_product_id = ?",
    )
    .bind(plugin_id)
    .fetch_one(services.database.pool())
    .await
    .unwrap();
    assert!(scope.is_none());
    assert!(services.plugin_runtime.project_agent_session_stream(&owner_id, session_id,
        &[json!({"conversation_id": session_id, "type": "text", "data": "after revoke"})])
        .await.unwrap().is_empty());
    let w = get(&router, &format!("/api/plugins/runtimes/{plugin_id}/workshop")).await;
    post(&router, &format!("/api/plugins/runtimes/{plugin_id}/enabled"), json!({
        "plugin_id": plugin_id, "expected_product_revision": w["plugin"]["product_revision"],
        "expected_pointer_revision": w["plugin"]["releases"]["pointer_revision"],
        "expected_active_release_digest": w["plugin"]["releases"]["active"]["release_digest"],
        "enabled": false
    })).await;
    assert_eq!(get(&router, "/api/agent-catalog/ui/agent-session").await, json!([]));
    assert!(!request(&router, "POST", &open_path, grant).await.0.is_success());

    // The product template is ordinary editable source, not a special runtime.
    // Creating it does not grant access, publish a view or invoke a model.
    let calls_before_template = upstream.received_requests().await.unwrap().len();
    let draft = post(&router, "/api/plugins/drafts/from-template/agent-session-view", json!({})).await;
    assert_eq!(draft["status"], "ready");
    assert!(draft["service_source"].is_null());
    assert!(draft["import"].is_null());
    assert!(draft["html"].as_str().unwrap().contains("Retry same request"));
    assert_eq!(get(&router, "/api/agent-catalog/ui/agent-session").await, json!([]));
    let reference = post(&router, &format!("/api/plugins/drafts/{}/save", draft["id"].as_str().unwrap()),
        json!({"expected_revision": draft["revision"]})).await;
    let views = get(&router, "/api/agent-catalog/ui/agent-session").await;
    assert_eq!(views.as_array().unwrap().len(), 1);
    let reference_id = reference["plugin"]["plugin_id"].as_str().unwrap();
    assert_eq!(views[0]["plugin_id"], reference_id);
    let reference_surface = post(&router, &format!("/api/plugins/runtimes/{reference_id}/surface/open"), json!({
        "plugin_id": reference_id, "agent_session": {"agent_session_id": session_id,
            "expected_release_digest": views[0]["expected_release_digest"], "ui_capability": views[0]["capability"]}
    })).await;
    let reference_history = post(&router, &format!("/api/plugins/runtimes/{reference_id}/surface/bridge"),
        bridge_body(&reference_surface, json!({"operation": "observe", "after_seq": 0, "limit": 50}))).await;
    assert_eq!(reference_history["session"]["agent_session_id"], session_id);
    assert!(reference_history["messages"].to_string().contains("PLUGIN_UI_REPLY"));
    assert_eq!(upstream.received_requests().await.unwrap().len(), calls_before_template);

    // Recovery data lives in existing plugin KV. Save the immutable intent
    // before a send, reopen, then explicitly retry with the same host key.
    let reference_bridge = format!("/api/plugins/runtimes/{reference_id}/surface/bridge");
    let key = format!("agent-session/composer/v1/{session_id}");
    let recovery = json!({"version": 1, "session_id": session_id, "text": "hello from plugin UI",
        "pending": {"key": "recovered-reference-intent", "input": {"content": "hello from plugin UI"}}});
    let empty = post(&router, &reference_bridge, storage_body(&reference_surface, json!({"operation": "get", "key": key}))).await;
    assert_eq!(empty["outcome"], "value");
    assert!(empty["value"].is_null() && empty["revision"].is_null());
    let written = post(&router, &reference_bridge, storage_body(&reference_surface,
        json!({"operation": "compare_and_swap", "key": key, "value": recovery}))).await;
    assert_eq!(written["applied"], true);
    assert_eq!(written["current_revision"], 1);
    let reference_turn = json!({"operation": "turn", "input": recovery["pending"]["input"],
        "idempotency_key": recovery["pending"]["key"]});
    let reference_sent = post(&router, &reference_bridge, bridge_body(&reference_surface, reference_turn.clone())).await;
    let reopened_reference = post(&router, &format!("/api/plugins/runtimes/{reference_id}/surface/open"), json!({
        "plugin_id": reference_id, "agent_session": {"agent_session_id": session_id,
            "expected_release_digest": views[0]["expected_release_digest"], "ui_capability": views[0]["capability"]}
    })).await;
    let restored = post(&router, &reference_bridge, storage_body(&reopened_reference,
        json!({"operation": "get", "key": key}))).await;
    assert_eq!(restored["value"], recovery);
    assert_eq!(restored["revision"], 1);
    let reference_retry = post(&router, &reference_bridge, bridge_body(&reopened_reference, reference_turn)).await;
    assert_eq!(reference_retry["operation_id"], reference_sent["operation_id"]);
    let stale_write = request(&router, "POST", &reference_bridge, storage_body(&reference_surface,
        json!({"operation": "compare_and_swap", "key": key, "expected_revision": 1, "value": {"stale": true}}))).await;
    assert!(!stale_write.0.is_success(), "old Surface cannot overwrite recovery after reopen");
    let conflict = post(&router, &reference_bridge, storage_body(&reopened_reference,
        json!({"operation": "compare_and_swap", "key": key, "expected_revision": 99, "value": {"stale": true}}))).await;
    assert_eq!(conflict["applied"], false);
    let unchanged = post(&router, &reference_bridge, storage_body(&reopened_reference,
        json!({"operation": "get", "key": key}))).await;
    assert_eq!(unchanged["value"], recovery);
    let cleared = post(&router, &reference_bridge, storage_body(&reopened_reference,
        json!({"operation": "compare_and_swap", "key": key, "expected_revision": 1}))).await;
    assert_eq!(cleared["applied"], true);
    assert_eq!(cleared["current_revision"], 2);
    let tombstone = post(&router, &reference_bridge, storage_body(&reopened_reference,
        json!({"operation": "get", "key": key}))).await;
    assert!(tombstone["value"].is_null());
    assert_eq!(tombstone["revision"], 2);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let page = post(&router, &reference_bridge, bridge_body(&reopened_reference,
                json!({"operation": "observe", "after_seq": 0, "limit": 50}))).await;
            if page["head"]["status"] != "running" { break; }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }).await.expect("reference turn must finish before teardown");
    assert_eq!(upstream.received_requests().await.unwrap().len(), calls_before_template + 1,
        "explicit retry after reopen must not execute another model request");

    // Persisted source kinds travel unchanged through the same public and
    // plugin observation, without guessing from content or executing tools.
    let mut fixtures = Vec::new();
    for (kind, content) in [
        ("tool_call", json!({"name": "Read", "status": "error", "error": "fixture failure"})),
        ("tool_group", json!([{"name": "Read", "status": "Success", "result_display": "fixture output"}])),
        ("plan", json!({"entries": [{"content": "fixture step", "status": "in_progress"}]})),
        ("thinking", json!({"content": "fixture thought", "status": "done"})),
        ("tips", json!({"content": "fixture warning", "type": "warning"})),
        ("agent_status", json!({"agent_name": "Nomi", "status": "connected"})),
        ("permission", json!({"content": "fixture permission is not an assistant reply"})),
    ] {
        let message_id = uuid::Uuid::now_v7().to_string();
        services.conversation_repo.insert_message(&nomifun_db::models::MessageRow {
            id: 0, message_id: message_id.clone(), conversation_id: session_id.to_owned(),
            msg_id: None, r#type: kind.to_owned(), content: content.to_string(),
            position: Some("left".to_owned()), status: Some("error".to_owned()),
            hidden: false, created_at: 1,
        }).await.unwrap();
        fixtures.push((message_id, kind, content));
    }
    let typed_history = post(&router, &reference_bridge, bridge_body(&reopened_reference,
        json!({"operation": "observe", "after_seq": 0, "limit": 50}))).await;
    let public = get(&router, &format!("/api/agent-sessions/{session_id}")).await;
    assert_eq!(typed_history["messages"], public["messages"]);
    for (id, kind, content) in fixtures {
        let message = typed_history["messages"].as_array().unwrap().iter()
            .find(|message| message["projection_id"] == id).expect("source row is visible");
        assert_eq!(message["message_type"], kind);
        assert_eq!(message["message_status"], "error");
        assert_eq!(message["presentation_intent"], "left");
        assert_eq!(message["projection"], content, "metadata must not rewrite the body");
        let digest = nomifun_agent_contracts::digest_payload(&content).unwrap();
        assert_eq!(message["semantic_digest"], digest.as_ref());
    }
    assert_eq!(upstream.received_requests().await.unwrap().len(), calls_before_template + 1);
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
}
