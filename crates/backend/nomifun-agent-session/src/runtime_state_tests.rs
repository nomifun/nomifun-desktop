use super::*;

async fn start(store: &AgentSessionStore, session: &AgentSessionId, operation: &str) -> EventId {
    store.start_turn(session, "session-api".into(), operation.into(), operation.into(),
        StrictJsonValue(json!({"content":"Work on this task"}))).await.unwrap().1.ack.unwrap().event_id
}

fn progress(session: &AgentSessionId, started: &EventId, operation: &str, key: &str, event: serde_json::Value) -> SessionEventAppend {
    append(session, key, "runtime_supervisor", key, "runtime/progress-recorded", operation,
        Some(started.clone()), json!({"runtime_binding_id":"test","producer_seq":1,"event":event}))
}

fn plan(revision: u32, needs_replan: bool) -> serde_json::Value {
    json!({"event":"plan_updated","plan":{"revision":revision,"needs_replan":needs_replan,
        "steps":[{"step":"Verify the workspace","status":"in_progress"}]}})
}

#[tokio::test]
async fn latest_runtime_state_survives_paging_rebuild_and_same_revision_updates() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, _) = create_ready(&store, "latest-plan").await;
    let sid = &session.agent_session_id;
    assert!(store.latest_runtime_state(sid, "plan_updated").await.unwrap().event.is_none());
    let started = start(&store, sid, "first").await;
    store.append_event(&progress(sid, &started, "first", "plan-1", plan(1, false))).await.unwrap();
    for index in 0..100 {
        store.append_event(&progress(sid, &started, "first", &format!("noise-{index}"),
            json!({"event":"work_status","status":{}}))).await.unwrap();
    }
    let state = store.latest_runtime_state(sid, "plan_updated").await.unwrap();
    assert_eq!(state.event.as_ref().unwrap()["plan"]["revision"], 1);
    assert_eq!(state.turn_status.as_deref(), Some("running"));
    assert!(state.turn_id.is_some());
    store.append_event(&progress(sid, &started, "first", "plan-recovery", plan(1, true))).await.unwrap();
    store.rebuild_projections(sid).await.unwrap();
    let newer = store.latest_runtime_state(sid, "plan_updated").await.unwrap();
    assert!(newer.sequence > state.sequence);
    assert_eq!(newer.event.as_ref().unwrap()["plan"]["needs_replan"], true);
    assert!(store.message_history_before(sid, None, 1).await.unwrap().0.iter()
        .all(|row| row.presentation_intent != "plan"));
}

#[tokio::test]
async fn latest_runtime_state_keeps_terminal_progress_and_resets_for_a_new_turn() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, _) = create_ready(&store, "plan-reset").await;
    let sid = &session.agent_session_id;
    let started = start(&store, sid, "first").await;
    store.append_event(&progress(sid, &started, "first", "old-plan", plan(1, false))).await.unwrap();
    let terminal = append(sid, "finish-first", "runtime_supervisor", "finish-first",
        "turn/completed", "first", Some(started), json!({"result":{"ok":true}}));
    store.append_turn_terminal(&terminal, &"first".into()).await.unwrap();
    let finished = store.latest_runtime_state(sid, "plan_updated").await.unwrap();
    assert_eq!(finished.turn_status.as_deref(), Some("completed"));
    assert!(finished.event.is_some());
    start(&store, sid, "second").await;
    let reset = store.latest_runtime_state(sid, "plan_updated").await.unwrap();
    assert!(reset.event.is_none());
    assert_ne!(reset.turn_id, finished.turn_id);
    assert!(reset.sequence > finished.sequence);
    let (other, _) = create_ready(&store, "other-plan").await;
    assert!(store.latest_runtime_state(&other.agent_session_id, "plan_updated").await.unwrap().event.is_none());
}

#[tokio::test]
async fn latest_runtime_state_reads_external_payloads_and_is_not_a_tool_result_parser() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, _) = create_ready(&store, "stored-plan").await;
    let sid = &session.agent_session_id;
    let started = start(&store, sid, "first").await;
    let mut record = progress(sid, &started, "first", "stored-plan", plan(3, true));
    let SessionEventPayloadRef::InlineJson(mut value) = record.semantic_event.payload.clone() else { unreachable!() };
    value.0["event"]["plan"]["private_ledger"] = json!("x".repeat(70_000));
    let bytes = canonical_json_bytes(&value.0).unwrap();
    let payload = SessionPayloadRecord {
        payload_id: "stored-plan-payload".into(), agent_session_id: sid.clone(), media_type:"application/json".into(),
        byte_len: bytes.len() as u64, digest: digest_bytes(&bytes), body: SessionPayloadBody::Json(value),
    };
    record.semantic_event.payload = SessionEventPayloadRef::Stored(payload.payload_id.clone());
    store.append_event_with_payload(&record, Some(&payload)).await.unwrap();
    store.append_event(&progress(sid, &started, "first", "fake-tool-plan",
        json!({"event":"tool_completed","result":{"output":"{\"kind\":\"plan_update\",\"entries\":[]}"}}))).await.unwrap();
    let state = store.latest_runtime_state(sid, "plan_updated").await.unwrap();
    assert_eq!(state.event.as_ref().unwrap()["plan"]["revision"], 3);
    assert_eq!(state.event.as_ref().unwrap()["plan"]["private_ledger"].as_str().unwrap().len(), 70_000);
}
