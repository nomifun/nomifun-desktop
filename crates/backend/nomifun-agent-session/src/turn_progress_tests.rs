use super::*;
use crate::TurnProgressSuspension;

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
    assert!(store.read_turn_progress(sid,&operation).await.unwrap().suspension.is_none());
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
        assert!(matches!(observed.suspension,Some(TurnProgressSuspension::Waiting {reason:nomifun_agent_contracts::AgentWaitReason::UserInput,
            request_id:Some(ref id),..}) if id=="authorize"));
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
        assert!(store.read_turn_progress(sid,&operation).await.unwrap().suspension.is_some());
        if label == "steer" {
            let (_,accepted) = store.steer_active_turn(sid,"progress-steer-input".into(),"session_api".into(),
                StrictJsonValue(json!({"content":"Continue the original task"}))).await.unwrap();
            event["inputs"][0]["receipt_operation_id"] = json!(accepted.record.unwrap().event_id);
        }
        progress(&store,sid,&operation,&start,&format!("progress-resolve-{label}"),event).await;
        assert!(store.read_turn_progress(sid,&operation).await.unwrap().suspension.is_none(),"{label}");
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
    let lease=store.claim_native_execution(crate::NativeExecutionClaim {owner:owner(),agent_session_id:sid.clone(),
        operation_id:operation.clone(),snapshot:snapshot_ref(),active_set_generation:0,holder:"original-progress".into(),
        expected_fence:0,checkpoint:None}).await.unwrap();
    store.pause_native_execution(&lease,"EXECUTION_USER_REQUESTED",false).await.unwrap();
    assert!(matches!(store.read_turn_progress(sid,&operation).await.unwrap().suspension,
        Some(TurnProgressSuspension::NativePause {ref reason,..}) if reason=="EXECUTION_USER_REQUESTED"));
    store.cancel_active_turn(sid,"progress-cancel".into(),"session_api".into()).await.unwrap();
    store.start_turn(sid,"session_api".into(),"progress-next-start".into(),"progress-successor".into(),
        StrictJsonValue(json!({"content":"Independent successor"}))).await.unwrap();
    let old=store.read_turn_progress(sid,&operation).await.unwrap();
    assert_eq!(old.status,TurnReceiptStatus::Cancelled);assert!(old.suspension.is_none());
    let next=store.read_turn_progress(sid,&"progress-successor".into()).await.unwrap();
    assert_eq!(next.status,TurnReceiptStatus::Running);assert!(next.suspension.is_none());
}

#[tokio::test]
async fn turn_progress_preserves_exact_native_pause_identity_and_distinguishes_user_decision() {
    for reason in ["EXECUTION_INTERVENTION_REQUIRED","NATIVE_RECOVERY_RECONCILIATION_REQUIRED","EXECUTION_USER_REQUESTED"] {
        let store=AgentSessionStore::open_in_memory().await.unwrap();
        let(session,_)=create_ready(&store,&format!("progress-{reason}")).await;
        let sid=&session.agent_session_id;let operation=OperationId::from("original-progress-pause");
        store.start_turn(sid,"session_api".into(),"pause-input".into(),operation.clone(),
            StrictJsonValue(json!({"content":"Keep the original task"}))).await.unwrap();
        let lease=store.claim_native_execution(crate::NativeExecutionClaim {owner:owner(),agent_session_id:sid.clone(),
            operation_id:operation.clone(),snapshot:snapshot_ref(),active_set_generation:0,holder:"progress-original".into(),
            expected_fence:0,checkpoint:None}).await.unwrap();
        let pause=store.pause_native_execution(&lease,reason,false).await.unwrap();
        let before=store.current_cursor(sid).await.unwrap();
        let observed=store.read_turn_progress(sid,&operation).await.unwrap();
        assert_eq!(observed.status,TurnReceiptStatus::Running,"a pause does not invent a canonical terminal");
        assert_eq!(observed.suspension,Some(TurnProgressSuspension::NativePause {
            revision:pause.revision,reason:pause.reason,checkpoint_revision:pause.checkpoint_revision,
            execution_fence:pause.execution_fence,cleanup_proven:false,
        }));
        assert_eq!(observed.suspension.unwrap().requires_intervention(),reason!="EXECUTION_USER_REQUESTED");
        assert_eq!(store.current_cursor(sid).await.unwrap(),before,"observation cannot author a new state or resume grant");
        assert!(store.read_turn_receipt(sid,&operation).await.unwrap().terminal_event.is_none());
    }
}

#[tokio::test]
async fn turn_progress_resource_and_intervention_waits_keep_their_distinct_typed_boundary() {
    for (reason,typed,decision) in [("external_resource",nomifun_agent_contracts::AgentWaitReason::ExternalResource,false),
        ("intervention_required",nomifun_agent_contracts::AgentWaitReason::InterventionRequired,true)] {
        let store=AgentSessionStore::open_in_memory().await.unwrap();
        let(session,_)=create_ready(&store,&format!("progress-wait-{reason}")).await;
        let sid=&session.agent_session_id;let operation=OperationId::from("typed-wait");
        let(_,started)=store.start_turn(sid,"session_api".into(),"typed-wait-input".into(),operation.clone(),
            StrictJsonValue(json!({"content":"Wait on the original owner"}))).await.unwrap();
        progress(&store,sid,&operation,&started.ack.unwrap().event_id,"typed-resource-wait",
            json!({"event":"turn_waiting","model_steps":1,"reason":reason,"request_id":null})).await;
        let observed=store.read_turn_progress(sid,&operation).await.unwrap();
        let boundary=observed.suspension.unwrap();
        assert!(matches!(&boundary,TurnProgressSuspension::Waiting {wait_seq,reason,request_id:None}
            if *wait_seq==observed.cursor && *reason==typed));
        assert_eq!(boundary.requires_intervention(),decision);
    }
}
