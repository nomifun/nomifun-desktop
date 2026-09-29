//! W155 — canonical settlement-error delivery over the production Realtime
//! transport.
//!
//! A turn that ends in a settlement error emits `turn.started`, `message.stream`
//! tool lifecycle frames, and `turn.completed` over the real WebSocket bridge.
//! The event bus is volatile: a consumer that disconnects mid-turn must recover
//! the missed settlement through the durable canonical cursors
//! (`/events?after_seq`, `/messages?after_seq`, `/message-history?cursor`) and
//! must never see a duplicated or second tool row after reconnect.
//!
//! The runtime handle is the existing `test-support` mock escape hatch so the
//! test reaches a real canonical turn admission, the real stream relay, the
//! real user-scoped broadcast bridge, and a real socket — no HTTP-layer or
//! bus-layer test doubles.

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::StreamExt;
use nomifun_ai_agent::{
    AgentRuntimeControl, AgentRuntimeHandle, AgentStreamEvent, InMemoryAgentRuntimeSessions,
    MockAgentRuntime,
    protocol::events::{ToolCallEventData, ToolCallStatus},
};
use nomifun_app::{AppConfig, compatibility::{AppServices, create_router}};
use nomifun_auth::AuthPolicy;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::{Notify, broadcast};
use tokio_tungstenite::tungstenite;
use tower::ServiceExt;

const TRUST: &str = "realtime-settlement-w155";
/// The exact recovery contract the settlement row must keep on every surface.
const SETTLEMENT_ERROR: &str = "Capability Kernel rejected Agent Runtime Tool \
    (CAPABILITY_UNAVAILABLE): The workspace owner reported success. Do not \
    retry automatically. Re-read the affected owner state.";
const TURN_ERROR: &str = "tool settlement could not be persisted";
const INTRUDER_USER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000099";

/// Shared control for the mock runtime: the test arms `mid_turn_gate_armed`
/// before dispatching a turn; that turn's `send_message` blocks on
/// `mid_turn_gate` after the `tool_call` running frame so the test can drop
/// the consumer socket while the settlement is still unsettled.
struct SettlementGate {
    notify: Notify,
    armed: AtomicBool,
}

impl SettlementGate {
    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }

    fn open(&self) {
        self.notify.notify_waiters();
    }
}

/// Mock Runtime that settles every accepted turn with the canonical
/// settlement-error sequence a real engine's ToolHost/Journal produces:
/// `tool/call-started` -> `tool/result-recorded` (error) -> `turn/failed`,
/// with the matching live `ToolCall`/`Error` stream frames.
struct SettlementErrorMockAgent {
    conversation_id: String,
    workspace: String,
    store: nomifun_agent_session::AgentSessionStore,
    events: broadcast::Sender<AgentStreamEvent>,
    turn_index: Arc<AtomicUsize>,
    gate: Arc<SettlementGate>,
}

#[async_trait::async_trait]
impl AgentRuntimeControl for SettlementErrorMockAgent {
    fn agent_type(&self) -> nomifun_common::AgentType {
        nomifun_common::AgentType::Nomi
    }
    fn conversation_id(&self) -> &str {
        &self.conversation_id
    }
    fn workspace(&self) -> &str {
        &self.workspace
    }
    fn status(&self) -> Option<nomifun_common::ConversationStatus> {
        None
    }
    fn is_transport_healthy(&self) -> bool {
        true
    }
    fn last_activity_at(&self) -> nomifun_common::TimestampMs {
        nomifun_common::now_ms()
    }
    fn subscribe(&self) -> broadcast::Receiver<AgentStreamEvent> {
        self.events.subscribe()
    }

    async fn send_message(
        &self,
        _data: nomifun_ai_agent::types::SendMessageData,
    ) -> Result<(), nomifun_ai_agent::AgentSendError> {
        let session_id = nomifun_agent_contracts::AgentSessionId::from(self.conversation_id.clone());
        let turn_no = self.turn_index.fetch_add(1, Ordering::SeqCst) + 1;
        let head = self
            .store
            .head(&session_id)
            .await
            .expect("canonical head must exist for an accepted turn");
        let operation_id = head
            .active_turn_id
            .expect("turn dispatch must publish an active canonical turn");
        let receipt = self
            .store
            .read_turn_receipt(
                &session_id,
                &nomifun_agent_contracts::OperationId::from(operation_id.clone()),
            )
            .await
            .expect("turn receipt must be readable");
        let started_event_id = receipt
            .started_event
            .expect("accepted turn must have a start fact")
            .event_id;

        // The ToolHost contract: the call-start fact lands before the live
        // running frame so a live consumer can always reconcile to the store.
        let call_id = format!("w155-call-{turn_no}");
        let projection_id = uuid::Uuid::now_v7().to_string();
        let call_started = nomifun_agent_contracts::SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: nomifun_agent_contracts::EventId::from(format!("w155-call-{turn_no}")),
            producer_id: nomifun_agent_contracts::EventProducerId::from("runtime_supervisor"),
            idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(format!(
                "w155-call-{turn_no}"
            )),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: nomifun_agent_contracts::SessionEventKind("tool/call-started".to_owned()),
                kind_version: 1,
                correlation_id: nomifun_agent_contracts::CorrelationId::from(
                    projection_id.clone(),
                ),
                causation_event_id: Some(started_event_id),
                payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                    nomifun_agent_contracts::StrictJsonValue(json!({
                        "operation_id": format!("w155-op-{turn_no}"),
                        "call_id": call_id,
                        "capability_id": "workspace.files",
                        "action_id": "workspace.files/write",
                        "name": "write_file"
                    })),
                ),
            },
        };
        let call_ack = self
            .store
            .append_event(&call_started)
            .await
            .expect("tool/call-started must append")
            .ack
            .expect("new tool call must be accepted");
        let _ = self.events.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: call_id.clone(),
            name: "write_file".to_owned(),
            args: json!({"path": "settlement.txt", "content": "x"}),
            status: ToolCallStatus::Running,
            input: None,
            output: None,
            description: None,
            retry: None,
            artifacts: Vec::new(),
        }));

        // Hold the settlement mid-turn when the test armed the disconnect gate.
        if self.gate.armed.swap(false, Ordering::SeqCst) {
            self.gate.notify.notified().await;
        }

        let result_recorded = nomifun_agent_contracts::SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: nomifun_agent_contracts::EventId::from(format!("w155-result-{turn_no}")),
            producer_id: nomifun_agent_contracts::EventProducerId::from("runtime_supervisor"),
            idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(format!(
                "w155-result-{turn_no}"
            )),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: nomifun_agent_contracts::SessionEventKind(
                    "tool/result-recorded".to_owned(),
                ),
                kind_version: 1,
                correlation_id: nomifun_agent_contracts::CorrelationId::from(projection_id),
                causation_event_id: Some(call_ack.event_id),
                payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                    nomifun_agent_contracts::StrictJsonValue(json!({
                        "operation_id": format!("w155-op-{turn_no}"),
                        "call_id": call_id,
                        "output": null,
                        "error": SETTLEMENT_ERROR
                    })),
                ),
            },
        };
        let result_ack = self
            .store
            .append_event(&result_recorded)
            .await
            .expect("tool/result-recorded must append")
            .ack
            .expect("tool result must be accepted");
        let _ = self.events.send(AgentStreamEvent::ToolCall(ToolCallEventData {
            call_id: call_id.clone(),
            name: "write_file".to_owned(),
            args: json!({"path": "settlement.txt", "content": "x"}),
            status: ToolCallStatus::Error,
            input: None,
            output: Some(SETTLEMENT_ERROR.to_owned()),
            description: None,
            retry: None,
            artifacts: Vec::new(),
        }));

        // The canonical terminal must be durable BEFORE the relay may treat the
        // Error frame as the turn terminal (`canonical_stream_terminal_agrees`
        // reads the committed state).
        let turn_terminal = nomifun_agent_contracts::SessionEventAppend {
            agent_session_id: session_id.clone(),
            event_id: nomifun_agent_contracts::EventId::from(format!("w155-failed-{turn_no}")),
            producer_id: nomifun_agent_contracts::EventProducerId::from("runtime_supervisor"),
            idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(format!(
                "w155-failed-{turn_no}"
            )),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: nomifun_agent_contracts::SessionEventKind("turn/failed".to_owned()),
                kind_version: 1,
                correlation_id: nomifun_agent_contracts::CorrelationId::from(
                    operation_id.clone(),
                ),
                causation_event_id: Some(result_ack.event_id),
                payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                    nomifun_agent_contracts::StrictJsonValue(json!({
                        "model_steps": 0,
                        "message": TURN_ERROR,
                        "error": {"code": "CAPABILITY_UNAVAILABLE", "retryable": false},
                        "finished_at_ms": 0
                    })),
                ),
            },
        };
        self.store
            .append_turn_terminal(
                &turn_terminal,
                &nomifun_agent_contracts::OperationId::from(operation_id),
            )
            .await
            .expect("turn/failed terminal must append");
        let _ = self.events.send(AgentStreamEvent::Error(
            nomifun_api_types::AgentStreamErrorData::legacy(TURN_ERROR, None),
        ));
        Ok(())
    }

    async fn cancel(&self) -> Result<(), nomifun_common::AppError> {
        Ok(())
    }
    fn kill(
        &self,
        _reason: Option<nomifun_common::AgentKillReason>,
    ) -> Result<(), nomifun_common::AppError> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl MockAgentRuntime for SettlementErrorMockAgent {}

struct RealtimeApp {
    addr: SocketAddr,
    router: axum::Router,
    services: AppServices,
    gate: Arc<SettlementGate>,
}

async fn start_realtime_app() -> RealtimeApp {
    let root = tempfile::Builder::new()
        .prefix("nomifun-w155-realtime-")
        .tempdir()
        .unwrap()
        .keep();
    let config = AppConfig {
        data_dir: root.join("data"),
        work_dir: root.join("work"),
        auth_policy: AuthPolicy::TrustLocalToken,
        local_trust_secret: Some(TRUST.into()),
        ..AppConfig::default()
    };
    let db = nomifun_db::init_database_memory().await.unwrap();
    let mut services = AppServices::from_config(db, &config).await.unwrap();
    let store = nomifun_agent_session::AgentSessionStore::from_pool(
        services.database.pool().clone(),
    )
    .await
    .unwrap();
    let gate = Arc::new(SettlementGate {
        notify: Notify::new(),
        armed: AtomicBool::new(false),
    });
    let turn_index = Arc::new(AtomicUsize::new(0));
    let workspace = config.work_dir.to_string_lossy().into_owned();
    let factory_gate = Arc::clone(&gate);
    let factory_turns = Arc::clone(&turn_index);
    let factory: Arc<
        dyn Fn(
                nomifun_ai_agent::types::AgentRuntimeBuildOptions,
            ) -> futures_util::future::BoxFuture<
                'static,
                Result<AgentRuntimeHandle, nomifun_common::AppError>,
            > + Send
            + Sync,
    > = Arc::new(move |opts| {
        let store = store.clone();
        let gate = Arc::clone(&factory_gate);
        let turn_index = Arc::clone(&factory_turns);
        let workspace = workspace.clone();
        Box::pin(async move {
            let (events, _) = broadcast::channel(64);
            Ok(AgentRuntimeHandle::Mock(Arc::new(
                SettlementErrorMockAgent {
                    conversation_id: opts.conversation_id,
                    workspace,
                    store,
                    events,
                    turn_index,
                    gate,
                },
            )))
        })
    });
    let runtime_sessions: Arc<dyn nomifun_ai_agent::AgentRuntimeSessions> =
        Arc::new(InMemoryAgentRuntimeSessions::new(factory));
    services = services.with_agent_runtime_sessions(runtime_sessions);
    let router = create_router(&services).await;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_router = router.clone();
    tokio::spawn(async move {
        axum::serve(listener, serve_router).await.unwrap();
    });

    RealtimeApp {
        addr,
        router,
        services,
        gate,
    }
}

async fn call(
    router: axum::Router,
    method: &str,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = router
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
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| {
            Value::String(String::from_utf8_lossy(&bytes).into_owned())
        })
    };
    (status, value)
}

async fn create_chat_provider(router: &axum::Router) -> Value {
    let (status, value) = call(
        router.clone(),
        "POST",
        "/api/providers",
        json!({
            "platform": "openai",
            "name": "Realtime settlement",
            "base_url": "https://example.invalid/v1",
            "auth_scheme": "bearer",
            "credentials": { "api_keys": ["test-only"] },
            "enabled": true,
            "sort_order": 1,
            "initial_model": {
                "model": "step-3.7-flash",
                "enabled": true,
                "capabilities": [{
                    "task": "chat",
                    "traits": [],
                    "protocol": "openai.chat_text",
                    "connection_role": "default",
                    "provider_params": {}
                }]
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{value}");
    value["data"].clone()
}

async fn create_settlement_session(app: &RealtimeApp) -> String {
    let provider = create_chat_provider(&app.router).await;
    let model = json!({"provider_id": provider["provider_id"], "model": "step-3.7-flash"});
    let (status, preset) = call(
        app.router.clone(),
        "POST",
        "/api/agent-presets/from-template/chat.minimal",
        json!({
            "display_name": "Realtime settlement",
            "reuse_existing": true,
            "model_route_refs": {},
            "chat_route_records": {},
            "model": model
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preset}");
    let preset_id = preset["data"]["preset"]["preset_id"].as_str().unwrap();
    let (status, session) = call(
        app.router.clone(),
        "POST",
        "/api/agent-sessions",
        json!({"preset_id": preset_id, "model": model, "title": "Realtime settlement"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{session}");
    session["data"]["agent_session_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// The production desktop handshake: a known local-webview origin plus the
/// per-boot local-trust secret carried as the WebSocket subprotocol.
async fn connect_owner(
    addr: SocketAddr,
) -> futures_util::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
> {
    let url = format!("ws://{addr}/ws");
    let request = tungstenite::http::Request::builder()
        .uri(&url)
        .header("Host", addr.to_string())
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
        .header("Origin", "http://tauri.localhost")
        .header("Sec-WebSocket-Protocol", TRUST)
        .body(())
        .unwrap();
    let (ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    ws.split().1
}

/// A second authenticated principal: a JWT for a different user id takes the
/// non-browser Bearer path and must never see the owner's session events.
async fn connect_intruder(
    addr: SocketAddr,
    token: &str,
) -> futures_util::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
> {
    let url = format!("ws://{addr}/ws");
    let request = tungstenite::http::Request::builder()
        .uri(&url)
        .header("Host", addr.to_string())
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
        .header("Authorization", format!("Bearer {token}"))
        .body(())
        .unwrap();
    let (ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    ws.split().1
}

async fn read_text<S>(stream: &mut S) -> Value
where
    S: StreamExt<Item = Result<tungstenite::Message, tungstenite::Error>> + Unpin,
{
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match stream.next().await {
                Some(Ok(tungstenite::Message::Text(t))) => {
                    return serde_json::from_str::<Value>(&t).unwrap();
                }
                Some(Ok(tungstenite::Message::Close(frame))) => {
                    panic!("unexpected close frame: {frame:?}");
                }
                Some(Err(e)) => panic!("read error: {e}"),
                None => panic!("stream ended"),
                _ => continue,
            }
        }
    })
    .await
    .expect("read_text timed out")
}

/// Assert no text frame arrives within the window. A lagging resync notice or
/// an unexpected replay would both surface here as failures.
async fn expect_silent<S>(stream: &mut S, window: Duration, context: &str)
where
    S: StreamExt<Item = Result<tungstenite::Message, tungstenite::Error>> + Unpin,
{
    match tokio::time::timeout(window, async {
        loop {
            match stream.next().await {
                Some(Ok(tungstenite::Message::Text(t))) => return Some(t),
                Some(Ok(_)) => continue,
                Some(Err(e)) => panic!("{context}: read error: {e}"),
                None => panic!("{context}: stream ended"),
            }
        }
    })
    .await
    {
        Err(_) => {}
        Ok(Some(text)) => panic!("{context}: unexpected frame: {text}"),
        Ok(None) => unreachable!(),
    }
}

async fn wait_for_clients(app: &RealtimeApp, expected: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if app.services.ws_manager.client_count() == expected {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "expected {expected} registered WebSocket clients, got {}",
                app.services.ws_manager.client_count()
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Read `message.stream` frames until the predicate matches, returning the
/// matched frame. `turn.started` and other product frames that legally
/// interleave are collected for the caller's own assertions.
async fn read_stream_until(
    rx: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    expected_turn: &str,
    predicate: impl Fn(&Value) -> bool,
) -> (Vec<Value>, Value) {
    let mut others = Vec::new();
    let frame = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let frame = read_text(rx).await;
            if predicate(&frame) {
                return frame;
            }
            assert_eq!(
                frame["data"]["turn_id"].as_str().unwrap_or_default(),
                expected_turn,
                "stream interleaved a frame from an unexpected turn: {frame}"
            );
            others.push(frame);
        }
    })
    .await
    .expect("timed out waiting for the expected realtime frame");
    (others, frame)
}

#[tokio::test]
async fn settlement_error_streams_over_realtime_and_recovers_via_durable_cursor_after_reconnect() {
    let app = start_realtime_app().await;
    let session_id = create_settlement_session(&app).await;
    let intruder_token = app
        .services
        .jwt_service
        .sign(INTRUDER_USER_ID, "intruder")
        .unwrap();

    // Owner on the desktop-webview handshake; a different authenticated user on
    // the Bearer path must stay silent for the whole batch.
    let mut owner_rx = connect_owner(app.addr).await;
    let mut intruder_rx = connect_intruder(app.addr, &intruder_token).await;
    wait_for_clients(&app, 2).await;

    // --- Turn 1: consumer disconnects mid-turn, before the settlement lands ---
    app.gate.arm();
    let turn_router = app.router.clone();
    let turn_path = format!("/api/agent-sessions/{session_id}/turns");
    let turn_one = tokio::spawn(async move {
        call(
            turn_router,
            "POST",
            &turn_path,
            json!({"input": {"content": "settle with an error"}, "idempotency_key": "w155-turn-1"}),
        )
        .await
    });

    let started = read_text(&mut owner_rx).await;
    assert_eq!(started["name"], "turn.started");
    assert_eq!(started["data"]["conversation_id"], session_id);
    assert_eq!(started["data"]["status"], "running");
    let turn_one_root = started["data"]["turn_id"].as_str().unwrap().to_owned();

    let (_, running) = read_stream_until(&mut owner_rx, &turn_one_root, |frame| {
        frame["name"] == "message.stream"
            && frame["data"]["type"] == "tool_call"
            && frame["data"]["data"]["status"] == "running"
    })
    .await;
    assert_eq!(running["data"]["data"]["call_id"], "w155-call-1");
    assert_eq!(running["data"]["data"]["name"], "write_file");

    // The consumer drops before the settlement frames exist. The volatile bus
    // delivers them to nobody; durable truth alone must carry the settlement.
    drop(owner_rx);
    wait_for_clients(&app, 1).await;
    app.gate.open();
    let (status, delivery) = turn_one.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{delivery}");
    assert_eq!(delivery["data"]["status"], "running");
    assert_eq!(delivery["data"]["replayed"], false);

    // Settlement is committed even though no consumer was attached.
    let typed_session = nomifun_agent_contracts::AgentSessionId::from(session_id.clone());
    let store = nomifun_agent_session::AgentSessionStore::from_pool(
        app.services.database.pool().clone(),
    )
    .await
    .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let head = store.head(&typed_session).await.unwrap();
        if head.active_turn_id.is_none() {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "turn never settled");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // --- Reconnect: no replay on the volatile bus; rebuild via durable cursor ---
    let mut owner_rx = connect_owner(app.addr).await;
    wait_for_clients(&app, 2).await;
    expect_silent(
        &mut owner_rx,
        Duration::from_millis(400),
        "reconnected socket must not replay missed turn frames",
    )
    .await;

    // Durable canonical event replay: exactly one call/result/terminal for the
    // turn the consumer missed, cursor-contiguous and unique.
    let mut after_seq = 0_u64;
    let mut events = Vec::new();
    loop {
        let (status, page) = call(
            app.router.clone(),
            "GET",
            &format!("/api/agent-sessions/{session_id}/events?after_seq={after_seq}&limit=2"),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let batch = page["data"]["events"].as_array().unwrap();
        if batch.is_empty() {
            break;
        }
        for event in batch {
            let seq = event["seq"].as_u64().unwrap();
            assert!(seq > after_seq, "event cursor must move strictly forward: {event}");
            after_seq = seq;
            events.push(event.clone());
        }
        if batch.len() < 2 {
            break;
        }
    }
    let count_kind = |kind: &str| {
        events
            .iter()
            .filter(|event| event["kind"].as_str() == Some(kind))
            .count()
    };
    assert_eq!(count_kind("turn/started"), 1, "{events:?}");
    assert_eq!(count_kind("tool/call-started"), 1, "{events:?}");
    assert_eq!(count_kind("tool/result-recorded"), 1, "{events:?}");
    assert_eq!(count_kind("turn/failed"), 1, "{events:?}");
    let recorded = events
        .iter()
        .find(|event| event["kind"].as_str() == Some("tool/result-recorded"))
        .unwrap();
    let recorded_error = recorded["payload"]["value"]["error"].as_str().unwrap();
    assert!(recorded_error.contains("CAPABILITY_UNAVAILABLE"), "{recorded}");
    assert!(recorded_error.contains("Do not retry"), "{recorded}");

    // The forward message feed converges on one error tool row — no second row
    // is created by the reconnect.
    let mut messages = Vec::new();
    let mut message_seq = 0_u64;
    loop {
        let (status, page) = call(
            app.router.clone(),
            "GET",
            &format!("/api/agent-sessions/{session_id}/messages?after_seq={message_seq}&limit=1"),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let batch = page["data"]["messages"].as_array().unwrap();
        if batch.is_empty() {
            break;
        }
        message_seq = page["data"]["next_cursor"]["seq"].as_u64().unwrap();
        messages.extend(batch.iter().cloned());
        if batch.len() < 1 {
            break;
        }
    }
    let tool_rows: Vec<&Value> = messages
        .iter()
        .filter(|message| message["presentation_intent"].as_str() == Some("tool"))
        .collect();
    assert_eq!(tool_rows.len(), 1, "{messages:?}");
    let summary_error = tool_rows[0]["projection"]["tool_summary"]["error"]
        .as_str()
        .unwrap_or_default();
    assert!(summary_error.contains("CAPABILITY_UNAVAILABLE"), "{:?}", tool_rows[0]);
    assert!(summary_error.contains("Do not retry"), "{:?}", tool_rows[0]);

    // The renderer-facing history shows exactly one error tool row carrying the
    // recovery guidance.
    let (status, history) = call(
        app.router.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}/message-history?page_size=50"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{history}");
    let history_tools: Vec<&Value> = history["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "tool_call")
        .collect();
    assert_eq!(history_tools.len(), 1, "{history}");
    assert_eq!(history_tools[0]["status"], "error");
    assert_eq!(history_tools[0]["content"]["status"], "error");
    assert!(
        history_tools[0]["content"]["output"]
            .as_str()
            .unwrap()
            .contains("Do not retry")
    );

    // Cursor axes stay distinct after the reconnect path: a history cursor is
    // not an event cursor and a bare seq is not a history cursor.
    let history_cursor = format!(
        "{}:{}",
        history["data"]["items"][0]["created_at"].as_i64().unwrap(),
        history["data"]["items"][0]["message_id"].as_str().unwrap()
    );
    let (status, rejected) = call(
        app.router.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}/events?after_seq={history_cursor}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");
    let (status, rejected) = call(
        app.router.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}/message-history?cursor={after_seq}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{rejected}");

    expect_silent(
        &mut intruder_rx,
        Duration::from_millis(300),
        "intruder must not receive the owner's session events",
    )
    .await;

    // --- Turn 2: the reconnected consumer receives the settlement live ---
    let turn_router = app.router.clone();
    let turn_path = format!("/api/agent-sessions/{session_id}/turns");
    let turn_two = tokio::spawn(async move {
        call(
            turn_router,
            "POST",
            &turn_path,
            json!({"input": {"content": "settle with an error again"}, "idempotency_key": "w155-turn-2"}),
        )
        .await
    });

    let started = read_text(&mut owner_rx).await;
    assert_eq!(started["name"], "turn.started");
    let turn_two_root = started["data"]["turn_id"].as_str().unwrap().to_owned();
    assert_ne!(turn_two_root, turn_one_root);

    let mut seen_running = false;
    let mut seen_error_frame = false;
    let mut completed: Option<Value> = None;
    for _ in 0..8 {
        let frame = read_text(&mut owner_rx).await;
        match frame["name"].as_str().unwrap() {
            "message.stream" => {
                assert_eq!(frame["data"]["turn_id"], turn_two_root, "{frame}");
                match frame["data"]["type"].as_str().unwrap() {
                    "tool_call" => {
                        let data = &frame["data"]["data"];
                        if data["status"] == "running" {
                            assert_eq!(data["call_id"], "w155-call-2");
                            seen_running = true;
                        } else if data["status"] == "error" {
                            assert_eq!(data["call_id"], "w155-call-2");
                            let output = data["output"].as_str().unwrap_or_default();
                            assert!(output.contains("CAPABILITY_UNAVAILABLE"), "{frame}");
                            assert!(output.contains("Do not retry"), "{frame}");
                            assert_eq!(data["artifacts"], json!([]));
                            seen_error_frame = true;
                        } else {
                            panic!("unexpected tool_call status: {frame}");
                        }
                    }
                    "error" => {
                        assert_eq!(
                            frame["data"]["data"]["message"].as_str().unwrap(),
                            TURN_ERROR
                        );
                    }
                    other => panic!("unexpected stream type {other}: {frame}"),
                }
            }
            "turn.completed" => {
                completed = Some(frame);
                break;
            }
            other => panic!("unexpected frame {other}"),
        }
    }
    assert!(seen_running, "live tool running frame missing");
    assert!(seen_error_frame, "live tool error frame missing");
    let completed = completed.expect("turn.completed must arrive live");
    assert_eq!(completed["data"]["turn_id"], turn_two_root);
    assert_eq!(completed["data"]["state"], "error");
    assert_eq!(completed["data"]["detail"], TURN_ERROR);

    let (status, delivery) = turn_two.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{delivery}");

    // Idempotent replay of turn 2 must not dispatch again: no WS frame and no
    // new canonical events, and the completed receipt carries the same truth.
    let (status, replay) = call(
        app.router.clone(),
        "POST",
        &format!("/api/agent-sessions/{session_id}/turns"),
        json!({"input": {"content": "settle with an error again"}, "idempotency_key": "w155-turn-2"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["data"]["replayed"], true, "{replay}");
    assert_eq!(replay["data"]["completed"], true, "{replay}");
    assert_eq!(replay["data"]["result_ok"], false, "{replay}");
    assert_eq!(
        replay["data"]["result_error"].as_str().unwrap(),
        TURN_ERROR
    );
    expect_silent(
        &mut owner_rx,
        Duration::from_millis(300),
        "replayed turn must not push duplicate frames",
    )
    .await;

    // Durable truth still holds exactly one error tool row per turn (two
    // turns, two distinct calls) — the replay and reconnect created none.
    let (status, events) = call(
        app.router.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}/events?after_seq=0&limit=100"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{events}");
    let all = events["data"]["events"].as_array().unwrap();
    let count_kind = |kind: &str| {
        all.iter()
            .filter(|event| event["kind"].as_str() == Some(kind))
            .count()
    };
    assert_eq!(count_kind("turn/started"), 2, "{all:?}");
    assert_eq!(count_kind("tool/call-started"), 2, "{all:?}");
    assert_eq!(count_kind("tool/result-recorded"), 2, "{all:?}");
    assert_eq!(count_kind("turn/failed"), 2, "{all:?}");
    let call_ids: std::collections::BTreeSet<String> = all
        .iter()
        .filter(|event| event["kind"].as_str() == Some("tool/call-started"))
        .filter_map(|event| {
            event["payload"]["value"]["call_id"].as_str().map(str::to_owned)
        })
        .collect();
    assert_eq!(call_ids.len(), 2, "{all:?}");

    let (status, history) = call(
        app.router.clone(),
        "GET",
        &format!("/api/agent-sessions/{session_id}/message-history?page_size=50"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{history}");
    let history_tools: Vec<&Value> = history["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "tool_call")
        .collect();
    assert_eq!(history_tools.len(), 2, "{history}");
    for tool in &history_tools {
        assert_eq!(tool["status"], "error");
        assert!(tool["content"]["output"].as_str().unwrap().contains("Do not retry"));
    }

    expect_silent(
        &mut intruder_rx,
        Duration::from_millis(300),
        "intruder must not receive the owner's session events",
    )
    .await;

    app.services.shutdown_browser_platform().await.unwrap();
    app.services.database.close().await;
}
