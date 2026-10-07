//! Opt-in real-provider check of the production WebSocket reasoning lifecycle.
//! Only bounded lifecycle evidence and loopback fixture identity leave the test.

use std::collections::BTreeMap;
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite;

use super::*;

#[derive(Default)]
struct Evidence {
    phases: BTreeMap<String, bool>,
    deltas: usize,
    done: usize,
    reopened: usize,
    tools_started: usize,
    tools_completed: usize,
    active_samples: usize,
    reconnects: usize,
    recovered_frames: usize,
    recovered_active: usize,
    history_active: usize,
    history_done: usize,
    anchor: Option<i64>,
}

pub(super) struct Navigation {
    port: u16,
    state: Arc<AtomicUsize>,
}

impl Navigation {
    pub(super) async fn before_turn(&self, session_id: &str) -> Result<(), SmokeFailure> {
        // Keep libtest's selected-test stdout receipt intact.
        eprintln!("NOMIFUN_LIVE_REASONING_UI_READY port={} session={}", self.port, session_id);
        use std::io::Write;
        let _ = std::io::stderr().flush();
        self.wait(1).await
    }

    async fn wait(&self, expected: usize) -> Result<(), SmokeFailure> {
        hard_deadline("reasoning.navigation", "REASONING_UI_DEADLINE_EXCEEDED",
            Duration::from_secs(90), async {
                loop {
                    match self.state.load(Ordering::SeqCst) {
                        value if value == expected => return Ok(()),
                        3 => return Err(SmokeFailure::new("reasoning.navigation", "REASONING_UI_FAILED", 422)),
                        _ => tokio::time::sleep(Duration::from_millis(100)).await,
                    }
                }
            }).await
    }
}

async fn navigation_state(
    axum::extract::State(state): axum::extract::State<Arc<AtomicUsize>>,
    axum::Json(payload): axum::Json<Value>,
) -> StatusCode {
    let next = match payload["stage"].as_str() {
        Some("ready") => 1,
        Some("passed") => 2,
        Some("failed") => 3,
        _ => return StatusCode::BAD_REQUEST,
    };
    state.store(next, Ordering::SeqCst);
    StatusCode::NO_CONTENT
}

async fn cold_read(
    router: &Router, session: &str, turn: &str, evidence: &mut Evidence,
    terminal: bool,
) -> Result<(), SmokeFailure> {
    let response = successful_json(router, "reasoning.recovery", Method::GET,
        format!("/api/agent-sessions/{session}/projection"), None,
        LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let projection = envelope_data("reasoning.recovery", response)?;
    let active = projection.pointer("/runtime/is_processing") == Some(&json!(true));
    if active == terminal {
        return Err(SmokeFailure::new("reasoning.recovery", "REASONING_RECOVERY_ACTIVITY_MISMATCH", 409));
    }
    if active {
        if projection.pointer("/runtime/active_turn_id").and_then(Value::as_str) != Some(turn) {
            return Err(SmokeFailure::new("reasoning.recovery", "REASONING_RECOVERY_TURN_CHANGED", 409));
        }
        let anchor = projection.pointer("/runtime/processing_started_at").and_then(Value::as_i64)
            .filter(|anchor| *anchor > 0).ok_or_else(||
                SmokeFailure::new("reasoning.recovery", "REASONING_RECOVERY_CLOCK_ANCHOR_MISSING", 422))?;
        if evidence.anchor.is_some_and(|previous| previous != anchor) {
            return Err(SmokeFailure::new("reasoning.recovery", "REASONING_RECOVERY_CLOCK_ANCHOR_CHANGED", 409));
        }
        evidence.anchor = Some(anchor);
        evidence.recovered_active += 1;
    }
    let response = successful_json(router, "reasoning.recovery", Method::GET,
        format!("/api/agent-sessions/{session}/message-history?page_size=500"), None,
        LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let history = envelope_data("reasoning.recovery", response)?;
    let items = history["items"].as_array().ok_or_else(||
        SmokeFailure::new("reasoning.recovery", "REASONING_RECOVERY_HISTORY_MISSING", 502))?;
    let mut thoughts = 0;
    for item in items.iter().filter(|item| item["type"] == "thinking"
        && item.pointer("/content/turn_id").and_then(Value::as_str) == Some(turn)) {
        let id = required_string("reasoning.recovery", item, "/msg_id", "REASONING_RECOVERY_MESSAGE_ID_MISSING")?;
        match item.pointer("/content/status").and_then(Value::as_str) {
            Some("thinking") if !terminal => {
                evidence.history_active += 1;
                evidence.phases.insert(id, false);
            }
            Some("done") => {
                evidence.history_done += 1;
                evidence.phases.insert(id, true);
            }
            _ => return Err(SmokeFailure::new("reasoning.recovery", "REASONING_RECOVERY_PHASE_INVALID", 422)),
        }
        thoughts += 1;
    }
    if thoughts == 0 {
        return Err(SmokeFailure::new("reasoning.recovery", "REASONING_RECOVERY_THINKING_MISSING", 422));
    }
    Ok(())
}

async fn observe(
    router: &Router,
    address: std::net::SocketAddr,
    mut socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Result<(), SmokeFailure> {
    let mut evidence = Evidence::default();
    let mut identity: Option<(String, String)> = None;
    let mut started = None;
    let mut sample = tokio::time::interval(Duration::from_secs(1));
    sample.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let observed = hard_deadline(
        "reasoning.stream", "REASONING_STREAM_DEADLINE_EXCEEDED",
        TURN_RESULT_DEADLINE + Duration::from_secs(90),
        async {
            loop {
                tokio::select! {
                    _ = sample.tick(), if identity.is_some() => {
                        let (session, _) = identity.as_ref().expect("guarded identity");
                        let response = successful_json(router, "reasoning.sample", Method::GET,
                            format!("/api/agent-sessions/{session}/projection"), None,
                            LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
                        let projection = envelope_data("reasoning.sample", response)?;
                        if projection.pointer("/runtime/is_processing") == Some(&json!(true)) {
                            if evidence.anchor.is_some_and(|anchor|
                                projection.pointer("/runtime/processing_started_at").and_then(Value::as_i64) != Some(anchor)) {
                                return Err(SmokeFailure::new("reasoning.recovery", "REASONING_RECOVERY_CLOCK_ANCHOR_CHANGED", 409));
                            }
                            evidence.active_samples += 1;
                        }
                    }
                    message = socket.next() => {
                        let text = match message {
                            Some(Ok(tungstenite::Message::Text(text))) => text,
                            Some(Ok(_)) => continue,
                            _ => return Err(SmokeFailure::new("reasoning.stream", "REASONING_SOCKET_CLOSED", 502)),
                        };
                        let frame: Value = serde_json::from_str(&text).map_err(|_|
                            SmokeFailure::new("reasoning.stream", "REASONING_FRAME_INVALID", 502))?;
                        let name = frame["name"].as_str().unwrap_or_default();
                        if name == "ping" {
                            socket.send(tungstenite::Message::Text(json!({
                                "name": "pong", "data": { "timestamp": nomifun_common::now_ms() },
                            }).to_string().into())).await.map_err(|_| SmokeFailure::new(
                                "reasoning.stream", "REASONING_HEARTBEAT_FAILED", 502))?;
                            continue;
                        }
                        let envelope = &frame["data"];
                        if name == "turn.started" && identity.is_none() {
                            let session = required_string("reasoning.stream", envelope,
                                "/conversation_id", "REASONING_SESSION_ID_MISSING")?;
                            let turn = required_string("reasoning.stream", envelope,
                                "/turn_id", "REASONING_TURN_ID_MISSING")?;
                            identity = Some((session, turn));
                            started = Some(tokio::time::Instant::now());
                        }
                        let Some((session, turn)) = identity.as_ref() else { continue; };
                        if envelope["conversation_id"].as_str() != Some(session.as_str()) { continue; }
                        if !matches!(name, "message.stream" | "turn.completed" | "turn.paused") { continue; }
                        if evidence.reconnects > 0 { evidence.recovered_frames += 1; }
                        if envelope["turn_id"].as_str() != Some(turn.as_str()) {
                            return Err(SmokeFailure::new("reasoning.stream", "REASONING_TURN_IDENTITY_CHANGED", 409));
                        }
                        if name == "turn.paused" {
                            return Err(SmokeFailure::new("reasoning.stream", "REASONING_TURN_PAUSED", 422));
                        }
                        if name == "turn.completed" {
                            if envelope["state"] == "error" {
                                return Err(SmokeFailure::new("reasoning.stream", "REASONING_TURN_FAILED", 422));
                            }
                            cold_read(router, session, turn, &mut evidence, true).await?;
                            return Ok(());
                        }
                        let data = &envelope["data"];
                        match envelope["type"].as_str() {
                            Some("thinking") => {
                                let id = required_string("reasoning.stream", envelope,
                                    "/msg_id", "REASONING_MESSAGE_ID_MISSING")?;
                                if data["content"].as_str().is_some_and(|text| !text.is_empty()) {
                                    if data["status"] != "thinking" {
                                        return Err(SmokeFailure::new("reasoning.stream", "REASONING_DELTA_NOT_ACTIVE", 422));
                                    }
                                    if evidence.phases.insert(id, false) == Some(true) { evidence.reopened += 1; }
                                    evidence.deltas += 1;
                                    if evidence.reconnects == 0 {
                                        cold_read(router, session, turn, &mut evidence, false).await?;
                                        socket.close(None).await.map_err(|_| SmokeFailure::new(
                                            "reasoning.recovery", "REASONING_RECOVERY_DISCONNECT_FAILED", 502))?;
                                        tokio::time::sleep(Duration::from_millis(100)).await;
                                        let (reconnected, _) = tokio_tungstenite::connect_async(format!("ws://{address}/ws"))
                                            .await.map_err(|_| SmokeFailure::new(
                                                "reasoning.recovery", "REASONING_RECOVERY_RECONNECT_FAILED", 502))?;
                                        socket = reconnected;
                                        evidence.reconnects += 1;
                                        cold_read(router, session, turn, &mut evidence, false).await?;
                                    }
                                } else if data["status"] == "done" {
                                    if !evidence.phases.contains_key(&id) {
                                        return Err(SmokeFailure::new("reasoning.stream", "REASONING_DONE_WITHOUT_DELTA", 422));
                                    }
                                    evidence.phases.insert(id, true);
                                    evidence.done += 1;
                                }
                            }
                            Some("tool_call") => match data["status"].as_str() {
                                Some("running") => evidence.tools_started += 1,
                                Some("completed") => evidence.tools_completed += 1,
                                _ => {}
                            },
                            _ => {}
                        }
                    }
                }
            }
        },
    ).await;
    let all_closed = !evidence.phases.is_empty() && evidence.phases.values().all(|closed| *closed);
    eprintln!("NOMIFUN_LIVE_REASONING_EVIDENCE messages={} deltas={} done={} reopened={} tools_started={} tools_completed={} active_samples={} elapsed_ms={} all_closed={} terminal={}",
        evidence.phases.len(), evidence.deltas, evidence.done, evidence.reopened,
        evidence.tools_started, evidence.tools_completed, evidence.active_samples,
        started.map_or(0, |started| started.elapsed().as_millis()), all_closed, observed.is_ok());
    eprintln!("NOMIFUN_LIVE_REASONING_RECOVERY reconnects={} recovered_frames={} active_reads={} history_active={} history_done={} stable_anchor={} terminal_history_closed={}",
        evidence.reconnects, evidence.recovered_frames, evidence.recovered_active,
        evidence.history_active, evidence.history_done, evidence.anchor.is_some(),
        all_closed && observed.is_ok());
    observed?;
    // A model may omit reasoning in later steps. Require real streamed reasoning
    // and settled tool work without assuming a reasoning message for every step.
    if !all_closed || evidence.deltas == 0 || evidence.tools_started < 2
        || evidence.tools_completed < 2 || evidence.active_samples == 0
        || evidence.reconnects != 1 || evidence.recovered_frames == 0 || evidence.recovered_active < 2 {
        return Err(SmokeFailure::new("reasoning.stream", "REASONING_LIFECYCLE_EVIDENCE_INCOMPLETE", 422));
    }
    Ok(())
}

pub(super) async fn run(
    router: &Router, api_key: &str, model: &str, work_dir: &Path,
) -> Result<(), SmokeFailure> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|_|
        SmokeFailure::new("reasoning.listener", "REASONING_LISTENER_FAILED", 500))?;
    let address = listener.local_addr().map_err(|_|
        SmokeFailure::new("reasoning.listener", "REASONING_ADDRESS_FAILED", 500))?;
    let navigation = (std::env::var("NOMIFUN_LIVE_REASONING_NAVIGATION").as_deref() == Ok("1"))
        .then(|| Navigation { port: address.port(), state: Arc::new(AtomicUsize::new(0)) });
    let served = if let Some(navigation) = &navigation {
        router.clone().merge(Router::new().route("/__reasoning-smoke/ui-state",
            axum::routing::post(navigation_state)).with_state(navigation.state.clone()))
    } else { router.clone() };
    let server = tokio::spawn(async move { axum::serve(listener, served).await });
    let result = async {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/ws"))
            .await.map_err(|_| SmokeFailure::new("reasoning.handshake", "REASONING_HANDSHAKE_FAILED", 502))?;
        tokio::try_join!(
            // This case checks lifecycle recovery. Exact file bytes, settled
            // write/read receipts and a real nonempty terminal reply remain
            // required; final-answer wording is not a navigation contract.
            run_live_workspace_file_chain_observed(router, api_key, model, work_dir, false, false, navigation.as_ref(), false),
            observe(router, address, socket),
        )?;
        if let Some(navigation) = &navigation { navigation.wait(2).await?; }
        Ok(())
    }.await;
    server.abort();
    let _ = server.await;
    result
}
