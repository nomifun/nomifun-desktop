use super::*;
use crate::{CommittedNativeToolCallCausation, NativeResumePreparation, NativeResumeRequest};

fn workspace_tool(f: &Fixture, key: &str, operation: &str) -> SessionEventAppend {
    append(
        &f.session.agent_session_id, key, "runtime_supervisor", key,
        "tool/call-started", key, Some(f.started.clone()),
        json!({"operation_id":operation,"call_id":key,"capability_id":"workspace.files","action_id":"workspace.files/read"}),
    )
}

async fn native_cause(
    store: &AgentSessionStore, f: &Fixture, operation: &str,
) -> Result<CommittedNativeToolCallCausation, SessionStoreError> {
    store.native_tool_call_causation(
        &owner(), &f.session.agent_session_id, &"lease-turn".into(),
        &operation.into(), &"workspace.files".into(), &"workspace.files/read".into(),
    ).await
}

#[tokio::test]
async fn committed_native_tool_causation_is_exact_per_call_and_keeps_first_zero_fence() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = fixture(&store, "exact-native-causation").await;
    let lease = store.claim_native_execution(claim(&f, "initial", 0, None)).await.unwrap();
    assert_eq!(lease.fence(), 0);
    assert!(lease.generation() > 0);
    let first = store.append_native_event(
        &lease, &workspace_tool(&f, "observe-first", "workspace-call-first"), None,
    ).await.unwrap().ack.unwrap();
    let second = store.append_native_event(
        &lease, &workspace_tool(&f, "observe-second", "workspace-call-second"), None,
    ).await.unwrap().ack.unwrap();
    store.append_native_observation(
        &lease, &progress(&f, "unrelated-progress", json!({"event":"host_cleanup_proven"})), None,
    ).await.unwrap();
    let before = store.current_cursor(&f.session.agent_session_id).await.unwrap();
    assert!(before.seq > second.seq && second.seq > first.seq);

    for (operation, ack) in [("workspace-call-first", &first), ("workspace-call-second", &second)] {
        let cause = native_cause(&store, &f, operation).await.unwrap();
        assert_eq!(cause.tool_call.event_id, ack.event_id);
        assert_eq!(cause.tool_call.seq, ack.seq);
        assert_eq!(cause.execution_scope.generation(), lease.generation());
        assert_eq!(cause.execution_scope.fence(), lease.fence());
        assert_eq!(cause.execution_scope.resolved_snapshot_ref(), &snapshot_ref());
        assert_eq!(store.effect_causation_event_id(
            &f.session.agent_session_id, &"lease-turn".into(), &operation.into(),
            &"workspace.files".into(), &"workspace.files/read".into(),
        ).await.unwrap(), ack.event_id);
    }
    assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(), before);
}

#[tokio::test]
async fn committed_native_tool_causation_rejects_missing_wrong_identity_and_ambiguity() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = fixture(&store, "causation-identity").await;
    let other = fixture(&store, "causation-other-session").await;
    let lease = store.claim_native_execution(claim(&f, "initial", 0, None)).await.unwrap();
    store.append_native_event(
        &lease, &workspace_tool(&f, "identity-call", "exact-call"), None,
    ).await.unwrap();
    let before = store.current_cursor(&f.session.agent_session_id).await.unwrap();
    assert!(matches!(native_cause(&store, &f, "missing-call").await,
        Err(SessionStoreError::InvalidEvent(message)) if message.contains("exact committed")));
    for (session, turn, capability, action) in [
        (&other.session.agent_session_id, "lease-turn", "workspace.files", "workspace.files/read"),
        (&f.session.agent_session_id, "different-turn", "workspace.files", "workspace.files/read"),
        (&f.session.agent_session_id, "lease-turn", "browser", "workspace.files/read"),
        (&f.session.agent_session_id, "lease-turn", "workspace.files", "workspace.files/write"),
    ] {
        assert!(store.tool_call_causation(
            session, &turn.into(), &"exact-call".into(), &capability.into(), &action.into(),
        ).await.is_err());
        assert!(store.native_tool_call_causation(
            &owner(), session, &turn.into(), &"exact-call".into(), &capability.into(), &action.into(),
        ).await.is_err());
    }
    let foreign = PrincipalRef { principal_kind:"user".into(), principal_id:"foreign-owner".into() };
    assert!(matches!(store.native_tool_call_causation(
        &foreign, &f.session.agent_session_id, &"lease-turn".into(), &"exact-call".into(),
        &"workspace.files".into(), &"workspace.files/read".into(),
    ).await, Err(SessionStoreError::ExecutionFenced)));
    assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(), before);

    // The public canonical writer allows distinct events with the same exact
    // invocation identity. A query must reject this ambiguity, not choose one.
    store.append_native_event(
        &lease, &workspace_tool(&f, "identity-duplicate", "exact-call"), None,
    ).await.unwrap();
    let before = store.current_cursor(&f.session.agent_session_id).await.unwrap();
    assert!(matches!(native_cause(&store, &f, "exact-call").await,
        Err(SessionStoreError::InvalidEvent(message)) if message.contains("ambiguous")));
    assert!(matches!(store.effect_causation_event_id(
        &f.session.agent_session_id, &"lease-turn".into(), &"exact-call".into(),
        &"workspace.files".into(), &"workspace.files/read".into(),
    ).await, Err(SessionStoreError::InvalidEvent(message)) if message.contains("ambiguous")));
    assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(), before);
}

#[tokio::test]
async fn committed_native_tool_causation_never_borrows_unclaimed_started_at() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = fixture(&store, "unclaimed-native-causation").await;
    let tool = store.append_event(&workspace_tool(&f, "unclaimed-call", "unclaimed-operation"))
        .await.unwrap().ack.unwrap();
    // Non-native Effect identity remains usable; that is not a native lease.
    assert_eq!(store.tool_call_causation(
        &f.session.agent_session_id, &"lease-turn".into(), &"unclaimed-operation".into(),
        &"workspace.files".into(), &"workspace.files/read".into(),
    ).await.unwrap().seq, tool.seq);
    assert_eq!(store.effect_causation_event_id(
        &f.session.agent_session_id, &"lease-turn".into(), &"unclaimed-operation".into(),
        &"workspace.files".into(), &"workspace.files/read".into(),
    ).await.unwrap(), tool.event_id);
    let before = store.current_cursor(&f.session.agent_session_id).await.unwrap();
    assert!(matches!(native_cause(&store, &f, "unclaimed-operation").await,
        Err(SessionStoreError::ExecutionFenced)));
    assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(), before);
}

#[tokio::test]
async fn committed_native_tool_causation_rejects_completed_call_without_revoking_other_calls() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = fixture(&store, "completed-native-causation").await;
    let lease = store.claim_native_execution(claim(&f, "initial", 0, None)).await.unwrap();
    let completed_tool = store.append_native_event(
        &lease, &workspace_tool(&f, "completed-call", "completed-operation"), None,
    ).await.unwrap().ack.unwrap();
    let active_tool = store.append_native_event(
        &lease, &workspace_tool(&f, "active-call", "active-operation"), None,
    ).await.unwrap().ack.unwrap();
    let result = append(
        &f.session.agent_session_id, "completed-result", "runtime_supervisor", "completed-result",
        "tool/result-recorded", "completed-call", Some(completed_tool.event_id.clone()),
        json!({"operation_id":"completed-operation","call_id":"completed-call","output":{"generation":completed_tool.seq},"error":null}),
    );
    store.append_native_observation(&lease, &result, None).await.unwrap();
    let before = store.current_cursor(&f.session.agent_session_id).await.unwrap();
    assert!(matches!(native_cause(&store, &f, "completed-operation").await,
        Err(SessionStoreError::ExecutionFenced)));
    assert_eq!(native_cause(&store, &f, "active-operation").await.unwrap().tool_call.seq, active_tool.seq);
    // Effect causation still identifies the original admitted call after its
    // tool result; this new guard is specifically native observation authority.
    assert_eq!(store.effect_causation_event_id(
        &f.session.agent_session_id, &"lease-turn".into(), &"completed-operation".into(),
        &"workspace.files".into(), &"workspace.files/read".into(),
    ).await.unwrap(), completed_tool.event_id);
    assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(), before);
}

#[tokio::test]
async fn committed_native_tool_causation_rejects_exact_settlement_before_result_projection() {
    for stored in [false, true] {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = fixture(&store, "settled-native-causation").await;
    let lease = store.claim_native_execution(claim(&f, "initial", 0, None)).await.unwrap();
    store.append_native_event(
        &lease, &workspace_tool(&f, "settled-call", "settled-operation"), None,
    ).await.unwrap();
    let active = store.append_native_event(
        &lease, &workspace_tool(&f, "still-active-call", "still-active-operation"), None,
    ).await.unwrap().ack.unwrap();
    let mut settlement = progress(&f, "exact-host-settlement", json!({
        "event":"host_tool_settled","operation_id":"settled-operation","call_id":"settled-call",
        "result":{"generation":1},"error":null,
    }));
    // Actual native progress cites the accepted input root; tool projections
    // cite the Turn-start event. No tool-result projection is appended here.
    settlement.semantic_event.causation_event_id = Some(f.root.clone());
    let payload = if stored {
        let SessionEventPayloadRef::InlineJson(value) = settlement.semantic_event.payload.clone() else { unreachable!() };
        let bytes = canonical_json_bytes(&value.0).unwrap();
        let payload = SessionPayloadRecord {
            agent_session_id:f.session.agent_session_id.clone(), payload_id:"exact-settlement-payload".into(),
            media_type:"application/json".into(), byte_len:bytes.len() as u64, digest:digest_bytes(&bytes),
            body:SessionPayloadBody::Json(value),
        };
        settlement.semantic_event.payload = SessionEventPayloadRef::Stored(payload.payload_id.clone());
        Some(payload)
    } else { None };
    store.append_native_observation(&lease, &settlement, payload.as_ref()).await.unwrap();
    let before = store.current_cursor(&f.session.agent_session_id).await.unwrap();
    assert!(matches!(native_cause(&store, &f, "settled-operation").await,
        Err(SessionStoreError::ExecutionFenced)));
    assert_eq!(native_cause(&store, &f, "still-active-operation").await.unwrap().tool_call.seq, active.seq);
    assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(), before);
    }
}

#[tokio::test]
async fn committed_native_tool_causation_requires_live_lease_and_exact_generation_fact() {
    for fault in [
        "execution_generation=0",
        "execution_generation=execution_generation+1",
        "execution_fence=execution_fence+1",
        "execution_owner=NULL",
        "execution_owner=''",
        "execution_owner='different-live-holder'",
        "execution_lease_until=0",
    ] {
        let store = AgentSessionStore::open_in_memory().await.unwrap();
        let f = fixture(&store, fault).await;
        let lease = store.claim_native_execution(claim(&f, "initial", 0, None)).await.unwrap();
        store.append_native_event(
            &lease, &workspace_tool(&f, "current-call", "current-operation"), None,
        ).await.unwrap();
        native_cause(&store, &f, "current-operation").await.unwrap();
        // Deterministic corruption/expiry injection in this isolated database.
        // Production does not expose setters for the lease or clock.
        sqlx::query(&format!("UPDATE agent_turns SET {fault} WHERE session_id=?"))
            .bind(f.session.agent_session_id.as_ref()).execute(store.test_pool()).await.unwrap();
        let before = store.current_cursor(&f.session.agent_session_id).await.unwrap();
        assert!(matches!(native_cause(&store, &f, "current-operation").await,
            Err(SessionStoreError::ExecutionFenced)), "fault must revoke native scope: {fault}");
        assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(), before);
    }
}

#[tokio::test]
async fn committed_native_tool_causation_requires_typed_exact_claim_payload() {
    for fault in [
        "inline_json=json_set(inline_json,'$.execution_fence',json('false'))",
        "inline_json=json_set(inline_json,'$.execution_fence',0.0)",
        "inline_json=json_set(inline_json,'$.operation_id','different-turn')",
        "inline_json=json_set(inline_json,'$.holder_digest','foreign-holder-digest')",
        "inline_json=json_set(inline_json,'$.extra_authority',true)",
        "kind_version=2",
        "producer_id='different-producer'",
        "seq=seq+1000",
    ] {
        let store = AgentSessionStore::open_in_memory().await.unwrap();
        let f = fixture(&store, "typed-claim-causation").await;
        let lease = store.claim_native_execution(claim(&f, "initial", 0, None)).await.unwrap();
        store.append_native_event(
            &lease, &workspace_tool(&f, "typed-call", "typed-operation"), None,
        ).await.unwrap();
        native_cause(&store, &f, "typed-operation").await.unwrap();
        // Damage is injected only into this isolated canonical database. The
        // production writer does not expose claim-payload mutation.
        let claim_id = format!("native-execution:{}:lease-turn:0", f.session.agent_session_id.as_ref());
        sqlx::query(&format!("UPDATE agent_events SET {fault} WHERE event_id=?"))
            .bind(claim_id).execute(store.test_pool()).await.unwrap();
        let before = store.current_cursor(&f.session.agent_session_id).await.unwrap();
        assert!(native_cause(&store, &f, "typed-operation").await.is_err(),
            "claim must be typed and exact: {fault}");
        assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(), before);
    }
}

#[tokio::test]
async fn committed_native_tool_causation_requires_native_tool_header_and_typed_identity() {
    for fault in [
        "kind_version=2",
        "producer_id='capability-host'",
        "inline_json=json_remove(inline_json,'$.call_id')",
        "inline_json=json_set(inline_json,'$.call_id',42)",
        "inline_json=json_set(inline_json,'$.call_id','')",
        "inline_json=json_set(inline_json,'$.call_id',' padded-call ')",
        "inline_json=json_set(inline_json,'$.name',42)",
        "inline_json=json_set(inline_json,'$.extra_authority',true)",
    ] {
        let store = AgentSessionStore::open_in_memory().await.unwrap();
        let f = fixture(&store, "typed-native-tool-causation").await;
        let lease = store.claim_native_execution(claim(&f, "initial", 0, None)).await.unwrap();
        let mut tool = workspace_tool(&f, "native-header-call", "native-header-operation");
        // Production uses a tool-message projection identity here. It must
        // not be incorrectly required to equal the canonical Turn operation.
        tool.semantic_event.correlation_id = uuid::Uuid::now_v7().to_string().into();
        let ack = store.append_native_event(&lease, &tool, None).await.unwrap().ack.unwrap();
        let valid = native_cause(&store, &f, "native-header-operation").await.unwrap();
        assert_eq!(valid.tool_call.event_id, ack.event_id);
        sqlx::query(&format!("UPDATE agent_events SET {fault} WHERE event_id=?"))
            .bind(ack.event_id.as_ref()).execute(store.test_pool()).await.unwrap();
        let before = store.current_cursor(&f.session.agent_session_id).await.unwrap();
        // The basic Effect API deliberately retains its older identity
        // contract. Native authority independently requires the native header
        // and all typed tool-call identity fields.
        assert_eq!(store.effect_causation_event_id(
            &f.session.agent_session_id, &"lease-turn".into(), &"native-header-operation".into(),
            &"workspace.files".into(), &"workspace.files/read".into(),
        ).await.unwrap(), ack.event_id);
        assert!(native_cause(&store, &f, "native-header-operation").await.is_err(),
            "native call identity must fail closed: {fault}");
        assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(), before);
    }
}

#[tokio::test]
async fn committed_native_tool_causation_survives_disk_reopen_without_reusing_old_epoch() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("causation.db");
    let database = nomifun_db::init_database(&path).await.unwrap();
    let original = AgentSessionStore::from_pool(database.pool().clone()).await.unwrap();
    let f = fixture(&original, "disk-causation").await;
    let old = original.claim_native_execution(claim(&f, "old", 0, None)).await.unwrap();
    let old_tool = original.append_native_event(
        &old, &workspace_tool(&f, "disk-old-call", "disk-old-operation"), None,
    ).await.unwrap().ack.unwrap();
    let cp = checkpoint(&original, &f, &old).await;
    let old_cause = native_cause(&original, &f, "disk-old-operation").await.unwrap();
    expire(&original, &f).await;
    original.test_pool().close().await;
    database.close().await;
    drop(original);

    let reopened = AgentSessionStore::connect_existing(&path).await.unwrap();
    let fresh = reopened.claim_native_execution(claim(&f, "fresh", old.fence(), Some(&cp)))
        .await.unwrap();
    assert!(fresh.generation() > old_tool.seq);
    assert!(fresh.fence() > old.fence());
    assert!(matches!(native_cause(&reopened, &f, "disk-old-operation").await,
        Err(SessionStoreError::ExecutionFenced)));
    // The immutable old call remains queryable under the basic Effect
    // causation contract; reopening does not author or reset its sequence.
    assert_eq!(reopened.tool_call_causation(
        &f.session.agent_session_id, &"lease-turn".into(), &"disk-old-operation".into(),
        &"workspace.files".into(), &"workspace.files/read".into(),
    ).await.unwrap(), old_cause.tool_call);
    let fresh_tool = reopened.append_native_event(
        &fresh, &workspace_tool(&f, "disk-fresh-call", "disk-fresh-operation"), None,
    ).await.unwrap().ack.unwrap();
    let cause = native_cause(&reopened, &f, "disk-fresh-operation").await.unwrap();
    assert_eq!(cause.tool_call.seq, fresh_tool.seq);
    assert!(fresh_tool.seq > old_tool.seq);
    assert_eq!(cause.execution_scope.generation(), fresh.generation());
    assert_eq!(cause.execution_scope.fence(), fresh.fence());
    reopened.test_pool().close().await;
}

#[tokio::test]
async fn committed_native_tool_causation_rejects_paused_and_pre_resume_calls() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = fixture(&store, "resume-causation").await;
    let old = store.claim_native_execution(claim(&f, "old", 0, None)).await.unwrap();
    store.append_native_event(
        &old, &workspace_tool(&f, "resume-old-call", "resume-old-operation"), None,
    ).await.unwrap();
    let cp = checkpoint(&store, &f, &old).await;
    let pause = store.pause_native_execution(&old, "OWNER_REQUESTED", true).await.unwrap();
    assert!(matches!(native_cause(&store, &f, "resume-old-operation").await,
        Err(SessionStoreError::ExecutionFenced)));
    let request = NativeResumeRequest {
        operation_id:"lease-turn".into(), idempotency_key:"resume-causation-once".into(),
        expected_pause_revision:pause.revision, expected_checkpoint_revision:cp.revision,
        expected_checkpoint_digest:cp.digest, budget:Default::default(), cleanup_attestation:None,
    };
    let facts = store.native_recovery_facts(&f.session.agent_session_id, &"lease-turn".into())
        .await.unwrap();
    // This tests the Store's transaction contract; native Runtime codec tests
    // separately validate the exact build-compatible checkpoint body.
    let preparation = NativeResumePreparation {
        expected_head_seq:facts.head.last_seq, expected_fence:facts.execution_fence,
        snapshot:snapshot_ref(), active_set_generation:0, observations:vec![],
        checkpoint_state:StrictJsonValue(json!({"version":1,"model_steps":0,"turn_operation_id":"lease-turn",
            "active_set_generation":0,"binding":{"agent_session_id":f.session.agent_session_id,
            "resolved_snapshot_ref":snapshot_ref()}})),
    };
    store.commit_native_resume(&owner(), &f.session.agent_session_id, &request, preparation)
        .await.unwrap();
    assert!(matches!(native_cause(&store, &f, "resume-old-operation").await,
        Err(SessionStoreError::ExecutionFenced)));
    let cp = store.load_native_checkpoint(&owner(), &f.session.agent_session_id, &"lease-turn".into())
        .await.unwrap().unwrap();
    let fresh = store.claim_native_execution(claim(&f, "resumed", pause.execution_fence, Some(&cp)))
        .await.unwrap();
    assert!(matches!(native_cause(&store, &f, "resume-old-operation").await,
        Err(SessionStoreError::ExecutionFenced)));
    let tool = store.append_native_event(
        &fresh, &workspace_tool(&f, "resume-fresh-call", "resume-fresh-operation"), None,
    ).await.unwrap().ack.unwrap();
    let cause = native_cause(&store, &f, "resume-fresh-operation").await.unwrap();
    assert_eq!(cause.tool_call.seq, tool.seq);
    assert_eq!(cause.execution_scope.generation(), fresh.generation());
}

#[tokio::test]
async fn committed_native_tool_causation_cannot_cross_into_successor_turn() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = fixture(&store, "successor-causation").await;
    let old = store.claim_native_execution(claim(&f, "old", 0, None)).await.unwrap();
    store.append_native_event(
        &old, &workspace_tool(&f, "successor-old-call", "shared-operation-name"), None,
    ).await.unwrap();
    store.cancel_active_turn(&f.session.agent_session_id, "close-original".into(), "session_api".into())
        .await.unwrap();
    let (_, turn) = store.start_turn(
        &f.session.agent_session_id, "session_api".into(), "successor-input".into(),
        "successor-turn".into(), StrictJsonValue(json!({"content":"independent turn"})),
    ).await.unwrap();
    let fresh = store.claim_native_execution(NativeExecutionClaim {
        owner:owner(), agent_session_id:f.session.agent_session_id.clone(),
        operation_id:"successor-turn".into(), snapshot:snapshot_ref(), active_set_generation:0,
        holder:"successor".into(), expected_fence:0, checkpoint:None,
    }).await.unwrap();
    assert!(matches!(native_cause(&store, &f, "shared-operation-name").await,
        Err(SessionStoreError::ExecutionFenced)));
    assert!(store.native_tool_call_causation(
        &owner(), &f.session.agent_session_id, &"successor-turn".into(), &"shared-operation-name".into(),
        &"workspace.files".into(), &"workspace.files/read".into(),
    ).await.is_err());
    let new_tool = append(
        &f.session.agent_session_id, "successor-fresh-call", "runtime_supervisor",
        "successor-fresh-call", "tool/call-started", "successor-fresh-call",
        Some(turn.ack.unwrap().event_id),
        json!({"operation_id":"shared-operation-name","call_id":"successor-fresh-call",
            "capability_id":"workspace.files","action_id":"workspace.files/read"}),
    );
    let tool = store.append_native_event(&fresh, &new_tool, None).await.unwrap().ack.unwrap();
    let cause = store.native_tool_call_causation(
        &owner(), &f.session.agent_session_id, &"successor-turn".into(), &"shared-operation-name".into(),
        &"workspace.files".into(), &"workspace.files/read".into(),
    ).await.unwrap();
    assert_eq!(cause.tool_call.seq, tool.seq);
    assert_eq!(cause.execution_scope.generation(), fresh.generation());
}
