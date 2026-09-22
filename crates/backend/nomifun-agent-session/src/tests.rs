use std::collections::{BTreeMap, BTreeSet};

use nomifun_agent_contracts::{
    ActionId, AgentBindingValue, AgentPresetId, AgentSessionId, AgentSessionLiveRecord,
    AgentSessionMetadata, ArtifactId, CapabilityId, CompactionCompletedPayload, CorrelationId,
    ChatRouteIdentity, DeleteAgentSessionCommand, DigestHex, EffectClass, EventId, EventProducerId,
    IdempotencyKey, LogicalArtifactRef, OperationId, PresetRevisionRef, PrincipalRef,
    RemoteBindingId, RemoteBindingProvenance, ResolvedSnapshotId, ResolvedSnapshotRef,
    ResourceBindingId, ResourceId, ResourceKind, TypedResourceBinding,
    RuntimeBindingId, RuntimeCapabilityExecutionContract,
    RuntimeCheckpointBinding, RuntimeCheckpointValidationInput, RuntimeCheckpointValidationResult,
    RuntimeEventEnvelope, RuntimeExecutionCeiling, RuntimeExecutorSupport, RuntimeProfileKind,
    SemanticSessionEventDraft, SessionEventAppend, SessionEventKind, SessionEventPayloadRef,
    SessionEventRecord, SessionPayloadBody, SessionPayloadRecord, SnapshotCompatibilityAdmissionInput,
    SnapshotCompatibilityAdmissionResult, StrictJsonValue, VersionString, canonical_json_bytes,
    digest_bytes,
};
use serde_json::json;
use uuid::Uuid;

use crate::{
    AgentEffectState, AgentSessionStore, ChatOperationClaimRequest, CreateSessionRequest,
    EffectEventRequest, EffectReconcileOutcome, EffectStrategy, EffectTerminalState, ForkRequest,
    RuntimeAppendContext, SessionStoreError, TurnReceiptStatus, evaluate_snapshot_compatibility,
    validate_checkpoint,
};
use crate::projector::reduce_agent_messages;

fn session_id() -> AgentSessionId {
    AgentSessionId(Uuid::now_v7().to_string())
}

fn event_id(value: &str) -> EventId {
    EventId(value.to_owned())
}

fn digest(byte: char) -> DigestHex {
    DigestHex(byte.to_string().repeat(64))
}

#[test]
fn effect_classes_collapse_to_exactly_three_lifecycle_strategies() {
    for class in [
        EffectClass::Pure,
        EffectClass::ReadLocal,
        EffectClass::ReadSensitive,
    ] {
        assert_eq!(
            EffectStrategy::from_effect_class(class),
            EffectStrategy::ReadOnly
        );
    }
    for class in [
        EffectClass::WriteReversible,
        EffectClass::WriteDurable,
        EffectClass::ExecuteLocal,
        EffectClass::Destructive,
        EffectClass::Irreversible,
    ] {
        assert_eq!(
            EffectStrategy::from_effect_class(class),
            EffectStrategy::ManagedEffect
        );
    }
    for class in [EffectClass::ExternalTransmit, EffectClass::Physical] {
        assert_eq!(
            EffectStrategy::from_effect_class(class),
            EffectStrategy::ExternalUncertainEffect
        );
    }
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

fn projection_event(
    session_id: &AgentSessionId,
    seq: u64,
    event: &str,
    kind: &str,
    correlation: &str,
    payload: serde_json::Value,
) -> SessionEventRecord {
    SessionEventRecord {
        agent_session_id: session_id.clone(),
        seq,
        event_id: event_id(event),
        producer_id: EventProducerId("projection-test".to_owned()),
        idempotency_key: IdempotencyKey(format!("projection-{event}")),
        runtime_binding_id: None,
        runtime_producer_seq: None,
        kind: SessionEventKind(kind.to_owned()),
        kind_version: 1,
        correlation_id: CorrelationId(correlation.to_owned()),
        causation_event_id: None,
        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(payload)),
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

async fn create_turn(
    store: &AgentSessionStore,
    key: &str,
    operation: &str,
) -> (AgentSessionLiveRecord, EventId) {
    let (session, ready_event) = create_ready(store, key).await;
    let turn = append(
        &session.agent_session_id,
        &format!("event-turn-started-{key}"),
        "session-api",
        &format!("turn-started-{key}"),
        "turn/started",
        operation,
        Some(ready_event),
        json!({"operation_id": operation}),
    );
    let turn_ack = store.append_event(&turn).await.unwrap().ack.unwrap();
    (session, turn_ack.event_id)
}

async fn create_pending_effect(
    store: &AgentSessionStore,
    key: &str,
    strategy: EffectStrategy,
) -> (AgentSessionLiveRecord, EffectEventRequest, EventId) {
    let operation_id = format!("effect-operation-{key}");
    let turn_id = format!("effect-turn-{key}");
    let (session, ready_event) = create_ready(store, key).await;
    let turn = append(
        &session.agent_session_id,
        &format!("event-effect-turn-{key}"),
        "session-api",
        &format!("effect-turn-{key}"),
        "turn/started",
        &turn_id,
        Some(ready_event),
        json!({"operation_id": turn_id}),
    );
    let turn_ack = store.append_event(&turn).await.unwrap().ack.unwrap();
    let tool = append(
        &session.agent_session_id,
        &format!("event-effect-tool-{key}"),
        "runtime-supervisor",
        &format!("effect-tool-{key}"),
        "tool/call-started",
        &format!("tool-{key}"),
        Some(turn_ack.event_id),
        json!({
            "operation_id": operation_id,
            "capability_id": "workspace.files",
            "action_id": "workspace.files/write"
        }),
    );
    let tool_ack = store.append_event(&tool).await.unwrap().ack.unwrap();
    let request = EffectEventRequest {
        agent_session_id: session.agent_session_id.clone(),
        effect_id: format!("effect-{key}"),
        turn_id: OperationId::from(turn_id),
        operation_id: OperationId::from(operation_id),
        owner_domain: "workspace".to_owned(),
        capability_module: CapabilityId::from("workspace.files"),
        action_id: ActionId::from("workspace.files/write"),
        resource_binding_id: None,
        resource_key: Some(format!("workspace:{key}")),
        input_digest: digest('7'),
        recorded_at: 1_788_000_000_010,
        event_id: event_id(&format!("event-effect-started-{key}")),
        producer_id: EventProducerId::from("capability-host"),
        idempotency_key: IdempotencyKey::from(format!("effect-started-{key}")),
        correlation_id: CorrelationId::from(format!("effect-{key}")),
        strategy,
        causation_event_id: Some(tool_ack.event_id),
        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({}))),
    };
    let started = store
        .record_effect_started(request.clone())
        .await
        .unwrap()
        .ack
        .unwrap();
    (session, request, started.event_id)
}

#[tokio::test]
async fn shared_agent_store_schema_and_session_creation_are_exact_and_idempotent() {
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
    let canonical_tables = nomifun_agent_contracts::agent_store_schema_manifest_payload()
        .tables
        .into_iter()
        .map(|table| table.table_name)
        .collect::<BTreeSet<_>>();
    assert_eq!(tables, canonical_tables);
    assert!(tables.len() > 5);
    for owned in [
        "agent_sessions",
        "agent_turns",
        "agent_effects",
        "agent_session_resources",
        "agent_messages",
        "agent_events",
        "agent_session_heads",
        "agent_payloads",
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
        .expect("the non-session canonical Agent Store tables must not be rejected");

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
async fn main_database_pool_is_the_same_canonical_agent_store() {
    let database = nomifun_db::init_database_memory().await.unwrap();
    let store = AgentSessionStore::from_pool(database.pool().clone())
        .await
        .expect("main SQLite must contain the canonical Agent Store generation");
    let created = store
        .create_session(create_request(live_session(session_id()), "main-pool"))
        .await
        .unwrap();
    assert_eq!(
        store
            .read_turn_receipt(
                &created.session.agent_session_id,
                &OperationId("missing-turn".to_owned()),
            )
            .await
            .unwrap()
            .status,
        TurnReceiptStatus::NotFound
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_session_creation_waits_for_the_sqlite_writer_and_both_succeed() {
    let directory = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database(&directory.path().join("parallel-sessions.db"))
        .await
        .unwrap();
    let store = AgentSessionStore::from_pool(database.pool().clone())
        .await
        .unwrap();

    // Hold the SQLite writer lock while both Session opens reach their write
    // boundary. BEGIN IMMEDIATE must wait here; the previous deferred BEGIN
    // read the idempotency index first and then failed its lock upgrade.
    let mut writer = database.pool().begin().await.unwrap();
    sqlx::query("UPDATE users SET updated_at = updated_at")
        .execute(&mut *writer)
        .await
        .unwrap();

    let first_store = store.clone();
    let first = tokio::spawn(async move {
        first_store
            .create_session(create_request(live_session(session_id()), "parallel-first"))
            .await
    });
    let second_store = store.clone();
    let second = tokio::spawn(async move {
        second_store
            .create_session(create_request(live_session(session_id()), "parallel-second"))
            .await
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!first.is_finished());
    assert!(!second.is_finished());
    writer.commit().await.unwrap();

    let first = tokio::time::timeout(std::time::Duration::from_secs(6), first)
        .await
        .expect("first Session open should honor SQLite busy_timeout")
        .expect("first Session task should not panic")
        .expect("first Session should be created");
    let second = tokio::time::timeout(std::time::Duration::from_secs(6), second)
        .await
        .expect("second Session open should honor SQLite busy_timeout")
        .expect("second Session task should not panic")
        .expect("second Session should be created");
    assert_ne!(first.session.agent_session_id, second.session.agent_session_id);
    assert!(!first.duplicate);
    assert!(!second.duplicate);
    database.close().await;
}

#[tokio::test]
async fn session_resource_bindings_are_frozen_with_the_session_and_cannot_change_owner() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let mut session = live_session(session_id());
    session.agent_binding.typed_resource_bindings = vec![TypedResourceBinding {
        binding_id: ResourceBindingId("binding-workspace".to_owned()),
        resource_kind: ResourceKind("workspace".to_owned()),
        resource_id: ResourceId("workspace-1".to_owned()),
        owner_id: owner().principal_id,
        operations: BTreeSet::from(["read".to_owned(), "patch".to_owned()]),
        connection_config_ref: None,
        typed_parameters: BTreeMap::from([("root".to_owned(), "C:/workspace".to_owned())]),
    }];
    let created = store
        .create_session(create_request(session.clone(), "resource-binding"))
        .await
        .unwrap();
    let resources = store
        .session_resources(&created.session.agent_session_id)
        .await
        .unwrap();
    assert_eq!(resources, session.agent_binding.typed_resource_bindings);

    let mut foreign = live_session(session_id());
    foreign.agent_binding.typed_resource_bindings = vec![TypedResourceBinding {
        binding_id: ResourceBindingId("binding-foreign".to_owned()),
        resource_kind: ResourceKind("workspace".to_owned()),
        resource_id: ResourceId("workspace-2".to_owned()),
        owner_id: "another-user".to_owned(),
        operations: BTreeSet::from(["read".to_owned()]),
        connection_config_ref: None,
        typed_parameters: BTreeMap::new(),
    }];
    assert!(matches!(
        store
            .create_session(create_request(foreign, "foreign-resource"))
            .await,
        Err(SessionStoreError::InvalidSession(message))
            if message.contains("foreign resource binding")
    ));
}

#[tokio::test]
async fn session_model_binding_replacement_is_exact_and_preserves_resource_authority() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let mut session = live_session(session_id());
    session.agent_binding.typed_resource_bindings = vec![TypedResourceBinding {
        binding_id: ResourceBindingId("binding-model-workspace".to_owned()),
        resource_kind: ResourceKind("workspace".to_owned()),
        resource_id: ResourceId("workspace-model".to_owned()),
        owner_id: owner().principal_id,
        operations: BTreeSet::from(["read".to_owned(), "patch".to_owned()]),
        connection_config_ref: None,
        typed_parameters: BTreeMap::new(),
    }];
    let created = store
        .create_session(create_request(session, "model-binding-replacement"))
        .await
        .unwrap();
    store
        .append_event(&append(
            &created.session.agent_session_id,
            "event-ready-model-binding-replacement",
            "runtime-supervisor",
            "ready-model-binding-replacement",
            "session/ready",
            "session-model-binding-replacement",
            Some(created.opening_ack.event_id),
            json!({}),
        ))
        .await
        .unwrap();

    let expected = created.session.agent_binding;
    let mut replacement = expected.clone();
    replacement.preset_revision_ref = PresetRevisionRef {
        preset_id: AgentPresetId("model-variant".to_owned()),
        revision: 1,
        revision_digest: digest('c'),
    };
    replacement.resolved_snapshot_ref = ResolvedSnapshotRef {
        snapshot_id: ResolvedSnapshotId("snapshot-model-variant".to_owned()),
        snapshot_digest: digest('d'),
    };
    replacement.binding_version += 1;

    let updated = store
        .replace_session_model_binding(
            &owner(),
            &created.session.agent_session_id,
            &expected,
            replacement.clone(),
        )
        .await
        .unwrap();
    assert_eq!(updated.agent_binding, replacement);
    assert_eq!(
        store
            .session_resources(&created.session.agent_session_id)
            .await
            .unwrap(),
        expected.typed_resource_bindings,
    );

    let mut stale_successor = replacement.clone();
    stale_successor.resolved_snapshot_ref = ResolvedSnapshotRef {
        snapshot_id: ResolvedSnapshotId("snapshot-stale-successor".to_owned()),
        snapshot_digest: digest('e'),
    };
    assert!(matches!(
        store
            .replace_session_model_binding(
                &owner(),
                &created.session.agent_session_id,
                &expected,
                stale_successor,
            )
            .await,
        Err(SessionStoreError::Conflict(message)) if message.contains("changed before")
    ));
}

#[tokio::test]
async fn session_model_binding_replacement_rejects_an_active_turn_and_remote_provenance() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (active, _) = create_turn(&store, "active-model-switch", "active-model-switch").await;
    let expected = active.agent_binding;
    let mut replacement = expected.clone();
    replacement.preset_revision_ref.preset_id = AgentPresetId("active-model-variant".to_owned());
    replacement.preset_revision_ref.revision_digest = digest('c');
    replacement.resolved_snapshot_ref.snapshot_id =
        ResolvedSnapshotId("active-model-snapshot".to_owned());
    replacement.resolved_snapshot_ref.snapshot_digest = digest('d');
    replacement.binding_version += 1;
    assert!(matches!(
        store
            .replace_session_model_binding(
                &owner(),
                &active.agent_session_id,
                &expected,
                replacement,
            )
            .await,
        Err(SessionStoreError::Conflict(message)) if message.contains("active Turn")
    ));

    let mut remote = live_session(session_id());
    remote.remote_binding_provenance = Some(RemoteBindingProvenance {
        remote_binding_id: RemoteBindingId::from("remote-model-binding"),
        binding_version: 1,
    });
    let remote = store
        .create_session(create_request(remote, "remote-model-binding"))
        .await
        .unwrap();
    let expected = remote.session.agent_binding;
    let mut replacement = expected.clone();
    replacement.preset_revision_ref.preset_id = AgentPresetId("remote-model-variant".to_owned());
    replacement.preset_revision_ref.revision_digest = digest('e');
    replacement.resolved_snapshot_ref.snapshot_id =
        ResolvedSnapshotId("remote-model-snapshot".to_owned());
    replacement.resolved_snapshot_ref.snapshot_digest = digest('f');
    replacement.binding_version += 1;
    assert!(matches!(
        store
            .replace_session_model_binding(
                &owner(),
                &remote.session.agent_session_id,
                &expected,
                replacement,
            )
            .await,
        Err(SessionStoreError::Conflict(message)) if message.contains("Remote")
    ));
}

#[tokio::test]
async fn the_same_product_resource_binding_can_be_frozen_into_distinct_sessions() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let binding = TypedResourceBinding {
        binding_id: ResourceBindingId("workspace:default-workspace".to_owned()),
        resource_kind: ResourceKind("workspace".to_owned()),
        resource_id: ResourceId("default-workspace".to_owned()),
        owner_id: owner().principal_id,
        operations: BTreeSet::from(["read".to_owned()]),
        connection_config_ref: None,
        typed_parameters: BTreeMap::new(),
    };
    let mut first = live_session(session_id());
    first.agent_binding.typed_resource_bindings = vec![binding.clone()];
    let mut second = live_session(session_id());
    second.agent_binding.typed_resource_bindings = vec![binding.clone()];

    let first = store.create_session(create_request(first, "shared-resource-first"))
        .await.unwrap().session;
    let second = store.create_session(create_request(second, "shared-resource-second"))
        .await.unwrap().session;

    assert_ne!(first.agent_session_id, second.agent_session_id);
    assert_eq!(store.session_resources(&first.agent_session_id).await.unwrap(), vec![binding.clone()]);
    assert_eq!(store.session_resources(&second.agent_session_id).await.unwrap(), vec![binding]);
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
        "SELECT COUNT(*) FROM agent_events \
         WHERE session_id = ? AND kind IN ('runtime/bound', 'session/ready')",
    )
    .bind(failed_created.session.agent_session_id.as_ref())
    .fetch_one(store.test_pool())
    .await
    .unwrap();
    assert_eq!(event_count, 0);
}

#[tokio::test]
async fn observations_keep_session_head_events_and_messages_in_one_committed_snapshot() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, ready_event) = create_ready(&store, "observation-snapshot").await;
    let turn = append(
        &session.agent_session_id,
        "observation-turn", "session-api", "observation-turn", "turn/started",
        "observation-turn", Some(ready_event), json!({}),
    );
    store.append_event(&turn).await.unwrap();

    // One pool connection makes competing acquisitions interleave predictably.
    // An observation that reacquires between fields can combine old session
    // metadata with a newer head, event page, or message projection.
    let writer = async {
        for index in 0..32 {
            let id = format!("observation-part-{index}");
            let event = append(
                &session.agent_session_id,
                &id, "runtime-supervisor", &id, "message/content-part",
                "observation-message", Some(turn.event_id.clone()),
                json!({"content": "part"}),
            );
            store.append_event(&event).await.unwrap();
        }
    };
    let reader = async {
        let mut observations = Vec::new();
        for _ in 0..32 {
            observations.push(store.observe(&session.agent_session_id, None, 500).await.unwrap());
        }
        observations
    };
    let ((), observations) = tokio::join!(writer, reader);
    for observation in observations {
        assert_eq!(observation.session.next_seq, observation.head.last_seq + 1);
        assert_eq!(observation.next_cursor.seq, observation.head.last_seq);
        assert_eq!(observation.events.last().unwrap().seq, observation.head.last_seq);
        assert!(observation.messages.iter().all(|message| message.last_seq <= observation.head.last_seq));
    }

    let page = store.observe(&session.agent_session_id, None, 1).await.unwrap();
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.next_cursor.seq, page.events[0].seq);
    assert!(page.next_cursor.seq < page.head.last_seq);
    let ahead = nomifun_agent_contracts::SessionEventCursor {
        agent_session_id: session.agent_session_id.clone(),
        seq: page.head.last_seq + 1,
    };
    assert!(matches!(
        store.observe(&session.agent_session_id, Some(&ahead), 10).await,
        Err(SessionStoreError::InvalidEvent(_))
    ));
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
        .messages_after(&session.agent_session_id, 0)
        .await
        .unwrap();
    assert_eq!(before_messages.len(), 2);
    let message = before_messages
        .iter()
        .find(|projection| projection.projection_id == "message:message-1")
        .unwrap();
    assert_eq!(message.projection["content"], "hello");
    assert_eq!(message.projection["part_count"], 1);
    assert!(message.projection.get("events").is_none());

    sqlx::query("DELETE FROM agent_messages WHERE session_id = ?")
        .bind(session.agent_session_id.as_ref())
        .execute(store.test_pool())
        .await
        .unwrap();
    sqlx::query("DELETE FROM agent_session_heads WHERE session_id = ?")
        .bind(session.agent_session_id.as_ref())
        .execute(store.test_pool())
        .await
        .unwrap();
    let rebuilt_head = store
        .rebuild_projections(&session.agent_session_id)
        .await
        .unwrap();
    let rebuilt_messages = store
        .messages_after(&session.agent_session_id, 0)
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

#[test]
fn projection_rejects_embedded_legacy_event_history() {
    let session_id = session_id();
    let existing = crate::MessageProjection {
        session_id: session_id.clone(),
        projection_id: "message:legacy-message".to_owned(),
        first_seq: 3,
        last_seq: 3,
        presentation_intent: "message".to_owned(),
        message_type: None,
        message_status: None,
        projection: json!({
            "projection_id": "message:legacy-message",
            "correlation_id": "legacy-message",
            "presentation_intent": "message",
            "events": [{
                "seq": 3,
                "kind": "message/content-part",
                "kind_version": 1,
                "payload": {"content": "hello"}
            }],
            "state": "streaming",
            "content": "hello"
        }),
        semantic_digest: "legacy-digest".to_owned(),
    };
    let payload = json!({
        "content_digest": digest_bytes(b"hello"),
        "part_count": 1
    });
    let completed = projection_event(
        &session_id,
        4,
        "event-legacy-message-completed",
        "message/completed",
        "legacy-message",
        payload.clone(),
    );

    assert!(
        reduce_agent_messages(Some(existing), &completed, &payload).is_err(),
        "the clean-cut Agent Store must not normalize an old embedded event transcript"
    );
}

#[test]
fn message_source_metadata_is_optional_and_does_not_rewrite_content() {
    let wire = json!({
        "session_id": session_id(), "projection_id": "message:fixture",
        "first_seq": 1, "last_seq": 1, "presentation_intent": "message",
        "projection": {"content": "same content"}, "semantic_digest": "fixture-digest"
    });
    let plain: crate::MessageProjection = serde_json::from_value(wire.clone()).unwrap();
    assert!(plain.message_type.is_none() && plain.message_status.is_none());
    assert_eq!(serde_json::to_value(&plain).unwrap(), wire);
    let mut typed_wire = wire.clone();
    typed_wire["message_type"] = json!("text");
    typed_wire["message_status"] = json!("finish");
    let typed: crate::MessageProjection = serde_json::from_value(typed_wire.clone()).unwrap();
    assert_eq!(typed.projection, plain.projection);
    assert_eq!(typed.semantic_digest, plain.semantic_digest);
    assert_eq!(serde_json::to_value(typed).unwrap(), typed_wire);
}

#[test]
fn projection_keeps_tool_references_and_terminal_effect_summary_without_events() {
    let session_id = session_id();
    let started_payload = json!({
        "operation_id": "operation-1",
        "capability_id": "coding.workspace",
        "action_id": "workspace.write",
        "input": {"content": "not copied into the projection"}
    });
    let started = projection_event(
        &session_id,
        3,
        "event-tool-started",
        "tool/call-started",
        "tool-1",
        started_payload.clone(),
    );
    let tool =
        reduce_agent_messages(None, &started, &started_payload).unwrap();
    let result_payload = json!({
        "operation_id": "operation-1",
        "capability_id": "coding.workspace",
        "action_id": "workspace.write",
        "output": {"content": "not copied into the projection"},
        "output_ref": {
            "artifact_id": "artifact-1",
            "normalized_relative_path": "results/artifact-1",
            "digest": "abc123"
        }
    });
    let result = projection_event(
        &session_id,
        4,
        "event-tool-result",
        "tool/result-recorded",
        "tool-1",
        result_payload.clone(),
    );
    let tool =
        reduce_agent_messages(Some(tool), &result, &result_payload).unwrap();

    assert!(tool.projection.get("events").is_none());
    assert_eq!(
        tool.projection["tool_summary"]["action_id"],
        "workspace.write"
    );
    assert!(
        tool.projection["tool_summary"]["result_digest"].is_string(),
        "tool output must be summarized by digest"
    );
    assert!(tool.projection["tool_summary"].get("input").is_none());
    assert!(tool.projection["tool_summary"].get("output").is_none());
    assert_eq!(
        tool.projection["reference"]["output_ref"]["artifact_id"],
        "artifact-1"
    );

    let effect_started_payload = json!({
        "effect_id": "effect-1",
        "operation_id": "operation-1",
        "capability_id": "coding.workspace",
        "action_id": "workspace.write"
    });
    let effect_started = projection_event(
        &session_id,
        5,
        "event-effect-started",
        "effect/started",
        "effect-1",
        effect_started_payload.clone(),
    );
    let effect =
        reduce_agent_messages(None, &effect_started, &effect_started_payload).unwrap();
    let effect_succeeded_payload = json!({
        "receipt": {"artifact_id": "artifact-1"}
    });
    let effect_succeeded = projection_event(
        &session_id,
        6,
        "event-effect-succeeded",
        "effect/succeeded",
        "effect-1",
        effect_succeeded_payload.clone(),
    );
    let effect = reduce_agent_messages(
        Some(effect),
        &effect_succeeded,
        &effect_succeeded_payload,
    )
    .unwrap();

    assert!(effect.projection.get("events").is_none());
    assert_eq!(effect.projection["terminal_effect"]["state"], "succeeded");
    assert!(
        effect.projection["terminal_effect"]["result_digest"].is_string(),
        "effect receipt must be summarized by digest"
    );
    assert_eq!(
        effect.projection["reference"]["last_event_id"],
        "event-effect-succeeded"
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
async fn initial_turn_is_atomic_replayable_and_cannot_be_admitted_twice() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, _) = create_ready(&store, "initial-only").await;
    let session_id = &session.agent_session_id;
    let producer = EventProducerId("session-api".to_owned());
    let key = IdempotencyKey("initial-only-key".to_owned());
    let operation = OperationId("turn-initial-only".to_owned());
    let input = StrictJsonValue(json!({"content": "hello"}));

    let first = store
        .start_initial_turn(
            session_id,
            producer.clone(),
            key.clone(),
            operation.clone(),
            input.clone(),
        )
        .await
        .unwrap();
    assert!(!first.1.duplicate);

    let replay = store
        .start_initial_turn(
            session_id,
            producer.clone(),
            key,
            operation.clone(),
            input,
        )
        .await
        .unwrap();
    assert!(replay.0.duplicate);
    assert!(replay.1.duplicate);

    store
        .cancel_active_turn(
            session_id,
            IdempotencyKey("cancel-initial-only".to_owned()),
            producer.clone(),
        )
        .await
        .unwrap();
    let cursor_before_rejected = store.current_cursor(session_id).await.unwrap();
    let second = store
        .start_initial_turn(
            session_id,
            producer.clone(),
            IdempotencyKey("different-initial-key".to_owned()),
            OperationId("turn-second-initial".to_owned()),
            StrictJsonValue(json!({"content": "must not be admitted"})),
        )
        .await
        .unwrap_err();
    assert!(matches!(second, SessionStoreError::Conflict(_)));
    assert_eq!(
        store.current_cursor(session_id).await.unwrap(),
        cursor_before_rejected,
        "a rejected second initial delivery must not append any fact"
    );

    let ordinary = store
        .start_turn(
            session_id,
            producer,
            IdempotencyKey("ordinary-follow-up".to_owned()),
            OperationId("turn-ordinary-follow-up".to_owned()),
            StrictJsonValue(json!({"content": "ordinary follow-up"})),
        )
        .await
        .unwrap();
    assert!(!ordinary.1.duplicate);
}

#[tokio::test]
async fn turn_receipt_is_running_without_a_terminal_fact_and_does_not_infer_from_text() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, turn_event) = create_turn(&store, "turn-receipt-running", "turn-running").await;
    let content = append(
        &session.agent_session_id,
        "turn-receipt-content",
        "runtime-supervisor",
        "turn-receipt-content",
        "message/content-part",
        "turn-running",
        Some(turn_event),
        json!({"content": "ordinary text is not a turn terminal"}),
    );
    store.append_event(&content).await.unwrap();

    let receipt = store
        .read_turn_receipt(
            &session.agent_session_id,
            &OperationId::from("turn-running"),
        )
        .await
        .unwrap();
    assert_eq!(receipt.status, TurnReceiptStatus::Running);
    assert!(receipt.started_event.is_some());
    assert!(receipt.terminal_event.is_none());

    let missing_operation = store
        .read_turn_receipt(
            &session.agent_session_id,
            &OperationId::from("turn-does-not-exist"),
        )
        .await
        .unwrap();
    assert_eq!(missing_operation.status, TurnReceiptStatus::NotFound);

    let missing_session = AgentSessionId::from(
        "0199a8c0-0000-7000-8000-000000000099",
    );
    assert!(matches!(
        store
            .read_turn_receipt(&missing_session, &OperationId::from("turn-running"))
            .await,
        Err(SessionStoreError::NotFound(_))
    ));
}

#[tokio::test]
async fn turn_receipt_reports_each_canonical_terminal_state() {
    for (key, operation, kind, producer, payload, expected) in [
        (
            "turn-receipt-completed",
            "turn-completed",
            "turn/completed",
            "runtime-supervisor",
            json!({}),
            TurnReceiptStatus::Completed,
        ),
        (
            "turn-receipt-failed",
            "turn-failed",
            "turn/failed",
            "runtime-supervisor",
            json!({"error": "model failed"}),
            TurnReceiptStatus::Failed,
        ),
        (
            "turn-receipt-cancelled",
            "turn-cancelled",
            "turn/cancelled",
            "session-api",
            json!({"target_operation_id": "turn-cancelled"}),
            TurnReceiptStatus::Cancelled,
        ),
    ] {
        let store = AgentSessionStore::open_in_memory().await.unwrap();
        let (session, turn_event) = create_turn(&store, key, operation).await;
        let terminal = append(
            &session.agent_session_id,
            &format!("event-{key}-terminal"),
            producer,
            &format!("{key}-terminal"),
            kind,
            operation,
            Some(turn_event),
            payload,
        );
        store.append_event(&terminal).await.unwrap();

        let receipt = store
            .read_turn_receipt(&session.agent_session_id, &OperationId::from(operation))
            .await
            .unwrap();
        assert_eq!(receipt.status, expected);
        assert_eq!(
            receipt.terminal_event.as_ref().map(|event| event.kind.0.as_str()),
            Some(kind)
        );
    }
}

#[tokio::test]
async fn turn_receipt_terminal_fence_is_monotonic_across_replays_and_late_events() {
    for (key, operation, first_kind, first_producer, first_payload, expected) in [
        (
            "turn-receipt-fence-completed",
            "turn-fence-completed",
            "turn/completed",
            "runtime-supervisor",
            json!({}),
            TurnReceiptStatus::Completed,
        ),
        (
            "turn-receipt-fence-failed",
            "turn-fence-failed",
            "turn/failed",
            "runtime-supervisor",
            json!({"error": "first terminal"}),
            TurnReceiptStatus::Failed,
        ),
        (
            "turn-receipt-fence-cancelled",
            "turn-fence-cancelled",
            "turn/cancelled",
            "session-api",
            json!({"target_operation_id": "turn-fence-cancelled"}),
            TurnReceiptStatus::Cancelled,
        ),
    ] {
        let store = AgentSessionStore::open_in_memory().await.unwrap();
        let (session, turn_event) = create_turn(&store, key, operation).await;
        let first_terminal = append(
            &session.agent_session_id,
            &format!("event-{key}-first-terminal"),
            first_producer,
            &format!("{key}-first-terminal"),
            first_kind,
            operation,
            Some(turn_event.clone()),
            first_payload,
        );
        let first_result = store.append_event(&first_terminal).await.unwrap();
        let first_event_id = first_result.ack.as_ref().unwrap().event_id.clone();

        let replay = store.append_event(&first_terminal).await.unwrap();
        assert!(replay.duplicate);
        assert_eq!(
            replay.ack.as_ref().unwrap().event_id,
            first_event_id
        );

        let competing = append(
            &session.agent_session_id,
            &format!("event-{key}-competing-terminal"),
            "runtime-supervisor",
            &format!("{key}-competing-terminal"),
            if first_kind == "turn/completed" {
                "turn/failed"
            } else {
                "turn/completed"
            },
            operation,
            Some(turn_event.clone()),
            json!({"error": "late competing terminal"}),
        );
        assert!(matches!(
            store.append_event(&competing).await,
            Err(SessionStoreError::Conflict(message))
                if message.contains("terminal fence")
        ));

        let late_start = append(
            &session.agent_session_id,
            &format!("event-{key}-late-start"),
            "session-api",
            &format!("{key}-late-start"),
            "turn/started",
            operation,
            Some(first_terminal.event_id.clone()),
            json!({"operation_id": operation, "retry": true}),
        );
        assert!(matches!(
            store.append_event(&late_start).await,
            Err(SessionStoreError::Conflict(_))
        ));

        let receipt = store
            .read_turn_receipt(&session.agent_session_id, &OperationId::from(operation))
            .await
            .unwrap();
        assert_eq!(receipt.status, expected);
        assert_eq!(
            receipt.terminal_event.as_ref().unwrap().event_id,
            first_event_id
        );
        let head = store.head(&session.agent_session_id).await.unwrap();
        assert_eq!(head.status, "ready");
        assert!(head.active_turn_id.is_none());
        let terminal_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_events \
             WHERE session_id = ? AND correlation_id = ? \
               AND kind IN ('turn/completed', 'turn/failed', 'turn/cancelled')",
        )
        .bind(session.agent_session_id.as_ref())
        .bind(operation)
        .fetch_one(store.test_pool())
        .await
        .unwrap();
        assert_eq!(terminal_count, 1);
    }
}

#[tokio::test]
async fn concurrent_turn_terminal_attempts_have_one_durable_winner() {
    let store = AgentSessionStore::open_in_memory_with_connections(4)
        .await
        .unwrap();
    let (session, turn_event) = create_turn(&store, "turn-receipt-concurrent", "turn-concurrent").await;
    let completed = append(
        &session.agent_session_id,
        "event-turn-receipt-concurrent-completed",
        "runtime-supervisor",
        "turn-receipt-concurrent-completed",
        "turn/completed",
        "turn-concurrent",
        Some(turn_event.clone()),
        json!({}),
    );
    let failed = append(
        &session.agent_session_id,
        "event-turn-receipt-concurrent-failed",
        "runtime-supervisor",
        "turn-receipt-concurrent-failed",
        "turn/failed",
        "turn-concurrent",
        Some(turn_event),
        json!({"error": "competing terminal"}),
    );

    let (completed_result, failed_result) =
        tokio::join!(store.append_event(&completed), store.append_event(&failed));
    assert_ne!(completed_result.is_ok(), failed_result.is_ok());
    assert!(
        matches!(
            completed_result.as_ref().err(),
            Some(SessionStoreError::Conflict(_))
        ) || matches!(
            failed_result.as_ref().err(),
            Some(SessionStoreError::Conflict(_))
        )
    );

    let receipt = store
        .read_turn_receipt(
            &session.agent_session_id,
            &OperationId::from("turn-concurrent"),
        )
        .await
        .unwrap();
    assert!(matches!(
        receipt.status,
        TurnReceiptStatus::Completed | TurnReceiptStatus::Failed
    ));
    assert!(receipt.terminal_event.is_some());
    assert_eq!(
        store.head(&session.agent_session_id).await.unwrap().status,
        "ready"
    );
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
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_payloads WHERE session_id = ?")
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
        .messages_after(&session.agent_session_id, 0)
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
async fn effect_store_rejects_read_only_lifecycles_and_managed_uncertainty() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, ready_event) = create_ready(&store, "effect-strategy").await;
    let turn = append(
        &session.agent_session_id,
        "event-effect-strategy-turn",
        "session-api",
        "effect-strategy-turn",
        "turn/started",
        "turn-effect-strategy",
        Some(ready_event),
        json!({}),
    );
    let turn_ack = store.append_event(&turn).await.unwrap().ack.unwrap();
    let tool = append(
        &session.agent_session_id,
        "event-effect-strategy-tool",
        "runtime-supervisor",
        "effect-strategy-tool",
        "tool/call-started",
        "tool-effect-strategy",
        Some(turn_ack.event_id),
        json!({
            "operation_id": "effect-operation-managed",
            "capability_id": "workspace.files",
            "action_id": "workspace.files/write"
        }),
    );
    let tool_ack = store.append_event(&tool).await.unwrap().ack.unwrap();

    let read_only = EffectEventRequest {
        agent_session_id: session.agent_session_id.clone(),
        effect_id: "effect-read-only".to_owned(),
        turn_id: OperationId("turn-effect-strategy".to_owned()),
        operation_id: OperationId("effect-operation-read-only".to_owned()),
        owner_domain: "workspace".to_owned(),
        capability_module: CapabilityId("workspace.files".to_owned()),
        action_id: ActionId("workspace.files/read".to_owned()),
        resource_binding_id: None,
        resource_key: None,
        input_digest: digest('8'),
        recorded_at: 10,
        event_id: event_id("event-effect-read-only-started"),
        producer_id: EventProducerId("capability-host".to_owned()),
        idempotency_key: IdempotencyKey("effect-read-only".to_owned()),
        correlation_id: CorrelationId("effect-read-only".to_owned()),
        strategy: EffectStrategy::ReadOnly,
        causation_event_id: Some(tool_ack.event_id.clone()),
        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({}))),
    };
    assert!(matches!(
        store.record_effect_started(read_only).await,
        Err(SessionStoreError::InvalidEvent(message))
            if message == "read-only operations must not emit effect lifecycle events"
    ));

    let managed = EffectEventRequest {
        agent_session_id: session.agent_session_id.clone(),
        effect_id: "effect-managed".to_owned(),
        turn_id: OperationId("turn-effect-strategy".to_owned()),
        operation_id: OperationId("effect-operation-managed".to_owned()),
        owner_domain: "workspace".to_owned(),
        capability_module: CapabilityId("workspace.files".to_owned()),
        action_id: ActionId("workspace.files/write".to_owned()),
        resource_binding_id: None,
        resource_key: Some("workspace:test".to_owned()),
        input_digest: digest('9'),
        recorded_at: 11,
        event_id: event_id("event-effect-managed-started"),
        producer_id: EventProducerId("capability-host".to_owned()),
        idempotency_key: IdempotencyKey("effect-managed".to_owned()),
        correlation_id: CorrelationId("effect-managed".to_owned()),
        strategy: EffectStrategy::ManagedEffect,
        causation_event_id: Some(tool_ack.event_id),
        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({}))),
    };
    let started_ack = store
        .record_effect_started(managed.clone())
        .await
        .unwrap()
        .ack
        .unwrap();
    let uncertain = EffectEventRequest {
        event_id: event_id("event-effect-managed-uncertain"),
        producer_id: EventProducerId("capability-owner".to_owned()),
        causation_event_id: Some(started_ack.event_id.clone()),
        ..managed.clone()
    };
    assert!(matches!(
        store
            .record_effect_terminal(uncertain, EffectTerminalState::Uncertain)
            .await,
        Err(SessionStoreError::InvalidEvent(message))
            if message == "managed effects require process-restart proof before becoming uncertain"
    ));

    let failed = EffectEventRequest {
        event_id: event_id("event-effect-managed-failed"),
        producer_id: EventProducerId("capability-owner".to_owned()),
        causation_event_id: Some(started_ack.event_id),
        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
            "error": "known local failure"
        }))),
        ..managed
    };
    store
        .record_effect_terminal(failed, EffectTerminalState::Failed)
        .await
        .unwrap();
    let effect = store
        .read_effect(&session.agent_session_id, "effect-managed")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(effect.state, AgentEffectState::Rejected);
    assert_eq!(effect.action_id.as_ref(), "workspace.files/write");
}

#[tokio::test]
async fn effect_store_requires_exact_tool_causation_and_immutable_terminal_identity() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, ready_event) = create_ready(&store, "effect-identity").await;
    let turn = append(
        &session.agent_session_id,
        "event-effect-identity-turn",
        "session-api",
        "effect-identity-turn",
        "turn/started",
        "turn-effect-identity",
        Some(ready_event),
        json!({}),
    );
    let turn_ack = store.append_event(&turn).await.unwrap().ack.unwrap();
    let tool = append(
        &session.agent_session_id,
        "event-effect-identity-tool",
        "capability-host",
        "effect-identity-tool",
        "tool/call-started",
        "tool-effect-identity",
        Some(turn_ack.event_id.clone()),
        json!({
            "operation_id": "effect-identity-operation",
            "capability_id": "workspace.files",
            "action_id": "workspace.files/write"
        }),
    );
    let tool_ack = store.append_event(&tool).await.unwrap().ack.unwrap();
    let unrelated_tool = append(
        &session.agent_session_id,
        "event-effect-unrelated-tool",
        "capability-host",
        "effect-unrelated-tool",
        "tool/call-started",
        "tool-effect-unrelated",
        Some(turn_ack.event_id.clone()),
        json!({
            "operation_id": "other-operation",
            "capability_id": "workspace.files",
            "action_id": "workspace.files/delete"
        }),
    );
    let unrelated_tool_ack = store
        .append_event(&unrelated_tool)
        .await
        .unwrap()
        .ack
        .unwrap();

    let started = EffectEventRequest {
        agent_session_id: session.agent_session_id.clone(),
        effect_id: "effect-identity-1".to_owned(),
        turn_id: OperationId("turn-effect-identity".to_owned()),
        operation_id: OperationId("effect-identity-operation".to_owned()),
        owner_domain: "workspace".to_owned(),
        capability_module: CapabilityId("workspace.files".to_owned()),
        action_id: ActionId("workspace.files/write".to_owned()),
        resource_binding_id: None,
        resource_key: Some("workspace:file.txt".to_owned()),
        input_digest: digest('6'),
        recorded_at: 30,
        event_id: event_id("event-effect-identity-started"),
        producer_id: EventProducerId("capability-host".to_owned()),
        idempotency_key: IdempotencyKey("effect-identity-idem".to_owned()),
        correlation_id: CorrelationId("effect-identity-1".to_owned()),
        strategy: EffectStrategy::ManagedEffect,
        causation_event_id: Some(tool_ack.event_id.clone()),
        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({}))),
    };

    let mut wrong_kind_cause = started.clone();
    wrong_kind_cause.event_id = event_id("event-effect-wrong-kind-cause");
    wrong_kind_cause.causation_event_id = Some(turn_ack.event_id);
    assert!(store.record_effect_started(wrong_kind_cause).await.is_err());

    let mut cross_tool_cause = started.clone();
    cross_tool_cause.event_id = event_id("event-effect-cross-tool-cause");
    cross_tool_cause.causation_event_id = Some(unrelated_tool_ack.event_id.clone());
    assert!(store.record_effect_started(cross_tool_cause).await.is_err());

    let started_ack = store
        .record_effect_started(started.clone())
        .await
        .unwrap()
        .ack
        .unwrap();
    for field in [
        "effect_id",
        "turn_id",
        "operation_id",
        "owner_domain",
        "capability_module",
        "action_id",
        "input_digest",
        "strategy",
        "resource_binding_id",
        "resource_key",
        "causation_event_id",
    ] {
        let mut terminal = EffectEventRequest {
            event_id: event_id(&format!("event-effect-identity-mutated-{field}")),
            producer_id: EventProducerId("owning-plugin".to_owned()),
            causation_event_id: Some(started_ack.event_id.clone()),
            recorded_at: 31,
            ..started.clone()
        };
        match field {
            "effect_id" => terminal.effect_id = "effect-identity-other".to_owned(),
            "turn_id" => terminal.turn_id = OperationId("turn-other".to_owned()),
            "operation_id" => terminal.operation_id = OperationId("operation-other".to_owned()),
            "owner_domain" => terminal.owner_domain = "other".to_owned(),
            "capability_module" => {
                terminal.capability_module = CapabilityId("workspace.vcs".to_owned())
            }
            "action_id" => terminal.action_id = ActionId("workspace.files/delete".to_owned()),
            "input_digest" => terminal.input_digest = digest('7'),
            "strategy" => terminal.strategy = EffectStrategy::ExternalUncertainEffect,
            "resource_binding_id" => {
                terminal.resource_binding_id =
                    Some(ResourceBindingId("binding-effect-identity".to_owned()))
            }
            "resource_key" => terminal.resource_key = Some("workspace:other.txt".to_owned()),
            "causation_event_id" => {
                terminal.causation_event_id = Some(unrelated_tool_ack.event_id.clone())
            }
            _ => unreachable!(),
        }
        assert!(
            store
                .record_effect_terminal(terminal, EffectTerminalState::Succeeded)
                .await
                .is_err(),
            "mutating {field} must fail closed"
        );
    }

    let terminal = EffectEventRequest {
        event_id: event_id("event-effect-identity-succeeded"),
        producer_id: EventProducerId("owning-plugin".to_owned()),
        causation_event_id: Some(started_ack.event_id),
        recorded_at: 32,
        ..started
    };
    store
        .record_effect_terminal(terminal, EffectTerminalState::Succeeded)
        .await
        .unwrap();
}

#[tokio::test]
async fn external_uncertain_effect_is_terminal_until_owning_plugin_reconciles() {
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
        json!({
            "operation_id": "effect-operation-1",
            "capability_id": "workspace.files",
            "action_id": "workspace.files/write"
        }),
    );
    let tool_ack = store.append_event(&tool).await.unwrap().ack.unwrap();

    let started = EffectEventRequest {
        agent_session_id: session.agent_session_id.clone(),
        effect_id: "effect-1".to_owned(),
        turn_id: OperationId("turn-effect".to_owned()),
        operation_id: OperationId("effect-operation-1".to_owned()),
        owner_domain: "workspace".to_owned(),
        capability_module: CapabilityId("workspace.files".to_owned()),
        action_id: ActionId("workspace.files/write".to_owned()),
        resource_binding_id: None,
        resource_key: Some("workspace:external".to_owned()),
        input_digest: digest('a'),
        recorded_at: 20,
        event_id: event_id("event-effect-started"),
        producer_id: EventProducerId("capability-host".to_owned()),
        idempotency_key: IdempotencyKey("effect-idem".to_owned()),
        correlation_id: CorrelationId("effect-1".to_owned()),
        strategy: EffectStrategy::ExternalUncertainEffect,
        causation_event_id: Some(tool_ack.event_id),
        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
            "strategy": "external_uncertain_effect"
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
        strategy: EffectStrategy::ExternalUncertainEffect,
        ..started.clone()
    };
    assert!(store.record_effect_started(retry).await.is_err());

    let competing = EffectEventRequest {
        effect_id: "effect-2".to_owned(),
        operation_id: OperationId("effect-operation-2".to_owned()),
        event_id: event_id("event-effect-competing"),
        producer_id: EventProducerId("capability-host-competing".to_owned()),
        idempotency_key: IdempotencyKey("effect-competing".to_owned()),
        correlation_id: CorrelationId("effect-2".to_owned()),
        ..started.clone()
    };
    assert!(
        store.record_effect_started(competing).await.is_err(),
        "an unknown external effect must fence a new effect on the same resource"
    );

    let wrong_reconcile_identity = EffectEventRequest {
        event_id: event_id("event-effect-reconciled-wrong-identity"),
        producer_id: EventProducerId("owning-plugin".to_owned()),
        action_id: ActionId("workspace.files/delete".to_owned()),
        causation_event_id: Some(uncertain_ack.event_id.clone()),
        ..started.clone()
    };
    assert!(
        store
            .reconcile_effect(
                wrong_reconcile_identity,
                EffectReconcileOutcome::StillUncertain
            )
            .await
            .is_err()
    );
    let wrong_reconcile_cause = EffectEventRequest {
        event_id: event_id("event-effect-reconciled-wrong-cause"),
        producer_id: EventProducerId("owning-plugin".to_owned()),
        causation_event_id: Some(started.event_id.clone()),
        ..started.clone()
    };
    assert!(
        store
            .reconcile_effect(wrong_reconcile_cause, EffectReconcileOutcome::StillUncertain)
            .await
            .is_err()
    );

    let reconcile = EffectEventRequest {
        event_id: event_id("event-effect-reconciled"),
        producer_id: EventProducerId("owning-plugin".to_owned()),
        causation_event_id: Some(uncertain_ack.event_id.clone()),
        ..started.clone()
    };
    store
        .reconcile_effect(reconcile, EffectReconcileOutcome::StillUncertain)
        .await
        .unwrap();
    let effect = store
        .read_effect(&session.agent_session_id, "effect-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(effect.state, AgentEffectState::Unknown);
    assert!(effect.terminal_event_id.is_some());
    let repeated_reconcile = EffectEventRequest {
        event_id: event_id("event-effect-reconciled-again"),
        producer_id: EventProducerId("owning-plugin".to_owned()),
        idempotency_key: IdempotencyKey("effect-reconcile-again".to_owned()),
        causation_event_id: Some(uncertain_ack.event_id),
        ..started
    };
    assert!(matches!(
        store
            .reconcile_effect(repeated_reconcile, EffectReconcileOutcome::StillUncertain)
            .await,
        Err(SessionStoreError::InvalidEvent(message))
            if message == "effect reconciliation is already committed"
    ));
    let projections = store
        .messages_after(&session.agent_session_id, 0)
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
        enabled_capabilities: BTreeMap::<CapabilityId, RuntimeCapabilityExecutionContract>::new(),

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
    assert_eq!(forked.child_cursor.seq, 3);
    assert_eq!(store.head(&child_id).await.unwrap().status, "ready");
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
    store.fence_delete(&delete).await.unwrap();
    store
        .complete_delete(&delete, 1_788_000_000_300)
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
    assert_eq!(
        store
            .get_deleting_session(&session.agent_session_id)
            .await
            .unwrap()
            .agent_session_id,
        session.agent_session_id
    );

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

    let deleted = store
        .complete_delete(
            &command,
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
        "agent_events",
        "agent_payloads",
        "agent_session_heads",
        "agent_messages",
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

#[tokio::test]
async fn interrupted_delete_requires_owner_cleanup_before_explicit_completion() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, _) = create_ready(&store, "delete-recovery").await;
    let command = DeleteAgentSessionCommand {
        operation_id: OperationId("delete-before-restart".to_owned()),
        agent_session_id: session.agent_session_id.clone(),
        owner_ref: owner(),
        requested_at: 1_788_000_002_000,
    };
    store.fence_delete(&command).await.unwrap();
    let reentered = store.fence_delete(&command).await.unwrap();
    assert_eq!(reentered.live.agent_session_id, session.agent_session_id);
    assert!(store.inspect_tombstone(&session.agent_session_id).await.unwrap().is_none());
    let completed = store
        .complete_delete(&command, 1_788_000_002_100)
        .await
        .unwrap();
    assert_eq!(completed.operation_id, command.operation_id);
    assert_eq!(completed.tombstone.agent_session_id, session.agent_session_id);
}

#[tokio::test]
async fn delete_fence_allows_existing_effect_to_settle_but_blocks_new_work() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, started, started_event_id) =
        create_pending_effect(&store, "delete-effect", EffectStrategy::ManagedEffect).await;
    let command = DeleteAgentSessionCommand {
        operation_id: OperationId::from("delete-effect-operation"),
        agent_session_id: session.agent_session_id.clone(),
        owner_ref: owner(),
        requested_at: 1_788_000_003_000,
    };
    store.fence_delete(&command).await.unwrap();
    let blockers = store
        .delete_blockers(&session.agent_session_id)
        .await
        .unwrap();
    assert_eq!(blockers.effects.len(), 1);
    assert_eq!(blockers.effects[0].state, AgentEffectState::Pending);
    assert!(matches!(
        store
            .complete_delete(&command, 1_788_000_003_100)
            .await,
        Err(SessionStoreError::Conflict(_))
    ));

    let terminal = EffectEventRequest {
        recorded_at: 1_788_000_003_050,
        event_id: event_id("event-effect-delete-effect-succeeded"),
        producer_id: EventProducerId::from("owning-plugin"),
        causation_event_id: Some(started_event_id),
        payload: SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({
            "receipt": "committed-before-delete-fence"
        }))),
        ..started
    };
    store
        .record_effect_terminal(terminal, EffectTerminalState::Succeeded)
        .await
        .unwrap();
    assert!(store
        .delete_blockers(&session.agent_session_id)
        .await
        .unwrap()
        .is_empty());
    store
        .complete_delete(&command, 1_788_000_003_100)
        .await
        .unwrap();
}

#[tokio::test]
async fn unknown_effect_must_be_explicitly_reconciled_while_delete_is_fenced() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, started, started_event_id) = create_pending_effect(
        &store,
        "delete-unknown-effect",
        EffectStrategy::ExternalUncertainEffect,
    )
    .await;
    let command = DeleteAgentSessionCommand {
        operation_id: OperationId::from("delete-unknown-effect-operation"),
        agent_session_id: session.agent_session_id.clone(),
        owner_ref: owner(),
        requested_at: 1_788_000_004_000,
    };
    store.fence_delete(&command).await.unwrap();
    let uncertain = EffectEventRequest {
        recorded_at: 1_788_000_004_010,
        event_id: event_id("event-delete-effect-uncertain"),
        producer_id: EventProducerId::from("runtime-supervisor"),
        causation_event_id: Some(started_event_id),
        ..started.clone()
    };
    let uncertain_event_id = store
        .record_effect_terminal(uncertain, EffectTerminalState::Uncertain)
        .await
        .unwrap()
        .ack
        .unwrap()
        .event_id;
    assert_eq!(
        store
            .delete_blockers(&session.agent_session_id)
            .await
            .unwrap()
            .effects[0]
            .state,
        AgentEffectState::Unknown
    );
    assert!(store
        .complete_delete(&command, 1_788_000_004_020)
        .await
        .is_err());

    let reconcile = EffectEventRequest {
        recorded_at: 1_788_000_004_015,
        event_id: event_id("event-delete-effect-reconciled"),
        producer_id: EventProducerId::from("owning-plugin"),
        causation_event_id: Some(uncertain_event_id),
        ..started
    };
    store
        .reconcile_effect(
            reconcile,
            EffectReconcileOutcome::ConfirmedFailed {
                error: nomifun_agent_contracts::CanonicalErrorCode::from(
                    "EXTERNAL_EFFECT_CONFIRMED_FAILED",
                ),
            },
        )
        .await
        .unwrap();
    assert!(store
        .delete_blockers(&session.agent_session_id)
        .await
        .unwrap()
        .is_empty());
    store
        .complete_delete(&command, 1_788_000_004_020)
        .await
        .unwrap();
}

#[tokio::test]
async fn restart_quarantines_managed_pending_effect_without_claiming_failure() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, _, _) =
        create_pending_effect(&store, "managed-restart", EffectStrategy::ManagedEffect).await;
    let command = DeleteAgentSessionCommand {
        operation_id: OperationId::from("delete-managed-restart"),
        agent_session_id: session.agent_session_id.clone(),
        owner_ref: owner(),
        requested_at: 1_788_000_004_100,
    };
    store.fence_delete(&command).await.unwrap();
    assert_eq!(
        store
            .quarantine_pending_effects_for_delete(
                &owner(),
                &session.agent_session_id,
                1_788_000_004_110,
            )
            .await
            .unwrap(),
        1
    );
    let blockers = store
        .delete_blockers(&session.agent_session_id)
        .await
        .unwrap();
    assert_eq!(blockers.effects[0].state, AgentEffectState::Unknown);
    store
        .override_unknown_effect_for_delete(
            &owner(),
            &session.agent_session_id,
            &blockers.effects[0].effect_id,
            &digest('d'),
            1_788_000_004_120,
        )
        .await
        .unwrap();
    assert!(store
        .delete_blockers(&session.agent_session_id)
        .await
        .unwrap()
        .is_empty());
    let retained_state: String = sqlx::query_scalar(
        "SELECT state FROM agent_effects WHERE session_id = ? AND effect_id = ?",
    )
    .bind(session.agent_session_id.as_ref())
    .bind(&blockers.effects[0].effect_id)
    .fetch_one(store.test_pool())
    .await
    .unwrap();
    assert_eq!(retained_state, "unknown", "override must not falsify outcome");
}

#[tokio::test]
async fn resource_cleanup_uncertainty_survives_store_restart_and_blocks_purge() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, _) = create_ready(&store, "delete-cleanup-unknown").await;
    let command = DeleteAgentSessionCommand {
        operation_id: OperationId::from("delete-cleanup-unknown-operation"),
        agent_session_id: session.agent_session_id.clone(),
        owner_ref: owner(),
        requested_at: 1_788_000_005_000,
    };
    store.fence_delete(&command).await.unwrap();
    store
        .record_resource_cleanup_started(&session.agent_session_id, "ssh")
        .await
        .unwrap();
    store
        .record_resource_cleanup_uncertain(
            &session.agent_session_id,
            "ssh",
            1_788_000_005_010,
        )
        .await
        .unwrap();

    let restarted = AgentSessionStore::from_pool(store.test_pool().clone())
        .await
        .unwrap();
    let blockers = restarted
        .delete_blockers(&session.agent_session_id)
        .await
        .unwrap();
    assert_eq!(
        blockers.resource_cleanup_uncertainties,
        vec![crate::ResourceCleanupUncertainty {
            owner_domain: "ssh".to_owned(),
            recorded_at: 1_788_000_005_010,
        }]
    );
    assert!(matches!(
        restarted
            .complete_delete(&command, 1_788_000_005_020)
            .await,
        Err(SessionStoreError::Conflict(message)) if message.contains("resource cleanup uncertainty")
    ));
    assert!(restarted
        .inspect_tombstone(&session.agent_session_id)
        .await
        .unwrap()
        .is_none());
    assert!(restarted
        .get_deleting_session(&session.agent_session_id)
        .await
        .is_ok());

    restarted
        .override_resource_cleanup_for_delete(
            &owner(),
            &session.agent_session_id,
            "ssh",
            &digest('c'),
            1_788_000_005_015,
        )
        .await
        .unwrap();
    assert!(matches!(
        restarted
            .record_resource_cleanup_succeeded(&session.agent_session_id, "ssh")
            .await,
        Err(SessionStoreError::Conflict(_))
    ), "manual risk acceptance must not be rewritten as cleanup success");
    assert!(restarted
        .delete_blockers(&session.agent_session_id)
        .await
        .unwrap()
        .is_empty());
    restarted
        .complete_delete(&command, 1_788_000_005_020)
        .await
        .unwrap();
    let reopened = AgentSessionStore::from_pool(restarted.test_pool().clone())
        .await
        .unwrap();
    let audits = reopened
        .deletion_audits(&owner(), &session.agent_session_id)
        .await
        .unwrap();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].target_kind, "resource_cleanup");
    assert_eq!(audits[0].target_id, "ssh");
    assert_eq!(audits[0].reason_digest, digest('c'));
    assert_eq!(audits[0].recorded_at, 1_788_000_005_015);
    let private_event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_events WHERE session_id = ?",
    )
    .bind(session.agent_session_id.as_ref())
    .fetch_one(reopened.test_pool())
    .await
    .unwrap();
    assert_eq!(private_event_count, 0, "tombstone must purge private events");
}

#[tokio::test]
async fn canonical_autowork_config_is_owner_scoped_cas_and_boot_listable() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, _) = create_ready(&store, "autowork-config").await;
    assert_eq!(
        store
            .automation_config(&session.agent_session_id)
            .await
            .unwrap(),
        crate::AgentSessionAutomationConfig::default()
    );
    let command = crate::CommitAgentSessionAutomationConfig {
        agent_session_id: session.agent_session_id.clone(),
        owner_ref: owner(),
        expected_revision: 0,
        enabled: true,
        tag: Some("release".to_owned()),
        max_requirements: Some(3),
        operation_id: Some("autowork-config-enable".to_owned()),
        recorded_at: 1_788_000_006_000,
    };
    let saved = store
        .commit_automation_config(command.clone())
        .await
        .unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(
        store
            .commit_automation_config(command.clone())
            .await
            .unwrap(),
        saved
    );
    let listed = store
        .list_enabled_automation_configs(&owner())
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].session.agent_session_id, session.agent_session_id);
    assert_eq!(listed[0].config, saved);

    let disabled = store
        .commit_automation_config(crate::CommitAgentSessionAutomationConfig {
            agent_session_id: session.agent_session_id.clone(),
            owner_ref: owner(),
            expected_revision: 1,
            enabled: false,
            tag: Some("release".to_owned()),
            max_requirements: Some(3),
            operation_id: Some("autowork-config-disable".to_owned()),
            recorded_at: 1_788_000_006_010,
        })
        .await
        .unwrap();
    assert_eq!(disabled.revision, 2);
    assert!(!disabled.enabled);
    assert_eq!(
        store
            .commit_automation_config(command.clone())
            .await
            .unwrap(),
        saved,
        "A -> B -> replay(A) must return A's historical receipt"
    );
    assert_eq!(
        store
            .automation_config(&session.agent_session_id)
            .await
            .unwrap(),
        disabled,
        "historical replay must not roll the current config back"
    );
    let mut changed_replay = command.clone();
    changed_replay.tag = Some("changed".to_owned());
    assert!(matches!(
        store.commit_automation_config(changed_replay).await,
        Err(SessionStoreError::IdempotencyConflict(_))
    ));

    let mut stale = command.clone();
    stale.operation_id = Some("autowork-config-stale".to_owned());
    stale.tag = Some("other".to_owned());
    assert!(matches!(
        store.commit_automation_config(stale).await,
        Err(SessionStoreError::Conflict(_))
    ));
    let mut foreign = command;
    foreign.owner_ref.principal_id = "foreign".to_owned();
    foreign.expected_revision = 1;
    foreign.operation_id = Some("autowork-config-foreign".to_owned());
    assert!(store.commit_automation_config(foreign).await.is_err());
}

#[tokio::test]
async fn restart_quarantines_unfinished_resource_cleanup_before_retry() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, _) = create_ready(&store, "delete-cleanup-started").await;
    let command = DeleteAgentSessionCommand {
        operation_id: OperationId::from("delete-cleanup-started-operation"),
        agent_session_id: session.agent_session_id.clone(),
        owner_ref: owner(),
        requested_at: 1_788_000_007_000,
    };
    store.fence_delete(&command).await.unwrap();
    store
        .record_resource_cleanup_started(&session.agent_session_id, "ssh")
        .await
        .unwrap();
    assert_eq!(
        store
            .delete_blockers(&session.agent_session_id)
            .await
            .unwrap()
            .resource_cleanup_pending,
        vec!["ssh"]
    );
    let restarted = AgentSessionStore::from_pool(store.test_pool().clone())
        .await
        .unwrap();
    assert_eq!(
        restarted
            .quarantine_pending_resource_cleanups_for_delete(
                &owner(),
                &session.agent_session_id,
                1_788_000_007_010,
            )
            .await
            .unwrap(),
        1
    );
    let blockers = restarted
        .delete_blockers(&session.agent_session_id)
        .await
        .unwrap();
    assert!(blockers.resource_cleanup_pending.is_empty());
    assert_eq!(blockers.resource_cleanup_uncertainties.len(), 1);
}
