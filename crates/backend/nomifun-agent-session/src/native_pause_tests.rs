use super::*;
use crate::{NativeOwnerEvidence, NativeResumePreparation, NativeResumeRequest,
    NativeEffectReconciliationRequest, NativeVerifiedOutcome};

fn evidence() -> NativeOwnerEvidence {
    NativeOwnerEvidence { verified: true, evidence_digest: digest('a'), reference: "fixture-owner-inspection".into() }
}

async fn paused(store: &AgentSessionStore, key: &str) -> (Fixture, NativeExecutionLease, NativeResumeRequest) {
    let f = fixture(store, key).await;
    let lease = store.claim_native_execution(claim(&f, "initial", 0, None)).await.unwrap();
    let cp = checkpoint(store, &f, &lease).await;
    let pause = store.pause_native_execution(&lease, "OWNER_REQUESTED", true).await.unwrap();
    let request = NativeResumeRequest { operation_id: "lease-turn".into(), idempotency_key: "resume-once".into(),
        expected_pause_revision: pause.revision, expected_checkpoint_revision: cp.revision,
        expected_checkpoint_digest: cp.digest, budget: Default::default(), cleanup_attestation: None };
    (f, lease, request)
}

async fn prepare(store: &AgentSessionStore, f: &Fixture) -> NativeResumePreparation {
    let facts = store.native_recovery_facts(&f.session.agent_session_id, &"lease-turn".into()).await.unwrap();
    // The Store tests its transaction/authority contract. Runtime codec tests
    // separately validate the full checkpoint; HTTP never supplies this data.
    NativeResumePreparation { expected_head_seq: facts.head.last_seq, expected_fence: facts.execution_fence,
        snapshot: snapshot_ref(), active_set_generation: 0, observations: vec![],
        checkpoint_state: StrictJsonValue(json!({"version":1,"model_steps":0,"turn_operation_id":"lease-turn",
            "active_set_generation":0,"binding":{"agent_session_id":f.session.agent_session_id,
            "resolved_snapshot_ref":snapshot_ref()}})) }
}

#[tokio::test]
async fn pause_keeps_active_turn_fences_producer_and_cancel_is_terminal() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = fixture(&store, "pause-cancel").await;
    let lease = store.claim_native_execution(claim(&f, "initial", 0, None)).await.unwrap();
    let cp = checkpoint(&store, &f, &lease).await;
    let ack = store.request_native_pause(&owner(), &f.session.agent_session_id, &"lease-turn".into(), "pause", "inspect work").await.unwrap();
    assert_eq!(store.request_native_pause(&owner(), &f.session.agent_session_id, &"lease-turn".into(), "pause", "inspect work").await.unwrap(), ack);
    assert!(store.request_native_pause(&owner(), &f.session.agent_session_id, &"lease-turn".into(), "pause", "changed").await.is_err());
    assert!(store.native_pause_requested(&lease).await.unwrap());
    store.pause_native_execution(&lease, "OWNER_REQUESTED", true).await.unwrap();
    let inspection = store.inspect_latest_native_execution(&owner(), &f.session.agent_session_id).await.unwrap().unwrap();
    assert_eq!(inspection.state, "paused");
    assert_eq!(inspection.turn_state, "running");
    assert!(!inspection.pause_requested && inspection.checkpoint_retained && !inspection.producer_lease_live);
    assert_eq!(store.native_execution_notification_state(&f.session.agent_session_id,&"lease-turn".into(),lease.generation()).await.unwrap().as_deref(),Some("paused"));
    let head = store.head(&f.session.agent_session_id).await.unwrap();
    assert_eq!(head.status, "paused");
    assert_eq!(head.active_turn_id.as_deref(), Some("lease-turn"));
    assert!(store.claim_native_execution(claim(&f, "unauthorized", 1, Some(&cp))).await.is_err());
    assert!(store.pause_native_execution(&lease, "OWNER_REQUESTED", true).await.is_err());
    assert!(store.claim_native_chat_operation(&lease, model(&f, "late-model")).await.is_err());
    let terminal_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind IN ('turn/completed','turn/failed','turn/cancelled')")
        .bind(f.session.agent_session_id.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    assert_eq!(terminal_count, 0);
    store.cancel_active_turn(&f.session.agent_session_id, "cancel".into(), "session-api".into()).await.unwrap();
    assert_eq!(store.read_turn_receipt(&f.session.agent_session_id, &"lease-turn".into()).await.unwrap().status, TurnReceiptStatus::Cancelled);
    assert!(store.native_pause_state(&f.session.agent_session_id, &"lease-turn".into()).await.unwrap().is_none());
    assert!(store.head(&f.session.agent_session_id).await.unwrap().active_turn_id.is_none());
}

#[tokio::test(flavor="multi_thread", worker_threads=2)]
async fn concurrent_same_resume_grants_budget_once_and_reopens_only_new_generation() {
    let store = AgentSessionStore::open_in_memory_with_connections(2).await.unwrap();
    let (f, old, mut request) = paused(&store, "resume-race").await;
    request.budget.additional_journal_mib = 1;
    request.budget.additional_payload_mib = 2;
    let a = prepare(&store, &f).await;
    let b = prepare(&store, &f).await;
    let principal = owner();
    let (a,b) = tokio::join!(store.commit_native_resume(&principal, &f.session.agent_session_id, &request, a),
        store.commit_native_resume(&principal, &f.session.agent_session_id, &request, b));
    let (a,b) = (a.unwrap(), b.unwrap());
    assert_eq!(a.authorization_event_id, b.authorization_event_id);
    assert_ne!(a.duplicate, b.duplicate);
    let inspection = store.inspect_latest_native_execution(&owner(), &f.session.agent_session_id).await.unwrap().unwrap();
    assert_eq!(inspection.budget.revision, 1);
    assert_eq!(inspection.budget.journal_bytes, 17 * 1024 * 1024);
    assert_eq!(inspection.budget.session_payload_bytes, 18 * 1024 * 1024);
    assert_eq!(inspection.checkpoint_revision, request.expected_checkpoint_revision + 1);
    assert!(inspection.execution_generation > old.generation());
    assert!(store.verify_native_execution(&old).await.is_err());
    let cp = store.load_native_checkpoint(&owner(), &f.session.agent_session_id, &request.operation_id).await.unwrap().unwrap();
    let new = store.claim_native_execution(claim(&f, "resumed", inspection.execution_fence, Some(&cp))).await.unwrap();
    assert!(new.generation() > old.generation());
    assert!(store.claim_native_chat_operation(&new, model(&f, "new-generation")).await.is_ok());
    let mut changed = request.clone(); changed.budget.additional_journal_mib = 2;
    assert!(store.native_resume_receipt(&owner(), &f.session.agent_session_id, &changed).await.is_err());
    store.cancel_active_turn(&f.session.agent_session_id, "cancel".into(), "session-api".into()).await.unwrap();
    assert!(store.native_resume_receipt(&owner(), &f.session.agent_session_id, &request).await.unwrap().unwrap().duplicate);
    assert_eq!(store.read_turn_receipt(&f.session.agent_session_id, &request.operation_id).await.unwrap().status, TurnReceiptStatus::Cancelled);
    assert!(store.native_execution_notification_state(&f.session.agent_session_id,&request.operation_id,old.generation()).await.unwrap().is_none());
    assert_eq!(store.native_execution_notification_state(&f.session.agent_session_id,&request.operation_id,new.generation()).await.unwrap().as_deref(),Some("cancelled"));
}

#[tokio::test]
async fn resume_rejects_wrong_owner_stale_identity_and_excess_budget_without_mutation() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (f, _, request) = paused(&store, "resume-invalid").await;
    let before = store.current_cursor(&f.session.agent_session_id).await.unwrap();
    let wrong = PrincipalRef { principal_kind: "user".into(), principal_id: "other".into() };
    assert!(store.commit_native_resume(&wrong, &f.session.agent_session_id, &request, prepare(&store,&f).await).await.is_err());
    for case in 0..7 {
        let mut changed = request.clone();
        let mut prepared = prepare(&store,&f).await;
        match case {
            0 => changed.expected_pause_revision += 1,
            1 => changed.expected_checkpoint_revision += 1,
            2 => changed.expected_checkpoint_digest = digest('f'),
            3 => changed.budget.additional_journal_mib = 17,
            4 => prepared.active_set_generation += 1,
            5 => prepared.snapshot.snapshot_digest = digest('f'),
            _ => prepared.expected_fence += 1,
        }
        assert!(store.commit_native_resume(&owner(), &f.session.agent_session_id, &changed, prepared).await.is_err(), "case {case}");
        assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(), before);
    }
    let prepared = prepare(&store,&f).await;
    store.request_native_pause(&owner(), &f.session.agent_session_id, &request.operation_id, "new-input-boundary", "keep paused").await.unwrap();
    assert!(store.commit_native_resume(&owner(), &f.session.agent_session_id, &request, prepared).await.is_err());
}

#[tokio::test]
async fn uncertain_cleanup_requires_explicit_evidence_and_cancel_cannot_be_reopened() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = fixture(&store, "cleanup-unknown").await;
    let lease = store.claim_native_execution(claim(&f, "initial", 0, None)).await.unwrap();
    let cp = checkpoint(&store, &f, &lease).await;
    store.pause_native_execution(&lease, "CLEANUP_UNKNOWN", false).await.unwrap();
    let mut request = NativeResumeRequest { operation_id: "lease-turn".into(), idempotency_key: "verified-cleanup".into(),
        expected_pause_revision: 1, expected_checkpoint_revision: cp.revision,
        expected_checkpoint_digest: cp.digest, budget: Default::default(), cleanup_attestation: None };
    assert!(store.commit_native_resume(&owner(), &f.session.agent_session_id, &request, prepare(&store,&f).await).await.is_err());
    let mut invalid = evidence(); invalid.verified = false; request.cleanup_attestation = Some(invalid);
    assert!(store.commit_native_resume(&owner(), &f.session.agent_session_id, &request, prepare(&store,&f).await).await.is_err());
    request.cleanup_attestation = Some(evidence());
    let prepared = prepare(&store,&f).await;
    store.cancel_active_turn(&f.session.agent_session_id, "cancel".into(), "session-api".into()).await.unwrap();
    assert!(store.commit_native_resume(&owner(), &f.session.agent_session_id, &request, prepared).await.is_err());
}

#[tokio::test]
async fn pending_effect_attestation_preserves_uncertainty_audit_and_never_reopens_cancelled_turn() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, effect, _) = create_pending_effect(&store, "attestation", EffectStrategy::ManagedEffect).await;
    store.cancel_active_turn(&session.agent_session_id, "cancel".into(), "session-api".into()).await.unwrap();
    let request = NativeEffectReconciliationRequest { operation_id: effect.turn_id.clone(), expected_pause_revision: 0,
        idempotency_key: "owner-verified".into(), effect_id: effect.effect_id.clone(), call_id: None,
        expected_input_digest: effect.input_digest.clone(), outcome: NativeVerifiedOutcome::ConfirmedSucceeded, evidence: evidence() };
    let mut bad = request.clone(); bad.expected_input_digest = digest('e');
    assert!(store.reconcile_native_effect_by_owner(&owner(), &session.agent_session_id, &bad).await.is_err());
    let ack = store.reconcile_native_effect_by_owner(&owner(), &session.agent_session_id, &request).await.unwrap();
    assert_eq!(store.reconcile_native_effect_by_owner(&owner(), &session.agent_session_id, &request).await.unwrap(), ack);
    let settled = store.read_effect(&session.agent_session_id, &effect.effect_id).await.unwrap().unwrap();
    assert_eq!(settled.state, AgentEffectState::Returned);
    let kinds: Vec<String> = sqlx::query_scalar("SELECT kind FROM agent_events WHERE session_id=? AND kind IN ('runtime/effect-reconciliation-attested','effect/uncertain','effect/reconciled') ORDER BY seq")
        .bind(session.agent_session_id.as_ref()).fetch_all(store.test_pool()).await.unwrap();
    assert_eq!(kinds, ["runtime/effect-reconciliation-attested", "effect/uncertain", "effect/reconciled"]);
    assert_eq!(store.read_turn_receipt(&session.agent_session_id, &effect.turn_id).await.unwrap().status, TurnReceiptStatus::Cancelled);
    assert!(store.head(&session.agent_session_id).await.unwrap().active_turn_id.is_none());
}

#[tokio::test(flavor="multi_thread", worker_threads=2)]
async fn actual_owner_and_human_reconciliation_race_keep_one_audited_outcome() {
    let store = AgentSessionStore::open_in_memory_with_connections(2).await.unwrap();
    let (session, effect, _) = create_pending_effect(&store, "owner-race", EffectStrategy::ManagedEffect).await;
    store.quarantine_native_recovery(&owner(), &session.agent_session_id, &effect.turn_id, 0).await.unwrap();
    let unknown = store.read_effect(&session.agent_session_id, &effect.effect_id).await.unwrap().unwrap();
    let request = NativeEffectReconciliationRequest { operation_id:effect.turn_id.clone(),expected_pause_revision:1,
        idempotency_key:"human-race".into(),effect_id:effect.effect_id.clone(),call_id:None,
        expected_input_digest:effect.input_digest.clone(),outcome:NativeVerifiedOutcome::ConfirmedSucceeded,evidence:evidence() };
    let original_owner = EffectEventRequest { event_id:"actual-owner-reconciliation".into(),producer_id:"actual-owner".into(),
        causation_event_id:unknown.terminal_event_id, ..effect.clone() };
    let principal = owner();
    let (human,actual) = tokio::join!(
        store.reconcile_native_effect_by_owner(&principal,&session.agent_session_id,&request),
        store.reconcile_effect(original_owner,EffectReconcileOutcome::ConfirmedSucceeded { receipt:json!({"owner_receipt":"actual-result"}) }));
    assert_ne!(human.is_ok(),actual.is_ok());
    let count:i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='effect/reconciled'")
        .bind(session.agent_session_id.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    assert_eq!(count,1);
    let audits:i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/effect-reconciliation-attested'")
        .bind(session.agent_session_id.as_ref()).fetch_one(store.test_pool()).await.unwrap();
    assert_eq!(audits,i64::from(human.is_ok()));
    assert!(!store.has_unsettled_effects(&session.agent_session_id).await.unwrap());
    assert_eq!(store.head(&session.agent_session_id).await.unwrap().status,"paused");
    assert_eq!(store.read_turn_receipt(&session.agent_session_id,&effect.turn_id).await.unwrap().status,TurnReceiptStatus::Running);
}

#[tokio::test(flavor="multi_thread", worker_threads=2)]
async fn competing_distinct_resume_authorizations_have_one_winner() {
    let store = AgentSessionStore::open_in_memory_with_connections(2).await.unwrap();
    let (f,_,mut first) = paused(&store,"different-resume-race").await;
    first.budget.additional_payload_mib=1;
    let mut second=first.clone(); second.idempotency_key="competing".into();
    let a=prepare(&store,&f).await; let b=prepare(&store,&f).await; let principal=owner();
    let (a,b)=tokio::join!(store.commit_native_resume(&principal,&f.session.agent_session_id,&first,a),
        store.commit_native_resume(&principal,&f.session.agent_session_id,&second,b));
    assert_ne!(a.is_ok(),b.is_ok());
    let budget=store.inspect_latest_native_execution(&owner(),&f.session.agent_session_id).await.unwrap().unwrap().budget;
    assert_eq!(budget.revision,1); assert_eq!(budget.session_payload_bytes,17*1024*1024);
}

#[tokio::test]
async fn payload_allowance_preserves_usage_and_readability_above_the_default_limit() {
    let store=AgentSessionStore::open_in_memory().await.unwrap();
    let f=fixture(&store,"payload-allowance").await;
    let lease=store.claim_native_execution(claim(&f,"initial",0,None)).await.unwrap();
    let cp=checkpoint(&store,&f,&lease).await;
    let content="x".repeat(1024*1024-512);
    let make_part=|index:usize| {
        let value=json!({"content":content});
        let logical=canonical_json_bytes(&value).unwrap();
        let payload=SessionPayloadRecord { payload_id:format!("large-payload-{index}").into(),agent_session_id:f.session.agent_session_id.clone(),
            media_type:"application/json".into(),byte_len:logical.len() as u64,digest:digest_bytes(&logical),body:SessionPayloadBody::Json(StrictJsonValue(value)) };
        let mut event=append(&f.session.agent_session_id,&format!("large-part-{index}"),"runtime_supervisor",&format!("large-part-{index}"),
            "message/content-part",&format!("large-message-{index}"),Some(f.started.clone()),json!({}));
        event.semantic_event.payload=SessionEventPayloadRef::Stored(payload.payload_id.clone());
        (event,payload)
    };
    let mut used=0u64;
    for index in 0..16 {
        let (event,payload)=make_part(index);
        store.append_native_observation(&lease,&event,Some(&payload)).await.unwrap();
        used+=payload.byte_len;
    }
    assert_eq!(store.native_payload_bytes(&lease).await.unwrap(),used);
    let (event,payload)=make_part(16);
    let cursor=store.current_cursor(&f.session.agent_session_id).await.unwrap();
    assert!(matches!(store.append_native_observation(&lease,&event,Some(&payload)).await,
        Err(SessionStoreError::InvalidPayload(message)) if message.contains("session payload budget")));
    assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(),cursor);
    assert_eq!(store.native_payload_bytes(&lease).await.unwrap(),used);
    store.pause_native_execution(&lease,"EXECUTION_SESSION_PAYLOAD_BUDGET",true).await.unwrap();
    let request=NativeResumeRequest { operation_id:"lease-turn".into(),idempotency_key:"approve-payload".into(),expected_pause_revision:1,
        expected_checkpoint_revision:cp.revision,expected_checkpoint_digest:cp.digest,
        budget:nomifun_agent_contracts::NativeBudgetIncrease { additional_payload_mib:8,..Default::default() },cleanup_attestation:None };
    store.commit_native_resume(&owner(),&f.session.agent_session_id,&request,prepare(&store,&f).await).await.unwrap();
    let saved=store.load_native_checkpoint(&owner(),&f.session.agent_session_id,&request.operation_id).await.unwrap().unwrap();
    let new=store.claim_native_execution(claim(&f,"resumed",saved.execution_fence,Some(&saved))).await.unwrap();
    let appended=store.append_native_observation(&new,&event,Some(&payload)).await.unwrap();
    let replay=store.append_native_observation(&new,&event,Some(&payload)).await.unwrap();
    assert!(replay.duplicate); assert_eq!(replay.cursor,appended.cursor);
    assert_eq!(store.native_payload_bytes(&new).await.unwrap(),used+payload.byte_len);
    let facts=store.native_recovery_facts(&f.session.agent_session_id,&request.operation_id).await.unwrap();
    assert_eq!(facts.event_payloads["large-part-0"]["content"].as_str().unwrap(),content);
    assert_eq!(facts.event_payloads["large-part-16"]["content"].as_str().unwrap(),content);
    assert_eq!(store.native_execution_budget(&new).await.unwrap().session_payload_bytes,24*1024*1024);
}

#[tokio::test]
async fn constructed_multi_day_pause_survives_reopen_without_automatic_authority() {
    let root=tempfile::tempdir().unwrap();
    let path=root.path().join("multi-day-pause.db");
    let database=nomifun_db::init_database(&path).await.unwrap();
    let store=AgentSessionStore::from_pool(database.pool().clone()).await.unwrap();
    let f=fixture(&store,"multi-day").await;
    let old=store.claim_native_execution(claim(&f,"before-restart",0,None)).await.unwrap();
    let cp=checkpoint(&store,&f,&old).await;
    // Seed a consistent historical pause event through the canonical fixture writer.
    // This models elapsed days without sleeping or exposing a production
    // clock override, and never rewrites existing audit facts.
    let old_time=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64 - 3*24*60*60*1000;
    let pause=crate::NativePauseState { revision:1,reason:"EXECUTION_USER_REQUESTED".into(),checkpoint_revision:cp.revision,
        checkpoint_digest:Some(cp.digest.clone()),execution_fence:old.fence()+1,cleanup_proven:true,paused_at_ms:old_time };
    let paused_event=append(&f.session.agent_session_id,
        &format!("native-paused:{}:lease-turn:1",f.session.agent_session_id.as_ref()),"runtime_supervisor","historical-pause",
        "turn/paused","lease-turn",Some(f.started.clone()),json!({"pause":pause}));
    assert!(store.append_native_event(&old,&paused_event,None).await.is_err(),
        "the ordinary native event port cannot mint owner control events");
    store.append_event(&paused_event).await.unwrap();
    let cursor=store.current_cursor(&f.session.agent_session_id).await.unwrap();
    drop(store); database.close().await;
    let reopened=AgentSessionStore::connect_existing(&path).await.unwrap();
    let inspection=reopened.inspect_latest_native_execution(&owner(),&f.session.agent_session_id).await.unwrap().unwrap();
    assert_eq!(inspection.state,"paused"); assert_eq!(inspection.turn_state,"running");
    assert_eq!(inspection.pause.unwrap().paused_at_ms,old_time);
    assert_eq!(reopened.current_cursor(&f.session.agent_session_id).await.unwrap(),cursor);
    assert!(reopened.claim_native_execution(claim(&f,"unauthorized-after-days",1,Some(&cp))).await.is_err());
    assert!(reopened.verify_native_execution(&old).await.is_err());
    let request=NativeResumeRequest { operation_id:"lease-turn".into(),idempotency_key:"explicit-after-days".into(),expected_pause_revision:1,
        expected_checkpoint_revision:cp.revision,expected_checkpoint_digest:cp.digest,budget:Default::default(),cleanup_attestation:None };
    reopened.commit_native_resume(&owner(),&f.session.agent_session_id,&request,prepare(&reopened,&f).await).await.unwrap();
    let saved=reopened.load_native_checkpoint(&owner(),&f.session.agent_session_id,&request.operation_id).await.unwrap().unwrap();
    let new=reopened.claim_native_execution(claim(&f,"authorized-after-days",saved.execution_fence,Some(&saved))).await.unwrap();
    assert!(new.generation()>old.generation());
    let roots:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/started'")
        .bind(f.session.agent_session_id.as_ref()).fetch_one(reopened.test_pool()).await.unwrap();
    assert_eq!(roots,1,"elapsed days and restart cannot create another task");
    reopened.test_pool().close().await;
}
