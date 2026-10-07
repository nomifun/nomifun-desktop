//! Opt-in real-provider check of the production WebSocket reasoning lifecycle.
//! Only counts and bounded timing evidence leave the disposable fixture.

use std::collections::BTreeMap;

use futures_util::StreamExt;
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
}

async fn observe(
    router: &Router,
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
        TURN_RESULT_DEADLINE + Duration::from_secs(20),
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
    observed?;
    // A model may omit reasoning in later steps. Require real streamed reasoning
    // and settled tool work without assuming a reasoning message for every step.
    if !all_closed || evidence.deltas == 0 || evidence.tools_started < 2
        || evidence.tools_completed < 2 || evidence.active_samples == 0 {
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
    let served = router.clone();
    let server = tokio::spawn(async move { axum::serve(listener, served).await });
    let result = async {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/ws"))
            .await.map_err(|_| SmokeFailure::new("reasoning.handshake", "REASONING_HANDSHAKE_FAILED", 502))?;
        let (execution, observation) = tokio::join!(
            run_live_workspace_file_chain(router, api_key, model, work_dir, false, false),
            observe(router, socket),
        );
        execution?;
        observation
    }.await;
    server.abort();
    let _ = server.await;
    result
}
