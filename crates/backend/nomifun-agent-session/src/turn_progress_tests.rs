use super::*;

async fn progress(store: &AgentSessionStore, session: &AgentSessionId, operation: &OperationId,
    start: &EventId, key: &str, event: serde_json::Value) {
    store.append_event(&append(session,key,"runtime_supervisor",key,"runtime/progress-recorded",
        operation.as_ref(),Some(start.clone()),json!({"producer_seq":1,"event":event}))).await.unwrap();
}

async fn wait(store: &AgentSessionStore, session: &AgentSessionId, operation: &OperationId,
    start: &EventId, key: &str) {
    progress(store,session,operation,start,&format!("{key}-question"),json!({"event":"user_input_requested","step":1,
        "request":{"request_id":key,"question":"Authorize this operation","remaining_work":"Continue the same task"}})).await;
    progress(store,session,operation,start,&format!("{key}-wait"),json!({"event":"turn_waiting","model_steps":1,
        "reason":"user_input","request_id":key})).await;
}

#[tokio::test]
async fn turn_progress_reads_current_wait_outside_history_pages_and_does_not_write_facts() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();
    let(session,_)=create_ready(&store,"progress-wait").await;
    let sid=&session.agent_session_id;let operation=OperationId::from("progress-original");
    let(_,started)=store.start_turn(sid,"session_api".into(),"progress-input".into(),operation.clone(),
        StrictJsonValue(json!({"content":"Authorize and continue"}))).await.unwrap();
    let start=started.ack.unwrap().event_id;
    assert!(!store.read_turn_progress(sid,&operation).await.unwrap().suspended);
    wait(&store,sid,&operation,&start,"authorize").await;
    for index in 0..100 {
        progress(&store,sid,&operation,&start,&format!("progress-noise-{index}"),
            json!({"event":"work_status","status":{}})).await;
    }
    let before=store.current_cursor(sid).await.unwrap();
    for _ in 0..3 {
        let observed=store.read_turn_progress(sid,&operation).await.unwrap();
        assert_eq!(observed.status,TurnReceiptStatus::Running);
        assert_eq!(observed.cursor,before.seq);
        assert!(observed.suspended);
    }
    assert_eq!(store.current_cursor(sid).await.unwrap(),before);
}

#[tokio::test]
async fn turn_progress_wait_resolves_on_the_same_canonical_boundaries_as_full_history() {
    for (label,mut event) in [
        ("resolved",json!({"event":"turn_wait_resolved","model_steps":1})),
        ("model",json!({"event":"model_step_started","step":2,"operation_id":"next-model"})),
        ("tool",json!({"event":"tool_started","step":1,"call":{"call_id":"next-tool","name":"read_file","arguments":{}}})),
        ("steer",json!({"event":"steering_inputs","step":1,"inputs":[{"content":"continue"}]})),
    ] {
        let store=AgentSessionStore::open_in_memory().await.unwrap();
        let(session,_)=create_ready(&store,&format!("progress-{label}")).await;
        let sid=&session.agent_session_id;let operation=OperationId::from("progress-turn");
        let(_,started)=store.start_turn(sid,"session_api".into(),"progress-start".into(),operation.clone(),
            StrictJsonValue(json!({"content":"Wait"}))).await.unwrap();
        let start=started.ack.unwrap().event_id;
        wait(&store,sid,&operation,&start,"first").await;
        assert!(store.read_turn_progress(sid,&operation).await.unwrap().suspended);
        if label == "steer" {
            let (_,accepted) = store.steer_active_turn(sid,"progress-steer-input".into(),"session_api".into(),
                StrictJsonValue(json!({"content":"Continue the original task"}))).await.unwrap();
            event["inputs"][0]["receipt_operation_id"] = json!(accepted.record.unwrap().event_id);
        }
        progress(&store,sid,&operation,&start,&format!("progress-resolve-{label}"),event).await;
        assert!(!store.read_turn_progress(sid,&operation).await.unwrap().suspended,"{label}");
        assert!(store.turn_outcome(sid,&operation).await.unwrap().is_none(),"{label}");
    }
}

#[tokio::test]
async fn turn_progress_validates_the_applicable_question_and_rejects_a_stale_wait() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();
    let(session,_)=create_ready(&store,"progress-stale").await;
    let sid=&session.agent_session_id;let operation=OperationId::from("progress-stale-turn");
    let(_,started)=store.start_turn(sid,"session_api".into(),"progress-stale-start".into(),operation.clone(),
        StrictJsonValue(json!({"content":"Wait"}))).await.unwrap();
    let start=started.ack.unwrap().event_id;
    wait(&store,sid,&operation,&start,"old-question").await;
    progress(&store,sid,&operation,&start,"progress-resolved",json!({"event":"turn_wait_resolved","model_steps":1})).await;
    progress(&store,sid,&operation,&start,"progress-stale-wait",json!({"event":"turn_waiting","model_steps":1,
        "reason":"user_input","request_id":"old-question"})).await;
    assert!(store.read_turn_progress(sid,&operation).await.is_err());
    assert!(store.turn_outcome(sid,&operation).await.is_err());
}

#[tokio::test]
async fn turn_progress_pause_and_terminal_remain_bound_to_the_original_turn() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();
    let(session,_)=create_ready(&store,"progress-pause").await;
    let sid=&session.agent_session_id;let operation=OperationId::from("progress-paused-turn");
    store.start_turn(sid,"session_api".into(),"progress-paused-start".into(),operation.clone(),
        StrictJsonValue(json!({"content":"Keep progress"}))).await.unwrap();
    sqlx::query("UPDATE agent_session_heads SET status='paused' WHERE session_id=?")
        .bind(sid.as_ref()).execute(store.test_pool()).await.unwrap();
    assert!(store.read_turn_progress(sid,&operation).await.unwrap().suspended);
    store.cancel_active_turn(sid,"progress-cancel".into(),"session_api".into()).await.unwrap();
    store.start_turn(sid,"session_api".into(),"progress-next-start".into(),"progress-successor".into(),
        StrictJsonValue(json!({"content":"Independent successor"}))).await.unwrap();
    let old=store.read_turn_progress(sid,&operation).await.unwrap();
    assert_eq!(old.status,TurnReceiptStatus::Cancelled);assert!(!old.suspended);
    let next=store.read_turn_progress(sid,&"progress-successor".into()).await.unwrap();
    assert_eq!(next.status,TurnReceiptStatus::Running);assert!(!next.suspended);
}
