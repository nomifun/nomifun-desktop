use super::*;
use crate::MAX_INLINE_JSON_BYTES;

#[tokio::test]
async fn technical_settlement_never_turns_unknown_business_effect_into_a_failed_or_successful_result() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,"settlement-unknown").await;
    let lease=store.claim_native_execution(claim(&f,"original-owner",0,None)).await.unwrap();
    unknown_effect(&store,&f,&lease).await;
    let effects=store.list_effects(&f.session.agent_session_id).await.unwrap();
    let receipt=store.request_native_turn_settlement(&lease,&crate::NativeTurnSettlementRequest {
        outcome:StrictJsonValue(json!({"model_steps":1,"terminal":{"status":"failed","message":"original failure"}})),
        first_cleanup_failure:"physical cleanup initially unavailable".into(),
    }).await.unwrap();
    let mut cleanup=progress(&f,"settlement-physical-cleanup",json!({"event":"host_cleanup_proven"}));
    cleanup.semantic_event.causation_event_id=Some(f.root.clone());
    store.append_native_settlement_observation(&receipt,&cleanup,None).await.unwrap();
    let message=append(&f.session.agent_session_id,"settlement-empty-message","runtime_supervisor","settlement-empty-message","message/completed","empty",Some(f.root.clone()),json!({"part_count":0,"content_digest":digest_bytes(b"")}));
    let failed=append(&f.session.agent_session_id,"settlement-must-not-fail","runtime_supervisor","settlement-must-not-fail","turn/failed","lease-turn",Some(message.event_id.clone()),json!({"model_steps":1,"message":"original failure"}));
    let runtime=progress(&f,"unknown-settlement-runtime",json!({"event":"turn_failed","model_steps":1,"message":"original failure"}));
    assert!(matches!(store.append_native_settlement_terminal_bundle(&receipt,&runtime,None,Some(&message),Some(&failed)).await,Err(SessionStoreError::RecoveryRequiresReconciliation)));
    assert_eq!(store.list_effects(&f.session.agent_session_id).await.unwrap(),effects);
    assert!(store.claim_native_chat_operation(&lease,model(&f,"unknown-never-replays")).await.is_err());
    assert!(store.complete_native_turn_settlement(&receipt).await.is_err());
}

fn cancellation(f:&Fixture,key:&str)->SessionEventAppend {
    append(&f.session.agent_session_id,key,"runtime_supervisor",key,"turn/cancelled","lease-turn",Some(f.started.clone()),
        json!({"operation_id":"lease-turn"}))
}

async fn cleanup(store:&AgentSessionStore,f:&Fixture,lease:&NativeExecutionLease,key:&str,stored:bool) {
    let value=json!({"producer_seq":1,"event":{"event":"host_cleanup_proven","padding":"x".repeat(if stored {MAX_INLINE_JSON_BYTES+256}else {0})}});
    let mut proof=progress(f,key,json!({"event":"host_cleanup_proven"}));
    // Real journals use the accepted input as their native-progress cause.
    proof.semantic_event.causation_event_id=Some(f.root.clone());
    let payload=stored.then(|| {
        let bytes=canonical_json_bytes(&value).unwrap();
        let payload=SessionPayloadRecord {payload_id:format!("payload-{key}").into(),agent_session_id:f.session.agent_session_id.clone(),
            media_type:"application/json".into(),byte_len:bytes.len() as u64,digest:digest_bytes(&bytes),body:SessionPayloadBody::Json(StrictJsonValue(value))};
        proof.semantic_event.payload=SessionEventPayloadRef::Stored(payload.payload_id.clone());payload
    });
    store.append_native_observation(lease,&proof,payload.as_ref()).await.unwrap();
}

async fn unknown_effect(store:&AgentSessionStore,f:&Fixture,lease:&NativeExecutionLease) {
    let tool=append(&f.session.agent_session_id,"unknown-mcp-call","runtime_supervisor","unknown-mcp-call","tool/call-started","unknown-call",Some(f.started.clone()),
        json!({"operation_id":"unknown-operation","call_id":"unknown-call","capability_id":"fixture.mcp","action_id":"fixture.mcp/invoke"}));
    let tool=store.append_native_event(lease,&tool,None).await.unwrap().ack.unwrap().event_id;
    let effect=EffectEventRequest {agent_session_id:f.session.agent_session_id.clone(),effect_id:"unknown-mcp".into(),turn_id:"lease-turn".into(),operation_id:"unknown-operation".into(),
        owner_domain:"mcp".into(),capability_module:"fixture.mcp".into(),action_id:"fixture.mcp/invoke".into(),resource_binding_id:None,resource_key:None,input_digest:digest('7'),
        recorded_at:1_788_000_000_010,event_id:"unknown-mcp-started".into(),producer_id:"capability_host".into(),idempotency_key:"unknown-mcp".into(),correlation_id:"unknown-mcp".into(),
        strategy:EffectStrategy::ExternalUncertainEffect,causation_event_id:Some(tool),payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({})))};
    let started=store.record_effect_started(effect.clone()).await.unwrap().ack.unwrap().event_id;
    store.record_effect_terminal(EffectEventRequest {event_id:"unknown-mcp-returned".into(),producer_id:"owning_plugin".into(),causation_event_id:Some(started),
        payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"execution":{"request_dispatched":true,"local_process_started":true,"returned":false},
            "cleanup":{"local":"confirmed","remote":"not_required","error":null}}))),..effect},EffectTerminalState::Uncertain).await.unwrap();
}

#[tokio::test]
async fn original_cleanup_cancels_reconciliation_turn_without_settling_unknown_or_reopening_work() {
    for stored in [false,true] {
        let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,"cleanup-unknown").await;
        let lease=store.claim_native_execution(claim(&f,"original-owner",0,None)).await.unwrap();
        unknown_effect(&store,&f,&lease).await;
        assert_eq!(store.head(&f.session.agent_session_id).await.unwrap().status,"reconciliation");
        let effects=store.list_effects(&f.session.agent_session_id).await.unwrap();
        assert!(matches!(store.append_native_event(&lease,&progress(&f,"no-new-work",json!({"event":"work_status"})),None).await,Err(SessionStoreError::ExecutionFenced)));
        assert!(matches!(store.claim_native_chat_operation(&lease,model(&f,"no-new-model")).await,Err(SessionStoreError::ExecutionFenced)));
        cleanup(&store,&f,&lease,"original-cleanup",stored).await;
        let cancel=cancellation(&f,"original-native-cancel");
        // The generic producer and observation entrypoints stay strict.
        assert!(matches!(store.append_native_event(&lease,&cancel,None).await,Err(SessionStoreError::ExecutionFenced)));
        assert!(matches!(store.append_native_observation(&lease,&cancel,None).await,Err(SessionStoreError::ExecutionFenced)));
        let first=store.append_native_cancellation_after_cleanup(&lease,&cancel).await.unwrap();
        let repeated=store.append_native_cancellation_after_cleanup(&lease,&cancel).await.unwrap();
        assert!(repeated.duplicate);assert_eq!(first.ack,repeated.ack);
        assert_eq!(store.read_turn_receipt(&f.session.agent_session_id,&"lease-turn".into()).await.unwrap().status,TurnReceiptStatus::Cancelled);
        assert_eq!(store.list_effects(&f.session.agent_session_id).await.unwrap(),effects);
        assert!(store.has_unsettled_effects(&f.session.agent_session_id).await.unwrap());
        assert!(matches!(store.append_native_event(&lease,&progress(&f,"late-work",json!({"event":"work_status"})),None).await,Err(SessionStoreError::ExecutionFenced)));
        assert!(matches!(store.claim_native_chat_operation(&lease,model(&f,"late-model")).await,Err(SessionStoreError::ExecutionFenced)));
        let facts=store.read_events(&f.session.agent_session_id,None,500).await.unwrap();
        assert_eq!(facts.events.iter().filter(|event|event.kind.0=="turn/cancelled").count(),1);
        assert!(!facts.events.iter().any(|event|matches!(event.kind.0.as_str(),"effect/reconciled"|"runtime/effect-reconciliation-attested"|"turn/resume-authorized")));
    }
}

#[tokio::test]
async fn native_cancellation_requires_exact_scope_current_producer_and_its_cleanup_proof() {
    for changed in ["session","producer","kind","version","correlation","payload","extra_payload","stored_cancel","cause","missing_proof",
        "proof_producer","proof_turn","proof_cause","corrupt_stored_proof","holder","fence","generation","head_status","head_turn","completed"] {
        let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,"cleanup-scope").await;
        let lease=store.claim_native_execution(claim(&f,"original-owner",0,None)).await.unwrap();
        if changed!="missing_proof" {cleanup(&store,&f,&lease,"original-cleanup",changed=="corrupt_stored_proof").await;}
        let sid=&f.session.agent_session_id;let mut cancel=cancellation(&f,"native-cancel");
        match changed {
            "session"=>cancel.agent_session_id=Uuid::now_v7().to_string().into(),
            "producer"=>cancel.producer_id="session_api".into(),
            "kind"=>cancel.semantic_event.kind.0="turn/completed".into(),
            "version"=>cancel.semantic_event.kind_version=2,
            "correlation"=>cancel.semantic_event.correlation_id="successor-turn".into(),
            "payload"=>cancel.semantic_event.payload=SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"operation_id":"successor-turn"}))),
            "extra_payload"=>cancel.semantic_event.payload=SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"operation_id":"lease-turn","reconciled":true}))),
            "stored_cancel"=>cancel.semantic_event.payload=SessionEventPayloadRef::Stored("not-a-cancellation-payload".into()),
            "cause"=>cancel.semantic_event.causation_event_id=Some(f.root.clone()),
            "proof_producer"=>{sqlx::query("UPDATE agent_events SET producer_id='another_owner' WHERE session_id=? AND event_id='original-cleanup'").bind(sid.as_ref()).execute(store.test_pool()).await.unwrap();},
            "proof_turn"=>{sqlx::query("UPDATE agent_events SET correlation_id='another-turn' WHERE session_id=? AND event_id='original-cleanup'").bind(sid.as_ref()).execute(store.test_pool()).await.unwrap();},
            "proof_cause"=>{sqlx::query("UPDATE agent_events SET causation_event_id='event-ready-cleanup-scope' WHERE session_id=? AND event_id='original-cleanup'").bind(sid.as_ref()).execute(store.test_pool()).await.unwrap();},
            "corrupt_stored_proof"=>{sqlx::query("UPDATE agent_payloads SET digest=? WHERE payload_id='payload-original-cleanup'").bind(digest('d').as_ref()).execute(store.test_pool()).await.unwrap();},
            "holder"=>{sqlx::query("UPDATE agent_turns SET execution_owner='replacement-owner' WHERE session_id=?").bind(sid.as_ref()).execute(store.test_pool()).await.unwrap();},
            "fence"=>{sqlx::query("UPDATE agent_turns SET execution_fence=execution_fence+1 WHERE session_id=?").bind(sid.as_ref()).execute(store.test_pool()).await.unwrap();},
            "generation"=>{sqlx::query("UPDATE agent_turns SET execution_generation=execution_generation+1 WHERE session_id=?").bind(sid.as_ref()).execute(store.test_pool()).await.unwrap();},
            "head_status"=>{sqlx::query("UPDATE agent_session_heads SET status='paused' WHERE session_id=?").bind(sid.as_ref()).execute(store.test_pool()).await.unwrap();},
            "head_turn"=>{sqlx::query("UPDATE agent_session_heads SET active_turn_id='successor-turn' WHERE session_id=?").bind(sid.as_ref()).execute(store.test_pool()).await.unwrap();},
            "completed"=>{store.append_native_event(&lease,&append(sid,"canonical-failed","runtime_supervisor","canonical-failed","turn/cancelled","lease-turn",Some(f.started.clone()),json!({"operation_id":"lease-turn"})),None).await.unwrap();
                sqlx::query("UPDATE agent_turns SET state='completed' WHERE session_id=?").bind(sid.as_ref()).execute(store.test_pool()).await.unwrap();},
            _=>{},
        }
        let before=store.current_cursor(sid).await.unwrap();
        assert!(store.append_native_cancellation_after_cleanup(&lease,&cancel).await.is_err(),"{changed}");
        assert_eq!(store.current_cursor(sid).await.unwrap(),before,"{changed}");
    }
}

#[tokio::test]
async fn resumed_or_paused_producer_cannot_borrow_old_cleanup_and_unknown_still_blocks_resume() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,"cleanup-recovery").await;
    let old=store.claim_native_execution(claim(&f,"old-owner",0,None)).await.unwrap();
    cleanup(&store,&f,&old,"old-cleanup",false).await;let cp=checkpoint(&store,&f,&old).await;expire(&store,&f).await;
    let fresh=store.claim_native_execution(claim(&f,"new-owner",0,Some(&cp))).await.unwrap();
    let cancel=cancellation(&f,"recovered-cancel");let before=store.current_cursor(&f.session.agent_session_id).await.unwrap();
    assert!(matches!(store.append_native_cancellation_after_cleanup(&old,&cancel).await,Err(SessionStoreError::ExecutionFenced)));
    assert!(matches!(store.append_native_cancellation_after_cleanup(&fresh,&cancel).await,Err(SessionStoreError::ExecutionFenced)));
    assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(),before);
    cleanup(&store,&f,&fresh,"fresh-cleanup",true).await;
    store.append_native_cancellation_after_cleanup(&fresh,&cancel).await.unwrap();

    let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,"cleanup-paused").await;
    let lease=store.claim_native_execution(claim(&f,"paused-owner",0,None)).await.unwrap();let cp=checkpoint(&store,&f,&lease).await;
    unknown_effect(&store,&f,&lease).await;cleanup(&store,&f,&lease,"paused-cleanup",false).await;
    let effects=store.list_effects(&f.session.agent_session_id).await.unwrap();
    let pause=store.pause_native_execution(&lease,"NATIVE_RECOVERY_RECONCILIATION_REQUIRED",true).await.unwrap();
    assert!(matches!(store.append_native_cancellation_after_cleanup(&lease,&cancellation(&f,"paused-cancel")).await,Err(SessionStoreError::ExecutionFenced)));
    let head=store.head(&f.session.agent_session_id).await.unwrap();
    let result=store.commit_native_resume(&owner(),&f.session.agent_session_id,&crate::NativeResumeRequest {operation_id:"lease-turn".into(),
        idempotency_key:"no-unknown-resume".into(),expected_pause_revision:pause.revision,expected_checkpoint_revision:cp.revision,
        expected_checkpoint_digest:cp.digest.clone(),budget:Default::default(),cleanup_attestation:None},crate::NativeResumePreparation {
            expected_head_seq:head.last_seq,expected_fence:pause.execution_fence,snapshot:snapshot_ref(),active_set_generation:0,
            checkpoint_state:cp.state,observations:vec![]}).await;
    assert!(matches!(result,Err(SessionStoreError::RecoveryRequiresReconciliation)),"{result:?}");
    assert_eq!(store.list_effects(&f.session.agent_session_id).await.unwrap(),effects);
}

#[tokio::test]
async fn original_cancellation_writer_failure_and_concurrent_stop_reuse_one_terminal() {
    let store=AgentSessionStore::open_in_memory_with_connections(2).await.unwrap();let f=fixture(&store,"cleanup-writer-retry").await;
    let lease=store.claim_native_execution(claim(&f,"original-owner",0,None)).await.unwrap();unknown_effect(&store,&f,&lease).await;
    cleanup(&store,&f,&lease,"original-cleanup",false).await;
    let cancel=cancellation(&f,"original-native-cancel");let before=store.current_cursor(&f.session.agent_session_id).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_native_cancel BEFORE INSERT ON agent_events WHEN NEW.kind='turn/cancelled' BEGIN SELECT RAISE(FAIL,'fixture cancellation writer failure'); END").execute(store.test_pool()).await.unwrap();
    assert!(store.append_native_cancellation_after_cleanup(&lease,&cancel).await.is_err());assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(),before);
    sqlx::query("DROP TRIGGER reject_native_cancel").execute(store.test_pool()).await.unwrap();
    let target:OperationId="lease-turn".into();
    let (native,ordinary)=tokio::join!(store.append_native_cancellation_after_cleanup(&lease,&cancel),
        store.cancel_exact_turn(&f.session.agent_session_id,&target,"concurrent-user-stop".into(),"session_api".into()));
    let native=native.unwrap();let ordinary=ordinary.unwrap().1;assert_eq!(native.ack,ordinary.ack);
    let repeated=store.append_native_cancellation_after_cleanup(&lease,&cancel).await.unwrap();assert_eq!(repeated.ack,native.ack);assert!(repeated.duplicate);
    let mut changed=cancel.clone();changed.semantic_event.causation_event_id=Some(f.root.clone());
    assert!(store.append_native_cancellation_after_cleanup(&lease,&changed).await.is_err());
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/cancelled'").bind(f.session.agent_session_id.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    assert_eq!(count,1);assert!(store.has_unsettled_effects(&f.session.agent_session_id).await.unwrap());
}

#[tokio::test]
async fn original_cancel_ack_never_retargets_successor_in_either_stop_order() {
    for user_first in [false,true] {
        let store=AgentSessionStore::open_in_memory().await.unwrap();let f=fixture(&store,"cleanup-successor").await;
        let lease=store.claim_native_execution(claim(&f,"original-owner",0,None)).await.unwrap();
        cleanup(&store,&f,&lease,"original-cleanup",false).await;
        let cancel=cancellation(&f,"original-native-cancel");
        let first=if user_first {store.cancel_exact_turn(&f.session.agent_session_id,&"lease-turn".into(),"original-user-stop".into(),"session_api".into()).await.unwrap().1}
            else {store.append_native_cancellation_after_cleanup(&lease,&cancel).await.unwrap()};
        assert_eq!(store.append_native_cancellation_after_cleanup(&lease,&cancel).await.unwrap().ack,first.ack);
        store.start_turn(&f.session.agent_session_id,"session_api".into(),"successor-input".into(),"successor-turn".into(),
            StrictJsonValue(json!({"content":"new independent work"}))).await.unwrap();
        let successor=store.claim_native_execution(NativeExecutionClaim {operation_id:"successor-turn".into(),holder:"successor-owner".into(),..claim(&f,"unused",0,None)}).await.unwrap();
        let before=store.current_cursor(&f.session.agent_session_id).await.unwrap();
        assert_eq!(store.append_native_cancellation_after_cleanup(&lease,&cancel).await.unwrap().ack,first.ack);
        let mut retargeted=cancel.clone();retargeted.semantic_event.correlation_id="successor-turn".into();
        retargeted.semantic_event.payload=SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"operation_id":"successor-turn"})));
        assert!(matches!(store.append_native_cancellation_after_cleanup(&lease,&retargeted).await,Err(SessionStoreError::ExecutionFenced)));
        assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(),before);
        assert_eq!(store.head(&f.session.agent_session_id).await.unwrap().active_turn_id.as_deref(),Some("successor-turn"));
        store.verify_native_execution(&successor).await.unwrap();
        sqlx::query("UPDATE agent_turns SET execution_fence=execution_fence+1 WHERE session_id=? AND operation_id='lease-turn'")
            .bind(f.session.agent_session_id.as_ref()).execute(store.test_pool()).await.unwrap();
        assert!(matches!(store.append_native_cancellation_after_cleanup(&lease,&cancel).await,Err(SessionStoreError::ExecutionFenced)),"even a duplicate ACK cannot borrow a replaced fence");
        assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(),before);
    }
}
