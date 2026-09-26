use super::*;
use crate::{NativeCheckpointWrite, MAX_NATIVE_CHECKPOINT_BYTES};

fn checkpoint_write(session: &AgentSessionLiveRecord, turn: &EventId, revision: u64, value: u64) -> (SessionEventAppend, NativeCheckpointWrite) {
    let state = StrictJsonValue(json!({"version":1,"model_steps":value,"plan":{"revision":value}}));
    let digest = digest_bytes(&canonical_json_bytes(&state.0).unwrap());
    let key = format!("checkpoint-{}-{revision}", session.agent_session_id.as_ref());
    let event = append(&session.agent_session_id, &key, "runtime_supervisor", &key,
        "runtime/progress-recorded", "checkpoint-turn", Some(turn.clone()), json!({
            "runtime_binding_id":format!("nomi:{}",session.agent_session_id.as_ref()), "producer_seq":revision + 1,
            "event":{"event":"execution_checkpoint_saved","step":value,"revision":revision + 1,"digest":digest}
        }));
    let write = NativeCheckpointWrite { owner: owner(), operation_id: "checkpoint-turn".into(),
        snapshot: snapshot_ref(), active_set_generation: 0, expected_revision: revision,
        execution_fence: 0, lease: None, state };
    (event, write)
}

#[tokio::test]
async fn checkpoint_is_atomic_idempotent_and_survives_database_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("checkpoint.db");
    let db = nomifun_db::init_database(&path).await.unwrap();
    let store = AgentSessionStore::from_pool(db.pool().clone()).await.unwrap();
    let (session, turn) = create_turn(&store, "disk-checkpoint", "checkpoint-turn").await;
    let (event, write) = checkpoint_write(&session, &turn, 0, 12);
    let saved = store.save_native_checkpoint(&event, write.clone()).await.unwrap();
    let cursor = store.current_cursor(&session.agent_session_id).await.unwrap();
    assert_eq!(store.save_native_checkpoint(&event, write).await.unwrap(), saved);
    assert_eq!(store.current_cursor(&session.agent_session_id).await.unwrap(), cursor);
    drop(store);
    db.close().await;
    let reopened = AgentSessionStore::connect_existing(&path).await.unwrap();
    assert_eq!(reopened.load_native_checkpoint(&owner(), &session.agent_session_id, &"checkpoint-turn".into()).await.unwrap(), Some(saved));
    let (next_event, next) = checkpoint_write(&session, &turn, 1, 24);
    let updated = reopened.save_native_checkpoint(&next_event, next).await.unwrap();
    assert_eq!(updated.revision, 2);
    assert!(updated.through_seq > cursor.seq);
    reopened.test_pool().close().await;
}

#[tokio::test]
async fn checkpoint_rejects_wrong_owner_snapshot_generation_stale_version_and_fenced_writer() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, turn) = create_turn(&store, "checkpoint-authority", "checkpoint-turn").await;
    let (event, original) = checkpoint_write(&session, &turn, 0, 1);
    for change in ["owner", "snapshot", "generation", "revision", "fence"] {
        let mut write = original.clone();
        match change {
            "owner" => write.owner.principal_id = "other".into(),
            "snapshot" => write.snapshot.snapshot_digest = digest('f'),
            "generation" => write.active_set_generation = 1,
            "revision" => write.expected_revision = 2,
            _ => write.execution_fence = 1,
        }
        let before = store.current_cursor(&session.agent_session_id).await.unwrap();
        assert!(store.save_native_checkpoint(&event, write).await.is_err(), "{change}");
        assert_eq!(store.current_cursor(&session.agent_session_id).await.unwrap(), before);
    }
    let saved = store.save_native_checkpoint(&event, original).await.unwrap();
    let mut wrong_owner = owner(); wrong_owner.principal_id = "other".into();
    assert!(store.load_native_checkpoint(&wrong_owner, &session.agent_session_id, &saved.operation_id).await.is_err());
}

#[tokio::test]
async fn checkpoint_event_rolls_back_if_snapshot_write_fails_and_corruption_is_detected() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, turn) = create_turn(&store, "checkpoint-rollback", "checkpoint-turn").await;
    sqlx::raw_sql("CREATE TRIGGER fail_checkpoint BEFORE UPDATE OF native_checkpoint_json ON agent_turns BEGIN SELECT RAISE(ABORT, 'injected'); END;")
        .execute(store.test_pool()).await.unwrap();
    let (event, write) = checkpoint_write(&session, &turn, 0, 3);
    let before = store.current_cursor(&session.agent_session_id).await.unwrap();
    assert!(store.save_native_checkpoint(&event, write.clone()).await.is_err());
    assert_eq!(store.current_cursor(&session.agent_session_id).await.unwrap(), before);
    assert!(store.load_native_checkpoint(&owner(), &session.agent_session_id, &write.operation_id).await.unwrap().is_none());
    sqlx::query("DROP TRIGGER fail_checkpoint").execute(store.test_pool()).await.unwrap();
    store.save_native_checkpoint(&event, write.clone()).await.unwrap();
    sqlx::query("UPDATE agent_turns SET native_checkpoint_json = '{}' WHERE session_id = ?")
        .bind(session.agent_session_id.as_ref()).execute(store.test_pool()).await.unwrap();
    assert!(store.load_native_checkpoint(&owner(), &session.agent_session_id, &write.operation_id).await.is_err());
}

#[tokio::test]
async fn checkpoint_is_cleared_at_terminal_and_cannot_resurrect_cancelled_work() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, turn) = create_turn(&store, "checkpoint-cancel", "checkpoint-turn").await;
    let (event, write) = checkpoint_write(&session, &turn, 0, 2);
    store.save_native_checkpoint(&event, write.clone()).await.unwrap();
    let cancelled = append(&session.agent_session_id, "checkpoint-cancelled", "session-api", "checkpoint-cancelled",
        "turn/cancelled", "checkpoint-turn", Some(turn), json!({"operation_id":"checkpoint-turn"}));
    store.append_event(&cancelled).await.unwrap();
    assert!(store.load_native_checkpoint(&owner(), &session.agent_session_id, &write.operation_id).await.unwrap().is_none());
    assert!(store.save_native_checkpoint(&event, write).await.is_err());
}

#[tokio::test]
async fn oversized_checkpoint_is_rejected_before_any_event_write() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, turn) = create_turn(&store, "checkpoint-budget", "checkpoint-turn").await;
    let (event, mut write) = checkpoint_write(&session, &turn, 0, 1);
    write.state = StrictJsonValue(json!({"oversized":"x".repeat(MAX_NATIVE_CHECKPOINT_BYTES)}));
    let before = store.current_cursor(&session.agent_session_id).await.unwrap();
    assert!(store.save_native_checkpoint(&event, write).await.is_err());
    assert_eq!(store.current_cursor(&session.agent_session_id).await.unwrap(), before);
}

#[tokio::test]
async fn failed_turn_keeps_progress_as_data_without_reopening_its_terminal() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, turn) = create_turn(&store, "checkpoint-failed", "checkpoint-turn").await;
    let (event, write) = checkpoint_write(&session, &turn, 0, 7);
    let saved = store.save_native_checkpoint(&event, write.clone()).await.unwrap();
    let failed = append(&session.agent_session_id, "checkpoint-failed-terminal", "runtime-supervisor", "checkpoint-failed-terminal",
        "turn/failed", "checkpoint-turn", Some(turn), json!({"operation_id":"checkpoint-turn","reason":"restart"}));
    store.append_event(&failed).await.unwrap();
    let loaded = store.load_native_checkpoint(&owner(), &session.agent_session_id, &write.operation_id).await.unwrap().unwrap();
    assert_eq!(loaded.state, saved.state);
    assert_eq!(loaded.turn_state, "failed");
    assert!(store.save_native_checkpoint(&event, write).await.is_err());
    assert_eq!(store.read_turn_receipt(&session.agent_session_id, &saved.operation_id).await.unwrap().status, TurnReceiptStatus::Failed);
}

#[tokio::test]
async fn unsettled_effect_defers_checkpoint_without_consuming_a_sequence() {
    let store = AgentSessionStore::open_in_memory().await.unwrap();
    let (session, effect, _) = create_pending_effect(&store, "checkpoint-pending", EffectStrategy::ManagedEffect).await;
    let turn = store.read_turn_receipt(&session.agent_session_id, &effect.turn_id).await.unwrap().started_event.unwrap().event_id;
    let (mut event, mut write) = checkpoint_write(&session, &turn, 0, 1);
    event.semantic_event.correlation_id = CorrelationId::from(effect.turn_id.as_ref());
    write.operation_id = effect.turn_id;
    let before = store.current_cursor(&session.agent_session_id).await.unwrap();
    assert!(matches!(store.save_native_checkpoint(&event, write).await, Err(SessionStoreError::CheckpointNotQuiescent)));
    assert_eq!(store.current_cursor(&session.agent_session_id).await.unwrap(), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_checkpoint_writers_have_one_cas_winner() {
    let store = AgentSessionStore::open_in_memory_with_connections(2).await.unwrap();
    let (session, turn) = create_turn(&store, "checkpoint-race", "checkpoint-turn").await;
    let (first_event, first) = checkpoint_write(&session, &turn, 0, 1);
    let (mut second_event, second) = checkpoint_write(&session, &turn, 0, 2);
    second_event.event_id = "competing-checkpoint".into();
    second_event.idempotency_key = "competing-checkpoint".into();
    let (left, right) = tokio::join!(store.save_native_checkpoint(&first_event, first), store.save_native_checkpoint(&second_event, second));
    assert_ne!(left.is_ok(), right.is_ok());
    let saved = store.load_native_checkpoint(&owner(), &session.agent_session_id, &"checkpoint-turn".into()).await.unwrap().unwrap();
    assert_eq!(saved.revision, 1);
}
