//! Public progress is recoverable from the journal across paging and restart.
use axum::{Router, body::Body, http::{Request, StatusCode}};
use nomifun_agent_contracts::{
    AgentSessionId, EventId, OperationId, SessionEventAppend, SemanticSessionEventDraft,
    SessionEventKind, SessionEventPayloadRef, StrictJsonValue,
};
use nomifun_agent_session::AgentSessionStore;
use nomifun_app::{AppConfig, compatibility::{AppServices, create_router}};
use nomifun_auth::AuthPolicy;
use serde_json::{Value, json};
use tower::ServiceExt;

const TRUST: &str = "task-plan-fixture";

async fn call(router: &Router, method: &str, path: &str, body: Value) -> Value {
    let response = router.clone().oneshot(Request::builder().method(method).uri(path)
        .header("x-nomi-local-trust", TRUST).header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(status.is_success(), "{status}: {value}");
    value["data"].clone()
}

fn record(session: &AgentSessionId, started: &EventId, operation: &str, key: &str, event: Value) -> SessionEventAppend {
    SessionEventAppend {
        agent_session_id: session.clone(), event_id: key.into(), producer_id: "runtime_supervisor".into(),
        idempotency_key: key.into(), runtime_binding_id: None, runtime_producer_seq: None,
        semantic_event: SemanticSessionEventDraft {
            kind: SessionEventKind("runtime/progress-recorded".into()), kind_version: 1, correlation_id: operation.into(),
            causation_event_id: Some(started.clone()), payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(
                json!({"runtime_binding_id":"fixture","producer_seq":1,"event":event}))),
        },
    }
}

#[tokio::test]
async fn task_plan_http_survives_long_history_and_restart_then_resets_for_the_next_turn() {
    let root = tempfile::tempdir().unwrap();
    let config = AppConfig {
        data_dir: root.path().join("data"), work_dir: root.path().join("work"),
        auth_policy: AuthPolicy::TrustLocalToken, local_trust_secret: Some(TRUST.into()), ..Default::default()
    };
    std::fs::create_dir_all(&config.data_dir).unwrap();
    let database = nomifun_db::init_database(&config.database_path()).await.unwrap();
    let first = AppServices::from_config(database, &config).await.unwrap();
    let router = create_router(&first).await;
    let provider = call(&router, "POST", "/api/providers", json!({
        "platform":"openai","name":"Task plan","base_url":"https://example.invalid/v1",
        "auth_scheme":"bearer","credentials":{"api_keys":["fixture"]},"enabled":true,
        "initial_model":{"model":"fixture","enabled":true,"capabilities":[{
            "task":"chat","traits":[],"protocol":"openai.chat_text","connection_role":"default","provider_params":{}
        }]}
    })).await;
    let model = json!({"provider_id":provider["provider_id"],"model":"fixture"});
    let preset = call(&router, "POST", "/api/agent-presets/from-template/chat.minimal", json!({
        "display_name":"Task plan","reuse_existing":true,"model_route_refs":{},"chat_route_records":{},"model":model
    })).await;
    let session = call(&router, "POST", "/api/agent-sessions", json!({
        "preset_id":preset["preset"]["preset_id"],"model":model,"title":"Task plan"
    })).await;
    let id = session["agent_session_id"].as_str().unwrap();
    let sid = AgentSessionId::from(id);
    let path = format!("/api/agent-sessions/{id}/task-plan");
    assert!(call(&router, "GET", &path, json!({})).await["plan"].is_null());
    let unauthorized = router.clone().oneshot(Request::builder().uri(&path)
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(unauthorized.status(), StatusCode::FORBIDDEN);

    let store = AgentSessionStore::from_pool(first.database.pool().clone()).await.unwrap();
    let operation = OperationId::from("task-plan-first");
    let started = store.start_turn(&sid, "session_api".into(), "task-plan-first".into(), operation.clone(),
        StrictJsonValue(json!({"content":"Inspect and verify"}))).await.unwrap().1.ack.unwrap().event_id;
    let plan = json!({"event":"plan_updated","plan":{
        "revision":2,"explanation":"Waiting for access","needs_replan":true,
        "steps":[{"step":"Inspect","status":"completed"},{"step":"Verify","status":"blocked"}],
        "requirements":[{"id":"req","description":"private-ledger-marker","source":{"input":0,"quote":"Inspect"}}]
    }});
    store.append_event(&record(&sid, &started, operation.as_ref(), "task-plan-update", plan)).await.unwrap();
    let initial = call(&router, "GET", &path, json!({})).await;
    for index in 0..80 {
        let message_id = uuid::Uuid::now_v7().to_string();
        store.append_event(&SessionEventAppend {
            agent_session_id: sid.clone(), event_id: format!("later-{index}").into(), producer_id:"runtime_supervisor".into(),
            idempotency_key: format!("later-{index}").into(), runtime_binding_id:None, runtime_producer_seq:None,
            semantic_event: SemanticSessionEventDraft {
                kind:SessionEventKind("message/content-part".into()),kind_version:1,correlation_id:message_id.into(),
                causation_event_id:Some(started.clone()),payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "content":format!("Later progress {index}"),"turn_id":initial["turn_id"]
                }))),
            },
        }).await.unwrap();
    }
    let history = call(&router, "GET", &format!("/api/agent-sessions/{id}/message-history?cursor=&page_size=1"), json!({})).await;
    assert!(history["total"].as_u64().unwrap() >= 80);
    assert_eq!(history["items"].as_array().unwrap().len(), 1);
    let snapshot = call(&router, "GET", &path, json!({})).await;
    assert_eq!(snapshot["plan"]["steps"][1]["status"], "blocked");
    assert_eq!(snapshot["turn_status"], "running");
    assert_eq!(snapshot["conversation_id"], id);
    assert!(uuid::Uuid::parse_str(snapshot["turn_id"].as_str().unwrap()).is_ok());
    assert!(!snapshot.to_string().contains("private-ledger-marker"));
    let terminal = SessionEventAppend {
        agent_session_id: sid.clone(), event_id:"task-plan-finish".into(), producer_id:"runtime_supervisor".into(),
        idempotency_key:"task-plan-finish".into(),runtime_binding_id:None,runtime_producer_seq:None,
        semantic_event: SemanticSessionEventDraft {
            kind:SessionEventKind("turn/completed".into()),kind_version:1,correlation_id:operation.as_ref().into(),causation_event_id:Some(started),
            payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"result":{"ok":true}}))),
        },
    };
    store.append_turn_terminal(&terminal, &operation).await.unwrap();
    let finished = call(&router, "GET", &path, json!({})).await;
    assert_eq!(finished["turn_status"], "completed");
    assert_eq!(finished["plan"], snapshot["plan"]);
    drop(router);
    first.shutdown_browser_platform().await.unwrap();
    first.database.close().await;
    drop(first);

    let reopened = nomifun_db::init_database(&config.database_path()).await.unwrap();
    let second = AppServices::from_config(reopened, &config).await.unwrap();
    let restored = create_router(&second).await;
    assert_eq!(call(&restored, "GET", &path, json!({})).await["plan"], snapshot["plan"]);
    let store = AgentSessionStore::from_pool(second.database.pool().clone()).await.unwrap();
    store.start_turn(&sid, "session_api".into(), "task-plan-second".into(), "task-plan-second".into(),
        StrictJsonValue(json!({"content":"A different task"}))).await.unwrap();
    let reset = call(&restored, "GET", &path, json!({})).await;
    assert!(reset["plan"].is_null());
    assert_ne!(reset["turn_id"], snapshot["turn_id"]);
    second.shutdown_browser_platform().await.unwrap();
    second.database.close().await;
}
