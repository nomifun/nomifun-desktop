use super::*;

async fn start(store: &AgentSessionStore, session: &AgentSessionId, operation: &str) -> EventId {
    store.start_turn(session, "session-api".into(), operation.into(), operation.into(),
        StrictJsonValue(json!({"content":"Work on this task"}))).await.unwrap().1.ack.unwrap().event_id
}

fn progress(session: &AgentSessionId, started: &EventId, operation: &str, key: &str, event: serde_json::Value) -> SessionEventAppend {
    append(session, key, "runtime_supervisor", key, "runtime/progress-recorded", operation,
        Some(started.clone()), json!({"producer_seq":1,"event":event}))
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

#[tokio::test]
async fn latest_runtime_state_projects_native_pause_resume_and_irreversible_cancellation() {
    use crate::{NativeCheckpointWrite, NativeExecutionClaim, NativeResumePreparation, NativeResumeRequest};

    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, _) = create_ready(&store, "plan-native-pause").await;
    let sid = &session.agent_session_id;
    let operation = OperationId::from("first");
    let started = start(&store, sid, operation.as_ref()).await;
    let lease = store.claim_native_execution(NativeExecutionClaim {
        owner: owner(), agent_session_id: sid.clone(), operation_id: operation.clone(),
        snapshot: snapshot_ref(), active_set_generation: 0, holder: "plan-fixture".into(),
        expected_fence: 0, checkpoint: None,
    }).await.unwrap();
    store.append_native_event(&lease, &progress(sid, &started, operation.as_ref(), "plan-before-pause", plan(1, false)), None)
        .await.unwrap();
    let checkpoint_state = StrictJsonValue(json!({"version":1,"model_steps":0}));
    let digest = digest_bytes(&canonical_json_bytes(&checkpoint_state.0).unwrap());
    let checkpoint = store.save_native_checkpoint(
        &progress(sid, &started, operation.as_ref(), "plan-checkpoint", json!({
            "event":"execution_checkpoint_saved","step":0,"revision":1,"digest":digest
        })),
        NativeCheckpointWrite {
            owner: owner(), operation_id: operation.clone(), snapshot: snapshot_ref(), active_set_generation: 0,
            expected_revision: 0, execution_fence: lease.fence(), lease: Some(lease.clone()), state: checkpoint_state,
        },
    ).await.unwrap();
    let pause = store.pause_native_execution(&lease, "OWNER_REQUESTED", true).await.unwrap();
    let paused = store.latest_runtime_state(sid, "plan_updated").await.unwrap();
    assert_eq!(paused.turn_status.as_deref(), Some("paused"));
    let durable_state: String = sqlx::query_scalar("SELECT state FROM agent_turns WHERE session_id=? AND operation_id=?")
        .bind(sid.as_ref()).bind(operation.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    assert_eq!(durable_state, "running", "public pause must not rewrite the Turn's terminal state machine");
    store.rebuild_projections(sid).await.unwrap();
    assert_eq!(store.latest_runtime_state(sid, "plan_updated").await.unwrap().turn_status.as_deref(), Some("paused"));
    let facts = store.native_recovery_facts(sid, &operation).await.unwrap();
    store.commit_native_resume(&owner(), sid, &NativeResumeRequest {
        operation_id: operation.clone(), idempotency_key: "plan-resume".into(),
        expected_pause_revision: pause.revision, expected_checkpoint_revision: checkpoint.revision,
        expected_checkpoint_digest: checkpoint.digest, budget: Default::default(), cleanup_attestation: None,
    }, NativeResumePreparation {
        expected_head_seq: facts.head.last_seq, expected_fence: facts.execution_fence,
        snapshot: snapshot_ref(), active_set_generation: 0, observations: vec![],
        checkpoint_state: StrictJsonValue(json!({"version":1,"model_steps":0,"turn_operation_id":operation,
            "active_set_generation":0,"binding":{"agent_session_id":sid,"resolved_snapshot_ref":snapshot_ref()}})),
    }).await.unwrap();
    let resumed = store.latest_runtime_state(sid, "plan_updated").await.unwrap();
    assert_eq!(resumed.turn_status.as_deref(), Some("running"));
    assert_eq!(resumed.event, paused.event);
    store.cancel_active_turn(sid, "plan-cancel".into(), "session-api".into()).await.unwrap();
    let cancelled = store.latest_runtime_state(sid, "plan_updated").await.unwrap();
    assert_eq!(cancelled.turn_status.as_deref(), Some("cancelled"));
    assert_eq!(cancelled.event, paused.event);
    store.rebuild_projections(sid).await.unwrap();
    assert_eq!(store.latest_runtime_state(sid, "plan_updated").await.unwrap().turn_status.as_deref(), Some("cancelled"));
}
