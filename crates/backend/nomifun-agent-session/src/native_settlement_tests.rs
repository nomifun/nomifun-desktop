use super::*;
use crate::NativeTurnSettlementRequest;

fn request()->NativeTurnSettlementRequest {
    NativeTurnSettlementRequest {outcome:StrictJsonValue(json!({"model_steps":1,"terminal":{"status":"failed","message":"original execution failure"}})),
        first_cleanup_failure:"original cleanup rejection".into()}
}

async fn proof(store:&AgentSessionStore,f:&Fixture,receipt:&crate::NativeTurnSettlementReceipt,key:&str) {
    let mut append=progress(f,key,json!({"event":"host_cleanup_proven"}));
    append.semantic_event.causation_event_id=Some(f.root.clone());
    store.append_native_settlement_observation(receipt,&append,None).await.unwrap();
}

fn terminal(f:&Fixture,key:&str,message:&str)->(SessionEventAppend,SessionEventAppend) {
    let message_event=append(&f.session.agent_session_id,&format!("{key}-message"),"runtime_supervisor",&format!("{key}-message"),"message/completed",key,Some(f.root.clone()),
        json!({"part_count":0,"content_digest":digest_bytes(b"")}));
    let terminal=append(&f.session.agent_session_id,key,"runtime_supervisor",key,"turn/failed","lease-turn",Some(message_event.event_id.clone()),
        json!({"model_steps":1,"message":message,"finished_at_ms":1_799_000_000_000i64}));
    (message_event,terminal)
}

async fn finish_failed(store:&AgentSessionStore,f:&Fixture,receipt:&crate::NativeTurnSettlementReceipt,message:&SessionEventAppend,end:&SessionEventAppend)
    ->Result<crate::NativeTurnSettlementBundleReceipt,SessionStoreError> {
    let runtime=progress(f,&format!("{}:runtime",end.event_id.as_ref()),json!({"event":"turn_failed",
        "model_steps":receipt.outcome.0["model_steps"],"message":receipt.outcome.0["terminal"]["message"]}));
    store.append_native_settlement_terminal_bundle(receipt,&runtime,None,Some(message),Some(end)).await
}

async fn finish_paused(store:&AgentSessionStore,f:&Fixture,receipt:&crate::NativeTurnSettlementReceipt)->crate::NativePauseState {
    let runtime=progress(f,&format!("{}:user-pause-runtime",receipt.event_id.as_ref()),json!({"event":"turn_paused",
        "model_steps":receipt.outcome.0["model_steps"],"reason":"EXECUTION_USER_REQUESTED"}));
    store.append_native_settlement_terminal_bundle(receipt,&runtime,None,None,None).await.unwrap();
    store.native_pause_state(&f.session.agent_session_id,&"lease-turn".into()).await.unwrap().unwrap()
}

#[tokio::test]
async fn successful_handoff_retains_original_exit_and_only_settlement_can_cross_technical_pause() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,"settlement-handoff").await;
    let lease=store.claim_native_execution(claim(&f,"exact-native-owner",0,None)).await.unwrap();
    let requested=request();let receipt=store.request_native_turn_settlement(&lease,&requested).await.unwrap();
    assert_eq!(store.head(&f.session.agent_session_id).await.unwrap().status,"paused");
    assert!(!receipt.pause.as_ref().unwrap().cleanup_proven);
    assert_eq!(receipt.outcome,requested.outcome);
    let repeated=store.request_native_turn_settlement(&lease,&requested).await.unwrap();
    assert_eq!(repeated.event_id,receipt.event_id);
    let mut changed=requested.clone();changed.first_cleanup_failure="later rejection must not replace the first".into();
    assert!(matches!(store.request_native_turn_settlement(&lease,&changed).await,Err(SessionStoreError::IdempotencyConflict(_))));
    let pending=store.pending_native_turn_settlements(0,500).await.unwrap();
    assert_eq!(pending.len(),1);assert_eq!(pending[0].receipt.event_id,receipt.event_id);
    let loaded=store.read_native_turn_settlement(&owner(),&f.session.agent_session_id,&"lease-turn".into()).await.unwrap().unwrap();
    assert_eq!(loaded.outcome,requested.outcome);
    assert!(store.claim_native_chat_operation(&lease,model(&f,"must-not-restart")).await.is_err());
    assert!(store.append_native_event(&lease,&progress(&f,"old-work",json!({"event":"model_step_started","step":2})),None).await.is_err());
    for event in [json!({"event":"model_step_started","step":2}),json!({"event":"tool_started","call_id":"new-tool"}),json!({"event":"host_tool_dispatch","dispatch":{"operation_id":"new-work","call_id":"new-call"}})] {
        assert!(store.append_native_settlement_observation(&receipt,&progress(&f,"forbidden-settlement",event),None).await.is_err());
    }
    let (message,end)=terminal(&f,"final-original","original execution failure");
    assert!(finish_failed(&store,&f,&receipt,&message,&end).await.is_err());
    proof(&store,&f,&receipt,"original-owner-cleanup").await;
    let (changed_message,changed_end)=terminal(&f,"changed-result","a different failure");
    assert!(finish_failed(&store,&f,&receipt,&changed_message,&changed_end).await.is_err());
    finish_failed(&store,&f,&receipt,&message,&end).await.unwrap();
    store.complete_native_turn_settlement(&receipt).await.unwrap();
    store.complete_native_turn_settlement(&receipt).await.unwrap();
    assert!(store.pending_native_turn_settlements(0,500).await.unwrap().is_empty());
    assert_eq!(store.read_turn_receipt(&f.session.agent_session_id,&"lease-turn".into()).await.unwrap().status,TurnReceiptStatus::Failed);
}

#[tokio::test]
async fn stop_wins_after_handoff_and_old_cleanup_cannot_overwrite_the_successor() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,"settlement-stop").await;
    let lease=store.claim_native_execution(claim(&f,"exact-native-owner",0,None)).await.unwrap();
    let receipt=store.request_native_turn_settlement(&lease,&request()).await.unwrap();
    store.cancel_active_turn(&f.session.agent_session_id,"user-stop".into(),"session-api".into()).await.unwrap();
    let stopped=store.read_turn_receipt(&f.session.agent_session_id,&"lease-turn".into()).await.unwrap().terminal_event.unwrap();
    let successor_input=append(&f.session.agent_session_id,"successor-input","session-api","successor-input","message/user-accepted","successor",None,json!({"content":"independent next task"}));
    let input=store.append_event(&successor_input).await.unwrap().ack.unwrap().event_id;
    store.append_event(&append(&f.session.agent_session_id,"successor-start","session-api","successor-start","turn/started","successor",Some(input),
        json!({"operation_id":"successor","resolved_snapshot_ref":snapshot_ref()}))).await.unwrap();
    proof(&store,&f,&receipt,"stopped-owner-cleanup").await;
    let (message,end)=terminal(&f,"must-not-fail-stopped","original execution failure");
    assert!(finish_failed(&store,&f,&receipt,&message,&end).await.unwrap().runtime_seq.is_none());
    store.complete_native_turn_settlement(&receipt).await.unwrap();
    let current=store.head(&f.session.agent_session_id).await.unwrap();
    assert_eq!(current.active_turn_id.as_deref(),Some("successor"));assert_eq!(current.status,"running");
    assert_eq!(store.read_turn_receipt(&f.session.agent_session_id,&"lease-turn".into()).await.unwrap().terminal_event.unwrap().event_id,stopped.event_id);
}

#[tokio::test]
async fn settlement_capability_rejects_replaced_owner_generation_pause_and_foreign_projection() {
    for mutation in ["holder","generation","fence","pause","foreign_projection"] {
        let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,mutation).await;
        let lease=store.claim_native_execution(claim(&f,"original-owner",0,None)).await.unwrap();
        let receipt=store.request_native_turn_settlement(&lease,&request()).await.unwrap();
        let sid=f.session.agent_session_id.as_ref();
        match mutation {
            "holder"=>{sqlx::query("UPDATE agent_turns SET execution_owner='replacement' WHERE session_id=?").bind(sid).execute(store.test_pool()).await.unwrap();},
            "generation"=>{sqlx::query("UPDATE agent_turns SET execution_generation=execution_generation+1 WHERE session_id=?").bind(sid).execute(store.test_pool()).await.unwrap();},
            "fence"=>{sqlx::query("UPDATE agent_turns SET execution_fence=execution_fence+1 WHERE session_id=?").bind(sid).execute(store.test_pool()).await.unwrap();},
            "pause"=>{sqlx::query("UPDATE agent_turns SET native_pause_json=json_set(native_pause_json,'$.reason','EXECUTION_USER_REQUESTED','$.revision',2) WHERE session_id=?").bind(sid).execute(store.test_pool()).await.unwrap();},
            _=>{},
        }
        let before=store.current_cursor(&f.session.agent_session_id).await.unwrap();
        let mut observed=progress(&f,"must-remain-fenced",json!({"event":"host_cleanup_proven"}));
        if mutation=="foreign_projection" {observed.semantic_event.causation_event_id=None;}
        assert!(store.append_native_settlement_observation(&receipt,&observed,None).await.is_err(),"{mutation}");
        assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(),before,"{mutation}");
    }
}

#[tokio::test]
async fn terminal_write_failure_keeps_the_original_cleanup_witness_without_repeating_cleanup() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,"settlement-terminal-fault").await;
    let lease=store.claim_native_execution(claim(&f,"original-owner",0,None)).await.unwrap();
    let mut proof=progress(&f,"cleanup-before-handoff",json!({"event":"host_cleanup_proven"}));
    proof.semantic_event.causation_event_id=Some(f.root.clone());
    store.append_native_observation(&lease,&proof,None).await.unwrap();
    let receipt=store.request_native_turn_settlement(&lease,&request()).await.unwrap();
    assert!(store.native_turn_settlement_cleanup_proven(&receipt).await.unwrap());
    let (message,end)=terminal(&f,"original-terminal-after-handoff","original execution failure");
    finish_failed(&store,&f,&receipt,&message,&end).await.unwrap();
    store.complete_native_turn_settlement(&receipt).await.unwrap();
    let events=store.read_events(&f.session.agent_session_id,None,500).await.unwrap().events;
    assert_eq!(events.iter().filter(|event|event.event_id.as_ref()=="cleanup-before-handoff").count(),1);
    assert_eq!(events.iter().filter(|event|event.kind.0=="turn/failed").count(),1);
}

#[tokio::test]
async fn lost_ack_of_committed_terminal_or_genuine_pause_never_rewrites_the_original_boundary() {
    for paused in [false,true] {
        let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,"settlement-ack-loss").await;
        let lease=store.claim_native_execution(claim(&f,"original-owner",0,None)).await.unwrap();
        store.append_native_observation(&lease,&progress(&f,"cleanup-before-original-exit",json!({"event":"host_cleanup_proven"})),None).await.unwrap();
        let mut original=request();
        let old_pause=if paused {
            original.outcome=StrictJsonValue(json!({"model_steps":1,"terminal":{"status":"paused","reason":"EXECUTION_USER_REQUESTED"}}));
            Some(store.pause_native_execution(&lease,"EXECUTION_USER_REQUESTED",true).await.unwrap())
        } else {
            let (message,end)=terminal(&f,"already-committed-original","original execution failure");
            store.append_native_chat_completion(&lease,&message,&end).await.unwrap();None
        };
        let receipt=store.request_native_turn_settlement(&lease,&original).await.unwrap();
        if let Some(old)=old_pause {
            let confirmed=finish_paused(&store,&f,&receipt).await;
            assert_eq!(serde_json::to_value(&confirmed).unwrap(),serde_json::to_value(&old).unwrap());
            assert_eq!(store.head(&f.session.agent_session_id).await.unwrap().status,"paused");
        } else {store.complete_native_turn_settlement(&receipt).await.unwrap();}
        assert!(store.pending_native_turn_settlements(0,500).await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn explicit_user_pause_survives_cleanup_handoff_and_never_authorizes_automatic_continuation() {
    for matching_exit in [true,false] {
        let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,"settlement-user-pause").await;
        let lease=store.claim_native_execution(claim(&f,"original-owner",0,None)).await.unwrap();
        let pause_request=store.request_native_pause(&owner(),&f.session.agent_session_id,&"lease-turn".into(),"original-user-pause","inspect current work before continuing").await.unwrap();
        let mut requested=request();
        if matching_exit {requested.outcome=StrictJsonValue(json!({"model_steps":1,"terminal":{"status":"paused","reason":"EXECUTION_USER_REQUESTED"}}));}
        let receipt=store.request_native_turn_settlement(&lease,&requested).await.unwrap();
        assert_eq!(receipt.pause.as_ref().unwrap().reason,"EXECUTION_USER_REQUESTED");
        assert!(store.native_turn_settlement_requires_user_pause(&receipt).await.unwrap());
        proof(&store,&f,&receipt,"user-pause-owner-cleanup").await;
        if matching_exit {
            let paused=finish_paused(&store,&f,&receipt).await;
            assert!(paused.cleanup_proven);assert_eq!(paused.reason,"EXECUTION_USER_REQUESTED");
        } else {
            let (message,end)=terminal(&f,"must-not-bypass-user-pause","original execution failure");
            assert!(finish_failed(&store,&f,&receipt,&message,&end).await.is_err());
            let paused=finish_paused(&store,&f,&receipt).await;
            assert!(paused.cleanup_proven);assert_eq!(paused.reason,"EXECUTION_USER_REQUESTED");
        }
        assert!(store.pending_native_turn_settlements(0,500).await.unwrap().is_empty());
        assert_eq!(store.head(&f.session.agent_session_id).await.unwrap().status,"paused");
        assert!(store.claim_native_chat_operation(&lease,model(&f,"pause-never-auto-resumes")).await.is_err());
        let events=store.read_events(&f.session.agent_session_id,None,500).await.unwrap().events;
        assert!(events.iter().any(|event|event.event_id==pause_request.event_id));
        assert!(!events.iter().any(|event|event.kind.0=="turn/resume-authorized"));
    }
}

#[tokio::test]
async fn late_user_pause_keeps_cleanup_authority_and_terminal_bundle_cannot_commit_a_premature_runtime_exit() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,"settlement-late-user-pause").await;
    let lease=store.claim_native_execution(claim(&f,"original-owner",0,None)).await.unwrap();
    let receipt=store.request_native_turn_settlement(&lease,&request()).await.unwrap();
    proof(&store,&f,&receipt,"original-cleanup").await;
    let runtime=progress(&f,"atomic-failed-runtime",json!({"event":"turn_failed","model_steps":1,"message":"original execution failure"}));
    let (message,end)=terminal(&f,"atomic-original-end","original execution failure");
    sqlx::query("CREATE TRIGGER reject_canonical_terminal BEFORE INSERT ON agent_events WHEN NEW.kind='turn/failed' BEGIN SELECT RAISE(FAIL,'fixture terminal rejected'); END")
        .execute(store.test_pool()).await.unwrap();
    assert!(store.append_native_settlement_terminal_bundle(&receipt,&runtime,None,Some(&message),Some(&end)).await.is_err());
    let events=store.read_events(&f.session.agent_session_id,None,500).await.unwrap().events;
    assert!(!events.iter().any(|event|event.event_id==runtime.event_id),"native terminal and canonical terminal must roll back together");
    sqlx::query("DROP TRIGGER reject_canonical_terminal").execute(store.test_pool()).await.unwrap();
    store.request_native_pause(&owner(),&f.session.agent_session_id,&"lease-turn".into(),"late-user-pause","inspect before continuation").await.unwrap();
    assert!(store.native_turn_settlement_current(&receipt).await.unwrap());
    assert!(store.native_turn_settlement_requires_user_pause(&receipt).await.unwrap());
    assert!(!store.native_turn_settlement_user_pause_committed(&receipt).await.unwrap());
    assert!(store.append_native_settlement_terminal_bundle(&receipt,&runtime,None,Some(&message),Some(&end)).await.is_err());
    proof(&store,&f,&receipt,"late-pause-cleanup-still-writable").await;
    let paused=progress(&f,"atomic-user-pause",json!({"event":"turn_paused","model_steps":1,"reason":"EXECUTION_USER_REQUESTED"}));
    store.append_native_settlement_terminal_bundle(&receipt,&paused,None,None,None).await.unwrap();
    let pause=store.native_pause_state(&f.session.agent_session_id,&"lease-turn".into()).await.unwrap().unwrap();
    assert_eq!(pause.reason,"EXECUTION_USER_REQUESTED");assert!(pause.cleanup_proven);
    assert!(!store.native_turn_settlement_current(&receipt).await.unwrap());
    assert!(store.native_turn_settlement_user_pause_committed(&receipt).await.unwrap());
    store.append_native_settlement_terminal_bundle(&receipt,&paused,None,None,None).await.unwrap();
    assert!(store.pending_native_turn_settlements(0,500).await.unwrap().is_empty());
    let events=store.read_events(&f.session.agent_session_id,None,500).await.unwrap().events;
    assert!(!events.iter().any(|event|matches!(event.kind.0.as_str(),"turn/failed"|"turn/completed"|"turn/resume-authorized")));
}

#[tokio::test]
async fn stop_after_a_committed_pause_can_still_accept_the_original_owner_handoff() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,"settlement-stop-after-pause-ack-loss").await;
    let lease=store.claim_native_execution(claim(&f,"original-owner",0,None)).await.unwrap();
    store.append_native_observation(&lease,&progress(&f,"original-physical-cleanup",json!({"event":"host_cleanup_proven"})),None).await.unwrap();
    store.pause_native_execution(&lease,"EXECUTION_USER_REQUESTED",true).await.unwrap();
    store.cancel_active_turn(&f.session.agent_session_id,"stop-after-paused-commit".into(),"session-api".into()).await.unwrap();
    let original=NativeTurnSettlementRequest {outcome:StrictJsonValue(json!({"model_steps":1,"terminal":{"status":"paused","reason":"EXECUTION_USER_REQUESTED"}})),
        first_cleanup_failure:"the original pause acknowledgement was lost".into()};
    let receipt=store.request_native_turn_settlement(&lease,&original).await.unwrap();
    assert!(store.native_turn_settlement_current(&receipt).await.unwrap());
    store.complete_native_turn_settlement(&receipt).await.unwrap();
    assert_eq!(store.read_turn_receipt(&f.session.agent_session_id,&"lease-turn".into()).await.unwrap().status,TurnReceiptStatus::Cancelled);
    assert!(store.pending_native_turn_settlements(0,500).await.unwrap().is_empty());
}
