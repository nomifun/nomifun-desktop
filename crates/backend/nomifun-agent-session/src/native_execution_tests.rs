use super::*;
use serde_json::Value;
use crate::{NativeCheckpoint, NativeCheckpointWrite, NativeExecutionClaim, NativeExecutionLease};

#[path = "native_pause_tests.rs"]
mod native_pause_tests;

struct Fixture { session: AgentSessionLiveRecord, root: EventId, started: EventId }

fn route() -> ChatRouteIdentity { ChatRouteIdentity::new("coding.codex@1", nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT, "chat-route".into(), 4) }

async fn fixture(store: &AgentSessionStore, key: &str) -> Fixture {
    let (session, _) = create_ready(store, key).await;
    let input = append(&session.agent_session_id, &format!("input-{key}"), "session-api", &format!("input-{key}"),
        "message/user-accepted", "lease-turn", None, json!({"content":"keep working"}));
    let root = store.append_event(&input).await.unwrap().ack.unwrap().event_id;
    let turn = append(&session.agent_session_id, &format!("turn-{key}"), "session-api", &format!("turn-{key}"),
        "turn/started", "lease-turn", Some(root.clone()), json!({"operation_id":"lease-turn", "input_event_id":root,
            "route_identity":route(),"resolved_snapshot_ref":snapshot_ref()}));
    let started = store.append_event(&turn).await.unwrap().ack.unwrap().event_id;
    Fixture { session, root, started }
}

fn claim(f: &Fixture, holder: &str, fence: u64, cp: Option<&NativeCheckpoint>) -> NativeExecutionClaim {
    NativeExecutionClaim { owner: owner(), agent_session_id: f.session.agent_session_id.clone(), operation_id: "lease-turn".into(),
        snapshot: snapshot_ref(), active_set_generation: 0, holder: holder.into(), expected_fence: fence,
        checkpoint: cp.map(|cp| (cp.revision, cp.digest.clone(), cp.through_seq)) }
}

fn progress(f: &Fixture, key: &str, event: Value) -> SessionEventAppend {
    append(&f.session.agent_session_id, key, "runtime_supervisor", key, "runtime/progress-recorded", "lease-turn", Some(f.started.clone()),
        json!({"runtime_binding_id":"native-fixture","producer_seq":1,"event":event}))
}

fn model(f: &Fixture, operation: &str) -> ChatOperationClaimRequest {
    ChatOperationClaimRequest { agent_session_id: f.session.agent_session_id.clone(), operation_id: operation.into(),
        turn_operation_id: "lease-turn".into(), causation_event_id: f.root.clone(), route_identity: route(), resolved_snapshot_ref: snapshot_ref() }
}

async fn checkpoint(store: &AgentSessionStore, f: &Fixture, lease: &NativeExecutionLease) -> NativeCheckpoint {
    let state = StrictJsonValue(json!({"version":1,"model_steps":0}));
    let digest = digest_bytes(&canonical_json_bytes(&state.0).unwrap());
    store.save_native_checkpoint(&progress(f, "checkpoint", json!({"event":"execution_checkpoint_saved","step":0,"revision":1,"digest":digest})),
        NativeCheckpointWrite { owner: owner(), operation_id: "lease-turn".into(), snapshot: snapshot_ref(), active_set_generation: 0,
            expected_revision: 0, execution_fence: lease.fence(), lease: Some(lease.clone()), state }).await.unwrap()
}

async fn expire(store: &AgentSessionStore, f: &Fixture) {
    // Deterministic fault injection in this test's isolated database; production
    // callers never select the clock or the lease expiry timestamp.
    sqlx::query("UPDATE agent_turns SET execution_lease_until = 0 WHERE session_id = ?")
        .bind(f.session.agent_session_id.as_ref()).execute(store.test_pool()).await.unwrap();
}

#[tokio::test]
async fn native_takeover_fences_old_model_tool_observation_and_checkpoint_writers() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = fixture(&store, "takeover").await;
    let old = store.claim_native_execution(claim(&f, "old", 0, None)).await.unwrap();
    assert_eq!(store.claim_native_execution(claim(&f, "old", 0, None)).await.unwrap(), old);
    let cp = checkpoint(&store, &f, &old).await;
    store.claim_native_chat_operation(&old, model(&f, "claimed-before-restart")).await.unwrap();
    assert!(matches!(store.claim_native_execution(claim(&f, "new", 0, Some(&cp))).await, Err(SessionStoreError::ExecutionLeaseActive)));
    expire(&store, &f).await;
    let new = store.claim_native_execution(claim(&f, "new", 0, Some(&cp))).await.unwrap();
    assert_eq!(new.fence(), 1);
    let before = store.current_cursor(&f.session.agent_session_id).await.unwrap();
    assert!(matches!(store.renew_native_execution(&old).await, Err(SessionStoreError::ExecutionFenced)));
    assert!(matches!(store.claim_native_chat_operation(&old, model(&f, "old-model")).await, Err(SessionStoreError::ExecutionFenced)));
    assert!(matches!(store.claim_chat_operation(model(&f, "unleased-model")).await, Err(SessionStoreError::ExecutionFenced)));
    assert!(matches!(store.claim_native_chat_operation(&new, model(&f, "claimed-before-restart")).await, Err(SessionStoreError::ExecutionFenced)));
    let state = StrictJsonValue(json!({"model_steps":1}));
    let digest = digest_bytes(&canonical_json_bytes(&state.0).unwrap());
    assert!(matches!(store.save_native_checkpoint(&progress(&f, "old-checkpoint", json!({"event":"execution_checkpoint_saved","step":1,"revision":2,"digest":digest})),
        NativeCheckpointWrite { owner: owner(), operation_id: "lease-turn".into(), snapshot: snapshot_ref(), active_set_generation: 0,
            expected_revision: 1, execution_fence: old.fence(), lease: Some(old.clone()), state }).await, Err(SessionStoreError::ExecutionFenced)));
    let tool = progress(&f, "old-tool", json!({"event":"tool_started","step":1,"call_id":"write","action_id":"workspace.files/write"}));
    assert!(matches!(store.append_native_event(&old, &tool, None).await, Err(SessionStoreError::ExecutionFenced)));
    assert!(matches!(store.append_event(&tool).await, Err(SessionStoreError::ExecutionFenced)));
    let observed = progress(&f, "old-observation", json!({"event":"host_cleanup_proven"}));
    assert!(matches!(store.append_native_observation(&old, &observed, None).await, Err(SessionStoreError::ExecutionFenced)));
    assert_eq!(store.current_cursor(&f.session.agent_session_id).await.unwrap(), before);
    assert!(store.claim_native_chat_operation(&new, model(&f, "new-model")).await.is_ok());
    assert!(store.renew_native_execution(&new).await.is_ok());
}

#[tokio::test]
async fn native_cancellation_preserves_cleanup_but_blocks_new_admission() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = fixture(&store, "lease-cancel").await;
    let lease = store.claim_native_execution(claim(&f, "owner", 0, None)).await.unwrap();
    store.cancel_active_turn(&f.session.agent_session_id, "cancel".into(), "session-api".into()).await.unwrap();
    let tool = progress(&f, "tool-after-cancel", json!({"event":"tool_started"}));
    assert!(store.append_native_event(&lease, &tool, None).await.is_err());
    assert!(store.append_native_observation(&lease, &tool, None).await.is_err());
    let cleanup = progress(&f, "cleanup-after-cancel", json!({"event":"host_cleanup_proven"}));
    assert!(store.append_native_observation(&lease, &cleanup, None).await.is_ok());
    assert!(!store.heartbeat_native_execution(&lease).await.unwrap());
}

#[tokio::test]
async fn native_recovery_never_replays_across_a_possible_effect_after_checkpoint() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = fixture(&store, "unsafe-tail").await;
    let lease = store.claim_native_execution(claim(&f, "old", 0, None)).await.unwrap();
    let cp = checkpoint(&store, &f, &lease).await;
    store.append_native_event(&lease, &progress(&f, "effect", json!({"event":"tool_started","step":1,"call_id":"effect"})), None).await.unwrap();
    expire(&store, &f).await;
    assert!(matches!(store.claim_native_execution(claim(&f, "new", 0, Some(&cp))).await, Err(SessionStoreError::RecoveryRequiresReconciliation)));
    assert!(store.verify_native_execution(&lease).await.is_ok());
}

#[tokio::test(flavor="multi_thread", worker_threads=2)]
async fn native_recovery_race_has_exactly_one_winner() {
    let store = AgentSessionStore::open_in_memory_with_connections(2).await.unwrap();
    let f = fixture(&store, "lease-race").await;
    let old = store.claim_native_execution(claim(&f, "old", 0, None)).await.unwrap();
    let cp = checkpoint(&store, &f, &old).await;
    expire(&store, &f).await;
    let (a,b) = tokio::join!(store.claim_native_execution(claim(&f,"a",0,Some(&cp))), store.claim_native_execution(claim(&f,"b",0,Some(&cp))));
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(a.ok().or_else(||b.ok()).unwrap().fence(), 1);
}

#[tokio::test]
async fn unleased_supervisor_cannot_terminate_a_live_native_producer() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = fixture(&store, "supervisor-fence").await;
    store.claim_native_execution(claim(&f, "live", 0, None)).await.unwrap();
    let terminal = append(&f.session.agent_session_id, "stale-failure", "runtime_supervisor", "stale-failure", "turn/failed", "lease-turn", Some(f.started.clone()), json!({"operation_id":"lease-turn"}));
    assert!(matches!(store.append_turn_terminal(&terminal, &"lease-turn".into()).await, Err(SessionStoreError::ExecutionLeaseActive)));
    expire(&store, &f).await;
    assert!(store.append_turn_terminal(&terminal, &"lease-turn".into()).await.is_ok());
}

#[tokio::test]
async fn recovery_quarantine_is_atomic_fenced_and_retains_checkpoint_without_success() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = fixture(&store, "quarantine").await;
    let lease = store.claim_native_execution(claim(&f, "old", 0, None)).await.unwrap();
    let cp = checkpoint(&store, &f, &lease).await;
    assert!(matches!(store.quarantine_native_recovery(&owner(), &f.session.agent_session_id, &"lease-turn".into(), 0).await,
        Err(SessionStoreError::ExecutionLeaseActive)));
    expire(&store, &f).await;
    assert!(store.quarantine_native_recovery(&owner(), &f.session.agent_session_id, &"lease-turn".into(), 0).await.unwrap());
    assert!(matches!(store.verify_native_execution(&lease).await, Err(SessionStoreError::ExecutionFenced)));
    let inspection = store.inspect_latest_native_execution(&owner(), &f.session.agent_session_id).await.unwrap().unwrap();
    assert_eq!(inspection.state, "paused");
    assert_eq!(inspection.turn_state, "running");
    assert!(!inspection.pause.as_ref().unwrap().cleanup_proven);
    let head = store.head(&f.session.agent_session_id).await.unwrap();
    assert_eq!(head.status, "paused");
    assert_eq!(head.active_turn_id.as_deref(), Some("lease-turn"));
    assert_eq!(store.read_turn_receipt(&f.session.agent_session_id, &"lease-turn".into()).await.unwrap().status, TurnReceiptStatus::Running);
    assert!(inspection.recovery_blocked && inspection.checkpoint_retained);
    assert!(!inspection.producer_lease_live && !inspection.automatic_replay_authorized);
    assert_eq!(inspection.execution_fence, 1);
    assert_eq!(inspection.checkpoint_revision, cp.revision);
    let saved = store.load_native_checkpoint(&owner(), &f.session.agent_session_id, &"lease-turn".into()).await.unwrap().unwrap();
    assert_eq!(saved.digest, cp.digest);
    assert!(!store.quarantine_native_recovery(&owner(), &f.session.agent_session_id, &"lease-turn".into(), 0).await.unwrap());
    let wrong = PrincipalRef { principal_kind: "user".into(), principal_id: "different-owner".into() };
    assert!(store.inspect_latest_native_execution(&wrong, &f.session.agent_session_id).await.is_err());
}

#[tokio::test]
async fn unused_recovery_claim_can_release_but_an_admitted_writer_cannot() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = fixture(&store, "claim-release").await;
    let old = store.claim_native_execution(claim(&f, "old", 0, None)).await.unwrap();
    let cp = checkpoint(&store, &f, &old).await;
    expire(&store, &f).await;
    let recovered = store.claim_native_execution(claim(&f, "new", 0, Some(&cp))).await.unwrap();
    store.release_unattached_native_claim(&recovered).await.unwrap();
    assert_eq!(store.native_execution_deadline(&f.session.agent_session_id, &"lease-turn".into()).await.unwrap(), 0);
    store.append_native_event(&recovered, &progress(&f, "new-progress", json!({"event":"model_step_started","step":1,"operation_id":"next"})), None).await.unwrap();
    assert!(matches!(store.release_unattached_native_claim(&recovered).await, Err(SessionStoreError::RecoveryRequiresReconciliation)));
}

#[tokio::test]
async fn orphan_quarantine_marks_pending_effect_unknown_and_never_replays_it() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, request, _) = create_pending_effect(&store, "native-quarantine-effect", EffectStrategy::ManagedEffect).await;
    assert!(store.quarantine_native_recovery(&owner(), &session.agent_session_id, &request.turn_id, 0).await.unwrap());
    let effect = store.read_effect(&session.agent_session_id, &request.effect_id).await.unwrap().unwrap();
    assert_eq!(effect.state, crate::AgentEffectState::Unknown);
    let inspection = store.inspect_latest_native_execution(&owner(), &session.agent_session_id).await.unwrap().unwrap();
    assert_eq!(inspection.state, "paused");
    assert_eq!(inspection.turn_state, "running");
    assert!(!inspection.pause.as_ref().unwrap().cleanup_proven);
    assert_eq!(store.head(&session.agent_session_id).await.unwrap().active_turn_id.as_deref(), Some(request.turn_id.as_ref()));
    assert_eq!(store.read_turn_receipt(&session.agent_session_id, &request.turn_id).await.unwrap().status, TurnReceiptStatus::Running);
    assert_eq!(inspection.pending_effects, 0);
    assert_eq!(inspection.unknown_effects, 1);
    assert!(inspection.recovery_blocked && !inspection.automatic_replay_authorized);
    assert!(store.has_unsettled_effects(&session.agent_session_id).await.unwrap());
}

#[tokio::test]
async fn empty_recovery_accepts_only_a_root_preamble_without_model_or_tool_work() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = fixture(&store, "empty-preamble").await;
    let old = store.claim_native_execution(claim(&f, "old", 0, None)).await.unwrap();
    store.append_native_event(&old, &progress(&f, "root-preamble", json!({"event":"turn_started"})), None).await.unwrap();
    expire(&store, &f).await;
    let recovered = store.claim_native_empty_recovery(claim(&f, "new", 0, None)).await.unwrap();
    assert_eq!(recovered.fence(), 1);
    store.claim_native_chat_operation(&recovered, model(&f, "admitted-model")).await.unwrap();
    expire(&store, &f).await;
    assert!(matches!(store.claim_native_empty_recovery(claim(&f, "third", 1, None)).await,
        Err(SessionStoreError::RecoveryRequiresReconciliation)));
}

#[tokio::test]
async fn resource_admission_requires_a_real_claim_not_an_operation_name_in_history() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let f = fixture(&store, "exact-native-claim").await;
    let lease = store.claim_native_execution(claim(&f, "current", 0, None)).await.unwrap();
    let request = model(&f, "model-claimed");
    store.append_native_event(&lease, &progress(&f, "untrusted-operation-name", json!({"event":"tool_call_completed","step":1,
        "call":{"call_id":"data-only","name":"read_file","arguments":{"path":"a","operation_id":"model-claimed"},"provider_metadata":null}})), None).await.unwrap();
    assert!(store.verify_native_chat_operation(&lease, &request).await.is_err());
    store.claim_native_chat_operation(&lease, request.clone()).await.unwrap();
    store.verify_native_chat_operation(&lease, &request).await.unwrap();
    let facts = store.native_recovery_facts(&f.session.agent_session_id, &"lease-turn".into()).await.unwrap();
    assert!(facts.events.iter().any(|event| event.event_id == f.root));
    assert!(facts.events.iter().any(|event| event.event_id == f.started));
}
