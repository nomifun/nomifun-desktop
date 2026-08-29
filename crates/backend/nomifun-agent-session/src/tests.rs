use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::{
    ActionId, AgentBindingValue, AgentPresetId, AgentSessionId, AgentSessionLiveRecord,
    AgentSessionMetadata, ArtifactId, CapabilityId, CompactionCompletedPayload, CorrelationId,
    DeleteAgentSessionCommand, DigestHex, EventId, EventProducerId, IdempotencyKey,
    LogicalArtifactRef, OperationId, PresetRevisionRef, PrincipalRef, ResolvedSnapshotId,
    ResolvedSnapshotRef, RuntimeBindingId, RuntimeCapabilityExecutionContract,
    RuntimeCheckpointBinding, RuntimeCheckpointValidationInput, RuntimeCheckpointValidationResult,
    RuntimeEventEnvelope, RuntimeExecutionCeiling, RuntimeExecutorSupport, RuntimeProfileKind,
    SemanticSessionEventDraft, SessionEventAppend, SessionEventKind, SessionEventPayloadRef,
    SessionPayloadBody, SessionPayloadRecord, SnapshotCompatibilityAdmissionInput,
    SnapshotCompatibilityAdmissionResult, StrictJsonValue, VersionString, canonical_json_bytes,
    digest_bytes,
};
use serde_json::json;
use uuid::Uuid;

use crate::{
    AgentSessionStore, CreateSessionRequest, EffectEventRequest, EffectReconcileOutcome,
    EffectTerminalState, ForkRequest, RuntimeAppendContext, SessionStoreError,
    ZeroOutstandingProof, evaluate_snapshot_compatibility, validate_checkpoint,
};

fn session_id() -> AgentSessionId {
    AgentSessionId(Uuid::now_v7().to_string())
}

fn event_id(value: &str) -> EventId {
    EventId(value.to_owned())
}

fn digest(byte: char) -> DigestHex {
    DigestHex(byte.to_string().repeat(64))
}

fn owner() -> PrincipalRef {
    PrincipalRef {
        principal_kind: "user".to_owned(),
        principal_id: "user-001".to_owned(),
    }
}

fn snapshot_ref() -> ResolvedSnapshotRef {
    ResolvedSnapshotRef {
        snapshot_id: ResolvedSnapshotId("snapshot-001".to_owned()),
        snapshot_digest: digest('b'),
    }
}

fn binding() -> AgentBindingValue {
    AgentBindingValue {
        preset_revision_ref: PresetRevisionRef {
            preset_id: AgentPresetId("coding.codex".to_owned()),
            revision: 1,
            revision_digest: digest('a'),
        },
        resolved_snapshot_ref: snapshot_ref(),
        typed_resource_bindings: Vec::new(),
        binding_version: 1,
    }
}

fn live_session(id: AgentSessionId) -> AgentSessionLiveRecord {
    AgentSessionLiveRecord {
        agent_session_id: id,
        owner_ref: owner(),
        metadata: AgentSessionMetadata {
            title: Some("Session fixture".to_owned()),
            archived: false,
            pinned: false,
        },
        agent_binding: binding(),
        remote_binding_provenance: None,
        parent_session_id: None,
        fork_base_payload_id: None,
        next_seq: 1,
    }
}

fn create_request(session: AgentSessionLiveRecord, key: &str) -> CreateSessionRequest {
    CreateSessionRequest {
        session,
        created_at: 1_788_000_000_000,
        operation_id: OperationId(format!("operation-{key}")),
        producer_id: EventProducerId("session-api".to_owned()),
        idempotency_key: IdempotencyKey(key.to_owned()),
        correlation_id: CorrelationId(format!("session-{key}")),
        opening_event_id: Some(event_id(&format!("event-opening-{key}"))),
        activation_event_id: Some(event_id(&format!("event-active-{key}"))),
        initial_active_capability_ids: vec!["coding.workspace".to_owned()],
    }
}

fn append(
    session_id: &AgentSessionId,
    event: &str,
    producer: &str,
    key: &str,
    kind: &str,
    correlation: &str,
    causation: Option<EventId>,
    payload: serde_json::Value,
) -> SessionEventAppend {
    SessionEventAppend {
        agent_session_id: session_id.clone(),
        event_id: event_id(event),
        producer_id: EventProducerId(producer.to_owned()),
        idempotency_key: IdempotencyKey(key.to_owned()),
        runtime_binding_id: None,
        runtime_producer_seq: None,
        semantic_event: SemanticSessionEventDraft {
            kind: SessionEventKind(kind.to_owned()),
            kind_version: 1,
            correlation_id: CorrelationId(correlation.to_owned()),
            causation_event_id: causation,
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(payload)),
        },
    }
}

async fn create_ready(store: &AgentSessionStore, key: &str) -> (AgentSessionLiveRecord, EventId) {
    let session = live_session(session_id());
    let created = store
        .create_session(create_request(session, key))
        .await
        .unwrap();
    let ready = append(
        &created.session.agent_session_id,
        &format!("event-ready-{key}"),
        "runtime-supervisor",
        &format!("ready-{key}"),
        "session/ready",
        &format!("session-{key}"),
        Some(created.opening_ack.event_id.clone()),
        json!({}),
    );
    let ready_ack = store.append_event(&ready).await.unwrap().ack.unwrap();
    (created.session, ready_ack.event_id)
}

#[tokio::test]
async fn shared_fresh_v4_schema_and_session_creation_are_exact_and_idempotent() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let tables: BTreeSet<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_schema \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(store.test_pool())
    .await
    .unwrap()
    .into_iter()
    .collect();
    let canonical_tables = nomifun_agent_contracts::fresh_v4_schema_manifest_payload()
        .tables
        .into_iter()
        .map(|table| table.table_name)
        .collect::<BTreeSet<_>>();
    assert_eq!(tables, canonical_tables);
    assert!(tables.len() > 5);
    for owned in [
        "agent_sessions",
        "message_projection",
        "session_events",
        "session_heads",
        "session_payloads",
    ] {
        assert!(tables.contains(owned));
    }
    let remote_fk: (String, String, String) = sqlx::query_as(
        "SELECT \"table\", \"from\", \"to\" \
         FROM pragma_foreign_key_list('agent_sessions') \
         WHERE \"from\" = 'remote_binding_id'",
    )
    .fetch_one(store.test_pool())
    .await
    .unwrap();
    assert_eq!(
        remote_fk,
        (
            "remote_bindings".to_owned(),
            "remote_binding_id".to_owned(),
            "remote_binding_id".to_owned(),
        )
    );
    AgentSessionStore::from_pool(store.test_pool().clone())
        .await
        .expect("the non-session Fresh-v4 tables must not be rejected");

    let request = create_request(live_session(session_id()), "create-1");
    let created = store.create_session(request.clone()).await.unwrap();
    assert!(!created.duplicate);
    assert_eq!(created.opening_ack.seq, 1);
    assert_eq!(created.activation_ack.seq, 2);
    assert_eq!(created.session.next_seq, 3);
    assert_eq!(
        store
            .head(&created.session.agent_session_id)
            .await
            .unwrap()
            .last_seq,
        2
    );

    let replay = store.create_session(request).await.unwrap();
    assert!(replay.duplicate);
    assert_eq!(
        replay.session.agent_session_id,
        created.session.agent_session_id
    );
    assert_eq!(replay.activation_ack.cursor, created.activation_ack.cursor);
}

#[tokio::test]
async fn append_projection_cursor_and_rebuild_are_one_deterministic_chain() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, ready_event) = create_ready(&store, "projection").await;
    let turn = append(
        &session.agent_session_id,
        "event-turn-started",
        "session-api",
        "turn-started",
        "turn/started",
        "turn-1",
        Some(ready_event),
        json!({}),
    );
    let turn_ack = store.append_event(&turn).await.unwrap().ack.unwrap();
    let part = append(
        &session.agent_session_id,
        "event-message-part",
        "runtime-supervisor",
        "message-part",
        "message/content-part",
        "message-1",
        Some(turn_ack.event_id.clone()),
        json!({"content": "hello"}),
    );
    let part_result = store.append_event(&part).await.unwrap();
    let duplicate = store.append_event(&part).await.unwrap();
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.cursor, part_result.cursor);
    let completed = append(
        &session.agent_session_id,
        "event-message-completed",
        "runtime-supervisor",
        "message-completed",
        "message/completed",
        "message-1",
        Some(part_result.ack.unwrap().event_id),
        json!({"content_digest": digest_bytes(b"hello"), "part_count": 1}),
    );
    store.append_event(&completed).await.unwrap();

    let before_head = store.head(&session.agent_session_id).await.unwrap();
    let before_messages = store
        .message_projections_after(&session.agent_session_id, 0)
        .await
        .unwrap();
    assert_eq!(before_messages.len(), 2);
    let message = before_messages
        .iter()
        .find(|projection| projection.projection_id == "message:message-1")
        .unwrap();
    assert_eq!(message.projection["content"], "hello");

    sqlx::query("DELETE FROM message_projection WHERE session_id = ?")
        .bind(session.agent_session_id.as_ref())
        .execute(store.test_pool())
        .await
        .unwrap();
    sqlx::query("DELETE FROM session_heads WHERE session_id = ?")
        .bind(session.agent_session_id.as_ref())
        .execute(store.test_pool())
        .await
        .unwrap();
    let rebuilt_head = store
        .rebuild_projections(&session.agent_session_id)
        .await
        .unwrap();
    let rebuilt_messages = store
        .message_projections_after(&session.agent_session_id, 0)
        .await
        .unwrap();
    assert_eq!(rebuilt_head, before_head);
    assert_eq!(rebuilt_messages, before_messages);

    let before_cursor = store
        .current_cursor(&session.agent_session_id)
        .await
        .unwrap();
    let skipped_generation = append(
        &session.agent_session_id,
        "event-active-generation-2",
        "capability-host",
        "active-generation-2",
        "capability/active-set-committed",
        "session-projection",
        None,
        json!({"generation": 2, "active_capability_ids": [], "delta": []}),
    );
    assert!(store.append_event(&skipped_generation).await.is_err());
    assert_eq!(
        store
            .current_cursor(&session.agent_session_id)
            .await
            .unwrap(),
        before_cursor
    );
}

#[tokio::test]
async fn stored_payload_and_event_commit_atomically_and_replay_without_budget_growth() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, ready_event) = create_ready(&store, "payload").await;
    let turn = append(
        &session.agent_session_id,
        "event-payload-turn",
        "session-api",
        "payload-turn",
        "turn/started",
        "turn-payload",
        Some(ready_event),
        json!({}),
    );
    let turn_ack = store.append_event(&turn).await.unwrap().ack.unwrap();
    let body = SessionPayloadBody::Json(StrictJsonValue(json!({"content": "stored"})));
    let logical = canonical_json_bytes(&json!({"content": "stored"})).unwrap();
    let payload = SessionPayloadRecord {
        payload_id: ArtifactId("payload-message-1".to_owned()),
        agent_session_id: session.agent_session_id.clone(),
        media_type: "application/json".to_owned(),
        byte_len: logical.len() as u64,
        digest: digest_bytes(&logical),
        body,
    };
    let part = SessionEventAppend {
        agent_session_id: session.agent_session_id.clone(),
        event_id: event_id("event-stored-part"),
        producer_id: EventProducerId("runtime-supervisor".to_owned()),
        idempotency_key: IdempotencyKey("stored-part".to_owned()),
        runtime_binding_id: None,
        runtime_producer_seq: None,
        semantic_event: SemanticSessionEventDraft {
            kind: SessionEventKind("message/content-part".to_owned()),
            kind_version: 1,
            correlation_id: CorrelationId("message-stored".to_owned()),
            causation_event_id: Some(turn_ack.event_id),
            payload: SessionEventPayloadRef::Stored(payload.payload_id.clone()),
        },
    };
    let committed = store
        .append_event_with_payload(&part, Some(&payload))
        .await
        .unwrap();
    let replay = store
        .append_event_with_payload(&part, Some(&payload))
        .await
        .unwrap();
    assert!(replay.duplicate);
    assert_eq!(replay.cursor, committed.cursor);
    let payload_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM session_payloads WHERE session_id = ?")
            .bind(session.agent_session_id.as_ref())
            .fetch_one(store.test_pool())
            .await
            .unwrap();
    assert_eq!(payload_count, 1);

    let completed = append(
        &session.agent_session_id,
        "event-stored-completed",
        "runtime-supervisor",
        "stored-completed",
        "message/completed",
        "message-stored",
        Some(committed.ack.unwrap().event_id),
        json!({"content_digest": digest_bytes(b"stored"), "part_count": 1}),
    );
    store.append_event(&completed).await.unwrap();
    let projection = store
        .message_projections_after(&session.agent_session_id, 0)
        .await
        .unwrap()
        .into_iter()
        .find(|projection| projection.projection_id == "message:message-stored")
        .unwrap();
    assert_eq!(projection.projection["content"], "stored");
}

#[tokio::test]
async fn completed_compaction_is_the_only_rehydration_base() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, ready_event) = create_ready(&store, "compaction").await;
    let turn = append(
        &session.agent_session_id,
        "event-compaction-turn",
        "session-api",
        "compaction-turn",
        "turn/started",
        "turn-compaction",
        Some(ready_event),
        json!({}),
    );
    let turn_ack = store.append_event(&turn).await.unwrap().ack.unwrap();
    let completed = append(
        &session.agent_session_id,
        "event-compaction-turn-completed",
        "runtime-supervisor",
        "compaction-turn-completed",
        "turn/completed",
        "turn-compaction",
        Some(turn_ack.event_id),
        json!({}),
    );
    let completed_ack = store.append_event(&completed).await.unwrap().ack.unwrap();

    let body = SessionPayloadBody::Json(StrictJsonValue(json!({
        "summary": "bounded completed context"
    })));
    let logical = canonical_json_bytes(&json!({
        "summary": "bounded completed context"
    }))
    .unwrap();
    let context_payload = SessionPayloadRecord {
        payload_id: ArtifactId("compaction-context-1".to_owned()),
        agent_session_id: session.agent_session_id.clone(),
        media_type: "application/json".to_owned(),
        byte_len: logical.len() as u64,
        digest: digest_bytes(&logical),
        body,
    };
    let compaction = CompactionCompletedPayload {
        agent_session_id: session.agent_session_id.clone(),
        through_seq: completed_ack.seq,
        context_payload_id: context_payload.payload_id.clone(),
        context_digest: context_payload.digest.clone(),
    };
    let event = append(
        &session.agent_session_id,
        "event-compaction-completed",
        "compaction-coordinator",
        "compaction-completed",
        "compaction/completed",
        "compaction-1",
        Some(completed_ack.event_id),
        serde_json::to_value(&compaction).unwrap(),
    );
    store
        .append_event_with_payload(&event, Some(&context_payload))
        .await
        .unwrap();

    let rehydration = store
        .rehydration_input(&session.agent_session_id)
        .await
        .unwrap();
    assert_eq!(rehydration.completed_compaction, Some(compaction));
    assert_eq!(
        rehydration.subsequent_events[0].kind.0,
        "compaction/completed"
    );
    assert_eq!(
        rehydration.resolved_snapshot_ref,
        session.agent_binding.resolved_snapshot_ref
    );
}

#[tokio::test]
async fn runtime_events_require_contiguous_binding_sequence_and_replay_original_ack() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let created = store
        .create_session(create_request(live_session(session_id()), "runtime"))
        .await
        .unwrap();
    let session_id = created.session.agent_session_id;
    let binding_id = RuntimeBindingId("runtime-binding-1".to_owned());
    let bound = SessionEventAppend {
        agent_session_id: session_id.clone(),
        event_id: event_id("event-runtime-bound"),
        producer_id: EventProducerId("runtime:runtime-binding-1".to_owned()),
        idempotency_key: IdempotencyKey("runtime-bound".to_owned()),
        runtime_binding_id: Some(binding_id.clone()),
        runtime_producer_seq: Some(1),
        semantic_event: SemanticSessionEventDraft {
            kind: SessionEventKind("runtime/bound".to_owned()),
            kind_version: 1,
            correlation_id: CorrelationId("runtime-binding-1".to_owned()),
            causation_event_id: Some(created.opening_ack.event_id),
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                "runtime_build_digest": digest('c'),
                "protocol_version": "runtime-v1",
                "snapshot_digest": digest('b')
            }))),
        },
    };
    store.append_event(&bound).await.unwrap();

    let checkpoint = RuntimeEventEnvelope {
        runtime_binding_id: binding_id.clone(),
        producer_seq: 2,
        event_id: event_id("event-runtime-checkpoint"),
        idempotency_key: IdempotencyKey("runtime-checkpoint".to_owned()),
        semantic_event: SemanticSessionEventDraft {
            kind: SessionEventKind("runtime/checkpointed".to_owned()),
            kind_version: 1,
            correlation_id: CorrelationId("runtime-binding-1".to_owned()),
            causation_event_id: Some(bound.event_id),
            payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                "locator": {
                    "normalized_relative_path": "runtime/checkpoint.json",
                    "digest": digest('d')
                },
                "runtime_bound_event_id": "event-runtime-bound",
                "protocol_version": "runtime-v1",
                "snapshot_digest": digest('b'),
                "through_seq": 3
            }))),
        },
    };
    let context = RuntimeAppendContext {
        agent_session_id: session_id.clone(),
        envelope: checkpoint.clone(),
    };
    let committed = store.append_runtime_event(context.clone()).await.unwrap();
    assert_eq!(committed.ack.unwrap().committed_producer_seq, 2);
    let replay = store.append_runtime_event(context).await.unwrap();
    assert!(replay.append.duplicate);

    let mut gap = checkpoint;
    gap.producer_seq = 4;
    gap.event_id = event_id("event-runtime-gap");
    gap.idempotency_key = IdempotencyKey("runtime-gap".to_owned());
    assert!(matches!(
        store
            .append_runtime_event(RuntimeAppendContext {
                agent_session_id: session_id,
                envelope: gap,
            })
            .await
            .unwrap_err(),
        SessionStoreError::RuntimeSequenceGap {
            committed_producer_seq: 2,
            expected: 3,
            actual: 4,
            ..
        }
    ));
}

#[tokio::test]
async fn uncertain_effect_is_terminal_until_owning_plugin_reconciles() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, ready_event) = create_ready(&store, "effect").await;
    let turn = append(
        &session.agent_session_id,
        "event-effect-turn",
        "session-api",
        "effect-turn",
        "turn/started",
        "turn-effect",
        Some(ready_event),
        json!({}),
    );
    let turn_ack = store.append_event(&turn).await.unwrap().ack.unwrap();
    let tool = append(
        &session.agent_session_id,
        "event-tool-started",
        "runtime-supervisor",
        "tool-started",
        "tool/call-started",
        "tool-1",
        Some(turn_ack.event_id),
        json!({"tool": "files.write"}),
    );
    let tool_ack = store.append_event(&tool).await.unwrap().ack.unwrap();

    let started = EffectEventRequest {
        agent_session_id: session.agent_session_id.clone(),
        event_id: event_id("event-effect-started"),
        producer_id: EventProducerId("capability-host".to_owned()),
        idempotency_key: IdempotencyKey("effect-idem".to_owned()),
        correlation_id: CorrelationId("effect-1".to_owned()),
        causation_event_id: Some(tool_ack.event_id),
        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
            "effect": "write"
        }))),
    };
    let started_ack = store
        .record_effect_started(started.clone())
        .await
        .unwrap()
        .ack
        .unwrap();
    let uncertain = EffectEventRequest {
        event_id: event_id("event-effect-uncertain"),
        producer_id: EventProducerId("runtime-supervisor".to_owned()),
        causation_event_id: Some(started_ack.event_id),
        ..started.clone()
    };
    let uncertain_ack = store
        .record_effect_terminal(uncertain, EffectTerminalState::Uncertain)
        .await
        .unwrap()
        .ack
        .unwrap();
    assert_eq!(
        store.head(&session.agent_session_id).await.unwrap().status,
        "failed"
    );

    let retry = EffectEventRequest {
        event_id: event_id("event-effect-retry"),
        producer_id: EventProducerId("capability-host-retry".to_owned()),
        idempotency_key: IdempotencyKey("effect-retry".to_owned()),
        ..started.clone()
    };
    assert!(store.record_effect_started(retry).await.is_err());

    let reconcile = EffectEventRequest {
        event_id: event_id("event-effect-reconciled"),
        producer_id: EventProducerId("owning-plugin".to_owned()),
        causation_event_id: Some(uncertain_ack.event_id),
        ..started
    };
    store
        .reconcile_effect(reconcile, EffectReconcileOutcome::StillUncertain)
        .await
        .unwrap();
    let projections = store
        .message_projections_after(&session.agent_session_id, 0)
        .await
        .unwrap();
    assert_eq!(
        projections
            .iter()
            .find(|projection| projection.projection_id == "effect:effect-1")
            .unwrap()
            .projection["state"],
        "still_uncertain"
    );
}

#[tokio::test]
async fn checkpoint_validation_and_snapshot_admission_are_exact() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let created = store
        .create_session(create_request(live_session(session_id()), "checkpoint"))
        .await
        .unwrap();
    let profile_digest = digest('e');
    let typed_resource_digest = digest('f');
    let ceiling = RuntimeExecutionCeiling {
        protocol_version: VersionString("runtime-v1".to_owned()),
        protocol_schema_digest: digest('1'),
        profile_kind: RuntimeProfileKind::ManagedMinimal,
        profile_digest: profile_digest.clone(),
        native_features: BTreeSet::new(),
        native_actions: BTreeSet::new(),
        initial_capabilities: BTreeMap::<CapabilityId, RuntimeCapabilityExecutionContract>::new(),
        on_demand_capabilities: BTreeMap::new(),
        packages: BTreeMap::new(),
        skills: BTreeMap::new(),
        mcp_tools: BTreeMap::new(),
        model_routes: BTreeMap::new(),
        typed_resource_bindings: Vec::new(),
        typed_resource_contract_digest: typed_resource_digest.clone(),
    };
    let support = RuntimeExecutorSupport {
        runtime_release_digest: digest('2'),
        hello_payload_digest: digest('3'),
        protocol_versions: BTreeSet::from([VersionString("runtime-v1".to_owned())]),
        protocol_schema_digests: BTreeSet::from([digest('1')]),
        profile_digests: BTreeMap::from([(
            RuntimeProfileKind::ManagedMinimal,
            BTreeSet::from([profile_digest]),
        )]),
        native_features: BTreeSet::new(),
        native_actions: BTreeSet::<ActionId>::new(),
        capabilities: BTreeMap::new(),
        packages: BTreeMap::new(),
        skills: BTreeMap::new(),
        mcp_tools: BTreeMap::new(),
        model_routes: BTreeMap::new(),
        typed_resource_contract_digests: BTreeSet::from([typed_resource_digest]),
    };
    let admission = SnapshotCompatibilityAdmissionInput {
        resolved_snapshot_ref: snapshot_ref(),
        required_ceiling: ceiling,
        available_executor: support,
    };
    assert!(matches!(
        evaluate_snapshot_compatibility(&admission),
        SnapshotCompatibilityAdmissionResult::CompatibleExact { .. }
    ));

    let checkpoint = RuntimeCheckpointBinding {
        runtime_binding_id: RuntimeBindingId("runtime-binding-1".to_owned()),
        locator: LogicalArtifactRef {
            artifact_id: ArtifactId("checkpoint-1".to_owned()),
            normalized_relative_path: "runtime/checkpoint-1".to_owned(),
            digest: digest('4'),
        },
        runtime_bound_event_id: event_id("runtime-bound-1"),
        protocol_version: VersionString("runtime-v1".to_owned()),
        resolved_snapshot_ref: snapshot_ref(),
        through_seq: 4,
    };
    let exact = RuntimeCheckpointValidationInput {
        checkpoint: checkpoint.clone(),
        referenced_runtime_build_digest: digest('5'),
        expected_runtime_bound_event_id: event_id("runtime-bound-1"),
        expected_runtime_build_digest: digest('5'),
        expected_protocol_version: VersionString("runtime-v1".to_owned()),
        expected_snapshot_ref: snapshot_ref(),
        expected_through_seq: 4,
    };
    assert_eq!(
        validate_checkpoint(&exact),
        RuntimeCheckpointValidationResult::ExactMatch
    );
    let admitted = store
        .admit_checkpoint(&created.session.agent_session_id, &exact, &admission)
        .await
        .unwrap();
    assert!(admitted.checkpoint_reusable);

    let mut mismatch = exact.clone();
    mismatch.expected_through_seq = 5;
    assert!(matches!(
        validate_checkpoint(&mismatch),
        RuntimeCheckpointValidationResult::Mismatch { .. }
    ));
    assert!(
        !store
            .admit_checkpoint(&created.session.agent_session_id, &mismatch, &admission,)
            .await
            .unwrap()
            .checkpoint_reusable
    );

    let mut unavailable = admission.clone();
    unavailable.available_executor.protocol_versions.clear();
    let error = store
        .admit_checkpoint(&created.session.agent_session_id, &exact, &unavailable)
        .await
        .unwrap_err();
    assert_eq!(error.code(), Some("SNAPSHOT_EXECUTOR_UNAVAILABLE"));
}

#[tokio::test]
async fn fork_is_self_contained_and_parent_deletion_leaves_child_live() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (parent, _) = create_ready(&store, "fork-parent").await;
    let child_id = session_id();
    let request = ForkRequest {
        child_session_id: child_id.clone(),
        child_owner_ref: owner(),
        child_metadata: AgentSessionMetadata {
            title: Some("Fork child".to_owned()),
            archived: false,
            pinned: false,
        },
        child_agent_binding: binding(),
        created_at: 1_788_000_000_100,
        producer_id: EventProducerId("fork-coordinator".to_owned()),
        operation_id: OperationId("fork-operation-1".to_owned()),
        idempotency_key: IdempotencyKey("fork-idem-1".to_owned()),
        correlation_id: CorrelationId("fork-1".to_owned()),
        event_id: Some(event_id("event-forked-1")),
        base_payload_id: ArtifactId("fork-base-1".to_owned()),
        base_body: SessionPayloadBody::Json(StrictJsonValue(json!({
            "summary": "self-contained completed semantics"
        }))),
        base_media_type: "application/json".to_owned(),
        child_initial_active_capability_ids: vec!["coding.workspace".to_owned()],
    };
    let forked = store
        .fork_session(&parent.agent_session_id, request.clone())
        .await
        .unwrap();
    assert_eq!(forked.child_session.agent_session_id, child_id);
    assert_eq!(
        forked.child_session.parent_session_id,
        Some(parent.agent_session_id.clone())
    );
    assert!(forked.contract.child_base_is_self_contained);
    assert!(!forked.contract.copies_full_transcript);
    assert_eq!(forked.child_cursor.seq, 2);
    let replay = store
        .fork_session(&parent.agent_session_id, request)
        .await
        .unwrap();
    assert_eq!(replay.child_session.agent_session_id, child_id);
    assert_eq!(replay.fork_ack, forked.fork_ack);

    let delete = DeleteAgentSessionCommand {
        operation_id: OperationId("delete-parent".to_owned()),
        agent_session_id: parent.agent_session_id.clone(),
        owner_ref: owner(),
        requested_at: 1_788_000_000_200,
    };
    store
        .delete_session(
            &delete,
            &ZeroOutstandingProof::verified(),
            1_788_000_000_300,
        )
        .await
        .unwrap();
    assert!(store.get_live_session(&child_id).await.is_ok());
}

#[tokio::test]
async fn deletion_fence_blocks_late_work_and_commits_exact_tombstone() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, ready_event) = create_ready(&store, "delete").await;
    let command = DeleteAgentSessionCommand {
        operation_id: OperationId("delete-operation".to_owned()),
        agent_session_id: session.agent_session_id.clone(),
        owner_ref: owner(),
        requested_at: 1_788_000_001_000,
    };
    store.fence_delete(&command).await.unwrap();
    assert_eq!(store.deleting_sessions().await.unwrap().len(), 1);

    let late = append(
        &session.agent_session_id,
        "event-late-turn",
        "session-api",
        "late-turn",
        "turn/started",
        "late-turn",
        Some(ready_event),
        json!({}),
    );
    assert!(matches!(
        store.append_event(&late).await.unwrap_err(),
        SessionStoreError::Deleted(_)
    ));
    assert!(matches!(
        store
            .current_cursor(&session.agent_session_id)
            .await
            .unwrap_err(),
        SessionStoreError::Deleted(_)
    ));

    let mut nonzero = ZeroOutstandingProof::verified();
    nonzero.counts.insert("runtime_binding".to_owned(), 1);
    assert!(
        store
            .complete_delete(&command, &nonzero, 1_788_000_001_100)
            .await
            .is_err()
    );
    let deleted = store
        .complete_delete(
            &command,
            &ZeroOutstandingProof::verified(),
            1_788_000_001_200,
        )
        .await
        .unwrap();
    assert_eq!(deleted.tombstone.agent_session_id, session.agent_session_id);
    assert_eq!(deleted.tombstone.owner_ref, owner());
    assert_eq!(deleted.tombstone.deleted_at, 1_788_000_001_200);

    let tombstone = store
        .inspect_tombstone(&session.agent_session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(tombstone, deleted.tombstone);
    for table in [
        "session_events",
        "session_payloads",
        "session_heads",
        "message_projection",
    ] {
        let count: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM {table} WHERE session_id = ?"
        ))
        .bind(session.agent_session_id.as_ref())
        .fetch_one(store.test_pool())
        .await
        .unwrap();
        assert_eq!(count, 0, "{table} retained deleted Session content");
    }
    assert!(matches!(
        store.fence_delete(&command).await.unwrap_err(),
        SessionStoreError::Deleted(_)
    ));
}
