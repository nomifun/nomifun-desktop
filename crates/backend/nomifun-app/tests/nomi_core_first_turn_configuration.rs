//! First-Turn regression for the production save -> warmup -> send path.
//! Uses only current-generation Session facts and a controlled local provider.

mod common;

use std::future::IntoFuture;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::{Json, Router, routing::post};
use nomifun_agent_contracts::{AgentSessionId, OperationId};
use nomifun_agent_session::{AgentSessionStore, TurnReceiptStatus};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tower::ServiceExt;

const TRUST: &str = "first-turn-configuration";

#[derive(Clone)]
struct ControlledProvider {
    requests: Arc<Mutex<Vec<Value>>>,
    first_request_release: Arc<Semaphore>,
}

async fn provider_reply(
    State(provider): State<ControlledProvider>,
    Json(request): Json<Value>,
) -> ([(&'static str, &'static str); 1], String) {
    let first = {
        let mut requests = provider.requests.lock().unwrap();
        requests.push(request);
        requests.len() == 1
    };
    if first {
        provider.first_request_release.acquire().await.unwrap().forget();
    }
    let reply = if first { "D2_FIRST_REPLY" } else { "D2_NEXT_REPLY" };
    (
        [("content-type", "text/event-stream")],
        format!("data: {{\"id\":\"d2\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{reply}\"}},\"finish_reason\":null}}]}}\n\ndata: {{\"id\":\"d2\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"),
    )
}

async fn call(router: &Router, method: &str, path: &str, body: Value) -> Value {
    let response = router.clone().oneshot(
        Request::builder().method(method).uri(path)
            .header("x-nomi-local-trust", TRUST)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())).unwrap(),
    ).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
    let response: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(matches!(status, StatusCode::OK | StatusCode::CREATED), "{path}: {status} {response}");
    response
}

fn capability(context: u64, output: u64, effort: &str, threshold: u64) -> Value {
    json!({
        "task": "chat", "traits": [], "protocol": "openai.chat_text",
        "connection_role": "default", "context_limit": context,
        "output_limit": output, "compaction_threshold_pct": threshold,
        "provider_params": {"reasoning_effort": effort}
    })
}

async fn save_configuration(router: &Router, provider_id: &str, capability: Value) {
    call(router, "PUT", "/api/provider-models", json!({
        "provider_id": provider_id,
        "model": {"model": "first-turn-model", "enabled": true, "capabilities": [capability]}
    })).await;
}

async fn wait_completed(store: &AgentSessionStore, session: &AgentSessionId, operation: &OperationId) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let receipt = store.read_turn_receipt(session, operation).await.unwrap();
            if receipt.status == TurnReceiptStatus::Completed { return; }
            assert_eq!(receipt.status, TurnReceiptStatus::Running, "Turn must complete normally: {receipt:?}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("native preparation and inference must reach a durable terminal");
}

#[tokio::test]
async fn first_turn_adopts_saved_configuration_after_warmup_and_freezes_it_until_terminal() {
    let provider = ControlledProvider {
        requests: Arc::new(Mutex::new(Vec::new())),
        first_request_release: Arc::new(Semaphore::new(0)),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(axum::serve(
        listener,
        Router::new().route("/v1/chat/completions", post(provider_reply)).with_state(provider.clone()),
    ).into_future());
    let (router, services) = common::build_local_trust_app(TRUST).await;
    let created = call(&router, "POST", "/api/providers", json!({
        "platform": "stepfun-plan", "name": "First Turn controlled provider",
        "base_url": format!("http://{address}/v1"), "auth_scheme": "bearer",
        "credentials": {"api_keys": ["fixture-only"]}, "enabled": true,
        "initial_model": {
            "model": "first-turn-model", "enabled": true,
            "capabilities": [capability(131_072, 2048, "low", 75)]
        }
    })).await;
    let provider_id = created["data"]["provider_id"].as_str().unwrap();
    let selection = json!({"provider_id": provider_id, "model": "first-turn-model"});
    let preset = call(&router, "POST", "/api/agent-presets/from-template/chat.minimal", json!({
        "display_name": "First Turn minimal Agent", "reuse_existing": false,
        "model_route_refs": {}, "chat_route_records": {}, "model": selection
    })).await;
    let created = call(&router, "POST", "/api/agent-sessions", json!({
        "preset_id": preset["data"]["preset"]["preset_id"],
        "model": selection, "title": "First Turn configuration"
    })).await;
    let session_id = created["data"]["agent_session_id"].as_str().unwrap();
    let session = AgentSessionId::from(session_id.to_owned());
    let store = AgentSessionStore::from_pool(services.database.pool().clone()).await.unwrap();
    let opened = store.get_live_session(&session).await.unwrap().agent_binding;

    // The provider revision changes while the canonical Session is still idle.
    // Warmup deliberately opens that idle Snapshot; it must own no Turn/claim.
    save_configuration(&router, provider_id, capability(65_536, 2048, "low", 75)).await;
    call(&router, "POST", &format!("/api/agent-sessions/{session_id}/warmup"), json!({})).await;
    assert_eq!(store.get_live_session(&session).await.unwrap().agent_binding, opened);
    assert!(store.head(&session).await.unwrap().active_turn_id.is_none());
    assert!(provider.requests.lock().unwrap().is_empty());

    let key = uuid::Uuid::now_v7().to_string();
    let turn_input = json!({"idempotency_key": key, "input": {"content": "D2 first turn: reply once without tools"}});
    let first = call(&router, "POST", &format!("/api/agent-sessions/{session_id}/turns"), turn_input.clone()).await;
    let operation = OperationId::from(first["data"]["operation_id"].as_str().unwrap().to_owned());
    tokio::time::timeout(Duration::from_secs(15), async {
        while provider.requests.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("the first admitted Turn must progress through native claim to a model request");
    let adopted = store.get_live_session(&session).await.unwrap().agent_binding;
    assert_ne!(adopted.resolved_snapshot_ref, opened.resolved_snapshot_ref);
    assert_eq!(adopted.binding_version, opened.binding_version + 1);
    assert_eq!(adopted.typed_resource_bindings, opened.typed_resource_bindings);
    let facts = store.chat_causality_facts(&session, &operation).await.unwrap();
    assert!(facts.execution_generation > 0, "HTTP admission must lead to a real native execution claim");
    assert!(facts.event_payloads.values().any(|payload| {
        let event = &payload["event"];
        event["event"] == "execution_budget_prepared"
            && event["context_window_tokens"] == 65_536 && event["max_output_tokens"] == 2048
    }), "the first Turn budget must use the saved pre-send configuration");
    assert!(facts.event_payloads.values().any(|payload| {
        payload["event"]["event"] == "turn_started"
            && payload["event"]["binding"]["resolved_snapshot_ref"] == serde_json::to_value(&adopted.resolved_snapshot_ref).unwrap()
    }), "the engine must open the same refreshed Snapshot as the accepted Turn");

    // Hold the actual first response while saving a successor configuration.
    // The first Turn's captured request and budget stay frozen.
    save_configuration(&router, provider_id, capability(1_000_000, 4096, "high", 80)).await;
    assert_eq!(store.read_turn_receipt(&session, &operation).await.unwrap().status, TurnReceiptStatus::Running);
    {
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["reasoning_effort"], "low");
        assert_eq!(requests[0]["max_tokens"], 2048);
    }
    provider.first_request_release.add_permits(1);
    wait_completed(&store, &session, &operation).await;
    let replay = call(&router, "POST", &format!("/api/agent-sessions/{session_id}/turns"), turn_input).await;
    assert_eq!(replay["data"]["operation_id"], first["data"]["operation_id"]);
    assert_eq!(provider.requests.lock().unwrap().len(), 1);

    call(&router, "POST", &format!("/api/agent-sessions/{session_id}/warmup"), json!({})).await;
    let next = call(&router, "POST", &format!("/api/agent-sessions/{session_id}/turns"), json!({
        "idempotency_key": uuid::Uuid::now_v7().to_string(),
        "input": {"content": "D2 next turn: reply once without tools"}
    })).await;
    let next_operation = OperationId::from(next["data"]["operation_id"].as_str().unwrap().to_owned());
    wait_completed(&store, &session, &next_operation).await;
    let next_facts = store.chat_causality_facts(&session, &next_operation).await.unwrap();
    assert!(next_facts.event_payloads.values().any(|payload| {
        let event = &payload["event"];
        event["event"] == "execution_budget_prepared"
            && event["context_window_tokens"] == 1_000_000 && event["max_output_tokens"] == 4096
    }), "the successor Turn must adopt the newly saved budget");
    {
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1]["reasoning_effort"], "high");
        assert_eq!(requests[1]["max_tokens"], 4096);
    }
    let history = call(&router, "GET", &format!("/api/agent-sessions/{session_id}/message-history?page_size=50"), json!({})).await;
    for reply in ["D2_FIRST_REPLY", "D2_NEXT_REPLY"] {
        assert!(history["data"]["items"].as_array().unwrap().iter().any(|item| item["content"]["content"] == reply), "{history}");
    }
    services.shutdown_browser_platform().await.unwrap();
    services.database.close().await;
    server.abort();
}
