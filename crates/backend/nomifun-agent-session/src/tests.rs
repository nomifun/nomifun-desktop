use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::{
    ActionId, AgentBindingValue, AgentPresetId, AgentSessionId, AgentSessionLiveRecord,
    AgentSessionMetadata, ArtifactId, CapabilityId, CompactionCompletedPayload, CorrelationId,
    ChatRouteIdentity, DeleteAgentSessionCommand, DigestHex, EventId, EventProducerId,
    IdempotencyKey, LogicalArtifactRef, OperationId, PresetRevisionRef, PrincipalRef,
    RemoteBindingId, RemoteBindingProvenance, ResolvedSnapshotId, ResolvedSnapshotRef,
    RuntimeBindingId, RuntimeCapabilityExecutionContract,
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
    AgentSessionStore, ChatOperationClaimRequest, CreateSessionRequest, EffectEventRequest,
    EffectReconcileOutcome, EffectTerminalState, ForkRequest, RuntimeAppendContext,
    SessionStoreError, ZeroOutstandingProof, evaluate_snapshot_compatibility, validate_checkpoint,
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
        initial_input: None,
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
    let remote_fk: Option<(String, String, String)> = sqlx::query_as(
        "SELECT \"table\", \"from\", \"to\" \
         FROM pragma_foreign_key_list('agent_sessions') \
         WHERE \"from\" = 'remote_binding_id'",
    )
    .fetch_optional(store.test_pool())
    .await
    .unwrap();
    assert!(
        remote_fk.is_none(),
        "Session provenance must not have a reverse foreign key to RemoteBinding"
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
async fn opening_remote_session_listing_is_exact_and_excludes_ready_or_local_sessions() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();

    let opening_id = session_id();
    let mut opening = live_session(opening_id.clone());
    opening.remote_binding_provenance = Some(RemoteBindingProvenance {
        remote_binding_id: RemoteBindingId::from("remote-opening"),
        binding_version: 1,
    });
    store
        .create_session(create_request(opening, "remote-opening"))
        .await
        .unwrap();

    let ready_id = session_id();
    let mut ready = live_session(ready_id.clone());
    ready.remote_binding_provenance = Some(RemoteBindingProvenance {
        remote_binding_id: RemoteBindingId::from("remote-ready"),
        binding_version: 1,
    });
    let ready_created = store
        .create_session(create_request(ready, "remote-ready"))
        .await
        .unwrap();
    store
        .append_event(&append(
            &ready_id,
            "event-ready-remote",
            "runtime-supervisor",
            "ready-remote",
            "session/ready",
            "session-ready-remote",
            Some(ready_created.opening_ack.event_id),
            json!({}),
        ))
        .await
        .unwrap();

    store
        .create_session(create_request(live_session(session_id()), "local-opening"))
        .await
        .unwrap();

    assert_eq!(
        store.list_opening_remote_sessions().await.unwrap(),
        vec![opening_id]
    );
}

#[tokio::test]
async fn runtime_admission_boundary_is_atomic_and_cannot_revive_open_failed() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();

    let ready_session = live_session(session_id());
    let ready_created = store
        .create_session(create_request(ready_session, "atomic-ready"))
        .await
        .unwrap();
    let ready_binding = RuntimeBindingId::from("atomic-ready-binding");
    let ready_bound_event = event_id("atomic-ready-bound");
    let ready_context = RuntimeAppendContext {
        agent_session_id: ready_created.session.agent_session_id.clone(),
        envelope: RuntimeEventEnvelope {
            runtime_binding_id: ready_binding.clone(),
            producer_seq: 1,
            event_id: ready_bound_event.clone(),
            idempotency_key: IdempotencyKey::from("atomic-ready-bound"),
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("runtime/bound".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from("atomic-ready-binding"),
                causation_event_id: Some(ready_created.opening_ack.event_id.clone()),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "runtime_build_digest": digest('c'),
                    "protocol_version": "runtime-v1",
                    "snapshot_digest": digest('b')
                }))),
            },
        },
    };
    let ready_event = append(
        &ready_created.session.agent_session_id,
        "atomic-ready-event",
        "runtime-supervisor",
        "atomic-ready",
        "session/ready",
        "atomic-ready-session",
        Some(ready_bound_event),
        json!({}),
    );
    store
        .append_runtime_bound_and_ready(ready_context, &ready_event)
        .await
        .unwrap();
    assert_eq!(
        store
            .head(&ready_created.session.agent_session_id)
            .await
            .unwrap()
            .status,
        "ready"
    );

    let failed_session = live_session(session_id());
    let failed_created = store
        .create_session(create_request(failed_session, "atomic-failed"))
        .await
        .unwrap();
    store
        .append_open_failed(
            &failed_created.session.agent_session_id,
            "REMOTE_OPEN_FAILED",
            "runtime unavailable",
            true,
        )
        .await
        .unwrap()
        .expect("open failure should be committed");

    let late_binding = RuntimeBindingId::from("atomic-late-binding");
    let late_bound_event = event_id("atomic-late-bound");
    let late_context = RuntimeAppendContext {
        agent_session_id: failed_created.session.agent_session_id.clone(),
        envelope: RuntimeEventEnvelope {
            runtime_binding_id: late_binding,
            producer_seq: 1,
            event_id: late_bound_event.clone(),
            idempotency_key: IdempotencyKey::from("atomic-late-bound"),
            semantic_event: SemanticSessionEventDraft {
                kind: SessionEventKind("runtime/bound".to_owned()),
                kind_version: 1,
                correlation_id: CorrelationId::from("atomic-late-binding"),
                causation_event_id: Some(failed_created.opening_ack.event_id),
                payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
                    "runtime_build_digest": digest('e'),
                    "protocol_version": "runtime-v1",
                    "snapshot_digest": digest('b')
                }))),
            },
        },
    };
    let late_ready = append(
        &failed_created.session.agent_session_id,
        "atomic-late-ready",
        "runtime-supervisor",
        "atomic-late-ready",
        "session/ready",
        "atomic-late-session",
        Some(late_bound_event),
        json!({}),
    );
    assert!(matches!(
        store
            .append_runtime_bound_and_ready(late_context, &late_ready)
            .await
            .unwrap_err(),
        SessionStoreError::Conflict(message)
            if message.contains("requires an opening Session")
    ));
    assert_eq!(
        store
            .head(&failed_created.session.agent_session_id)
            .await
            .unwrap()
            .status,
        "open_failed"
    );
    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM session_events \
         WHERE session_id = ? AND kind IN ('runtime/bound', 'session/ready')",
    )
    .bind(failed_created.session.agent_session_id.as_ref())
    .fetch_one(store.test_pool())
    .await
    .unwrap();
    assert_eq!(event_count, 0);
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
async fn chat_operation_claim_is_atomic_and_respects_turn_fence() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, _ready_event) = create_ready(&store, "chat-claim").await;
    let turn_operation = OperationId("turn-chat-claim".to_owned());
    let input = append(
        &session.agent_session_id,
        "event-chat-input",
        "session-api",
        "chat-input",
        "message/user-accepted",
        turn_operation.as_ref(),
        None,
        json!({"content": "hello"}),
    );
    let input_ack = store.append_event(&input).await.unwrap().ack.unwrap();
    let turn = append(
        &session.agent_session_id,
        "event-chat-turn",
        "session-api",
        "chat-turn",
        "turn/started",
        turn_operation.as_ref(),
        Some(input_ack.event_id.clone()),
        json!({
            "operation_id": turn_operation,
            "input_event_id": input_ack.event_id,
            "route_identity": ChatRouteIdentity::new(
                "coding.codex@1",
                nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,
                "chat-route".into(),
                4,
            ),
            "resolved_snapshot_ref": snapshot_ref(),
        }),
    );
    let turn_ack = store.append_event(&turn).await.unwrap().ack.unwrap();
    assert_eq!(turn_ack.seq, 5);

    let claim = ChatOperationClaimRequest {
        agent_session_id: session.agent_session_id.clone(),
        operation_id: OperationId("model-chat-claim".to_owned()),
        turn_operation_id: turn_operation.clone(),
        causation_event_id: input_ack.event_id,
        route_identity: ChatRouteIdentity::new(
            "coding.codex@1",
            nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,
            "chat-route".into(),
            4,
        ),
        resolved_snapshot_ref: snapshot_ref(),
    };
    let first = store.claim_chat_operation(claim.clone()).await.unwrap();
    assert!(!first.duplicate);
    let replay = store.claim_chat_operation(claim).await.unwrap();
    assert!(replay.duplicate);

    let cancelled = append(
        &session.agent_session_id,
        "event-chat-cancelled",
        "session-api",
        "chat-cancelled",
        "turn/cancelled",
        turn_operation.as_ref(),
        Some(turn_ack.event_id),
        json!({"target_operation_id": "turn-chat-claim"}),
    );
    store.append_event(&cancelled).await.unwrap();
    let fenced = store
        .claim_chat_operation(ChatOperationClaimRequest {
            agent_session_id: session.agent_session_id,
            operation_id: OperationId("model-after-cancel".to_owned()),
            turn_operation_id: turn_operation,
            causation_event_id: EventId("event-chat-input".to_owned()),
            route_identity: ChatRouteIdentity::new(
                "coding.codex@1",
                nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,
                "chat-route".into(),
                4,
            ),
            resolved_snapshot_ref: snapshot_ref(),
        })
        .await
        .unwrap_err();
    assert!(matches!(fenced, SessionStoreError::Conflict(_)));
}

#[tokio::test]
async fn remote_cancel_selects_active_turn_atomically_and_replays() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, ready_event) = create_ready(&store, "remote-cancel").await;
    let turn_operation = OperationId::from("remote-turn-operation");
    let input = append(
        &session.agent_session_id,
        "remote-cancel-input",
        "session-api",
        "remote-cancel-input",
        "message/user-accepted",
        turn_operation.as_ref(),
        Some(ready_event),
        json!({"content": "cancel me"}),
    );
    let input_ack = store.append_event(&input).await.unwrap().ack.unwrap();
    let turn = append(
        &session.agent_session_id,
        "remote-cancel-turn",
        "session-api",
        "remote-cancel-turn",
        "turn/started",
        turn_operation.as_ref(),
        Some(input_ack.event_id),
        json!({"operation_id": turn_operation}),
    );
    store.append_event(&turn).await.unwrap();

    let key = IdempotencyKey::from("remote-cancel-key");
    let (target, first) = store
        .cancel_active_turn(
            &session.agent_session_id,
            key.clone(),
            EventProducerId::from("remote_rest"),
        )
        .await
        .unwrap();
    assert_eq!(target, turn_operation);
    assert!(!first.duplicate);
    assert_eq!(
        store.head(&session.agent_session_id).await.unwrap().status,
        "ready"
    );

    let (replayed_target, replay) = store
        .cancel_active_turn(
            &session.agent_session_id,
            key,
            EventProducerId::from("remote_rest"),
        )
        .await
        .unwrap();
    assert_eq!(replayed_target, target);
    assert!(replay.duplicate);

    let no_active = store
        .cancel_active_turn(
            &session.agent_session_id,
            IdempotencyKey::from("remote-cancel-no-active"),
            EventProducerId::from("remote_rest"),
        )
        .await
        .unwrap_err();
    assert!(matches!(no_active, SessionStoreError::Conflict(_)));
}

#[tokio::test]
async fn chat_completion_is_atomic_and_cannot_cross_a_cancel_fence() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, ready_event) = create_ready(&store, "chat-terminal").await;
    let turn_operation = OperationId("turn-chat-terminal".to_owned());
    let input = append(
        &session.agent_session_id,
        "event-chat-terminal-input",
        "session-api",
        "chat-terminal-input",
        "message/user-accepted",
        turn_operation.as_ref(),
        Some(ready_event.clone()),
        json!({"content": "hello"}),
    );
    let input_ack = store.append_event(&input).await.unwrap().ack.unwrap();
    let turn = append(
        &session.agent_session_id,
        "event-chat-terminal-turn",
        "session-api",
        "chat-terminal-turn",
        "turn/started",
        turn_operation.as_ref(),
        Some(input_ack.event_id),
        json!({"operation_id": turn_operation}),
    );
    let turn_ack = store.append_event(&turn).await.unwrap().ack.unwrap();
    let cancelled = append(
        &session.agent_session_id,
        "event-chat-terminal-cancelled",
        "session-api",
        "chat-terminal-cancelled",
        "turn/cancelled",
        turn_operation.as_ref(),
        Some(turn_ack.event_id),
        json!({"target_operation_id": turn_operation}),
    );
    store.append_event(&cancelled).await.unwrap();

    let message = append(
        &session.agent_session_id,
        "event-chat-terminal-message",
        "runtime-supervisor",
        "chat-terminal-message",
        "message/completed",
        "message-chat-terminal",
        Some(EventId("event-chat-terminal-turn".to_owned())),
        json!({
            "content_digest": digest_bytes(b""),
            "part_count": 0
        }),
    );
    let terminal = append(
        &session.agent_session_id,
        "event-chat-terminal-completed",
        "runtime-supervisor",
        "chat-terminal-completed",
        "turn/completed",
        turn_operation.as_ref(),
        Some(message.event_id.clone()),
        json!({"message_event_id": message.event_id}),
    );
    let error = store
        .append_chat_completion(&message, &terminal, &turn_operation)
        .await
        .unwrap_err();
    assert!(matches!(error, SessionStoreError::Conflict(_)));

    let events = store
        .read_events(&session.agent_session_id, None, crate::MAX_EVENT_PAGE_SIZE)
        .await
        .unwrap()
        .events;
    assert!(
        events
            .iter()
            .all(|event| event.kind.0 != "message/completed"),
        "message terminal must roll back when the turn is already cancelled"
    );
    assert_eq!(
        store.head(&session.agent_session_id).await.unwrap().status,
        "ready"
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
        parent_through_seq: 1,
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
    assert_eq!(forked.contract.fork.parent_through_seq, 1);
    assert_eq!(forked.child_cursor.seq, 2);
    let replay = store
        .fork_session(&parent.agent_session_id, request.clone())
        .await
        .unwrap();
    assert_eq!(replay.child_session.agent_session_id, child_id);
    assert_eq!(replay.fork_ack, forked.fork_ack);

    let mut changed_cursor = request.clone();
    changed_cursor.parent_through_seq = 2;
    assert_eq!(
        store
            .fork_session(&parent.agent_session_id, changed_cursor)
            .await
            .unwrap_err()
            .code(),
        Some("IDEMPOTENCY_CONFLICT")
    );

    let mut beyond_head = request;
    beyond_head.child_session_id = session_id();
    beyond_head.idempotency_key = IdempotencyKey("fork-idem-beyond-head".to_owned());
    beyond_head.operation_id = OperationId("fork-operation-beyond-head".to_owned());
    beyond_head.parent_through_seq = u64::MAX;
    assert_eq!(
        store
            .fork_session(&parent.agent_session_id, beyond_head)
            .await
            .unwrap_err()
            .code(),
        Some("INVALID_SESSION")
    );

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
