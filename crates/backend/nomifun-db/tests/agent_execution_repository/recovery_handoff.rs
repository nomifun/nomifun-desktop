use super::*;

async fn ongoing_fixture(
    head_status: &str, native_lease_until: i64,
) -> (nomifun_db::Database, SqliteAgentExecutionRepository, String, nomifun_db::AgentExecutionStepDetailRow, RecoveredAgentExecutionAttemptOutput) {
    let db = database().await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
    let (execution, running, output) = recovery_fixture(&db, &repository).await;
    // Keep a real exact start fact and no terminal. No model, process, Runtime
    // or recovery command is invoked by this database handoff fixture.
    nomifun_db::sqlx::query("UPDATE agent_events SET kind='turn/started',inline_json='{}' WHERE event_id=?")
        .bind(&output.terminal_event_id).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("UPDATE agent_turns SET state='running',started_event_id=terminal_event_id,terminal_event_id=NULL,finished_at=NULL,execution_owner='native-original-owner',execution_generation=17,execution_lease_until=? WHERE operation_id=?")
        .bind(native_lease_until).bind(&output.canonical_operation_id).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("INSERT INTO agent_session_heads(session_id,status,active_turn_id,active_set_generation,last_seq,unread_count) VALUES(?,?,?,0,1,0)")
        .bind(&output.conversation_id).bind(head_status).bind(&output.canonical_operation_id).execute(db.pool()).await.unwrap();
    (db, repository, execution, running, output)
}

#[tokio::test]
async fn recovery_handoff_preserves_original_turn_across_all_four_lease_combinations() {
    for execution_live in [true, false] {
        for native_live in [true, false] {
            let now = nomifun_common::now_ms();
            let (db, repository, execution_id, running, original) = ongoing_fixture("running",
                if native_live { now + 90_000 } else { 0 }).await;
            nomifun_db::sqlx::query("UPDATE agent_executions SET lease_owner='original-execution-owner',lease_expires_at=? WHERE execution_id=?")
                .bind(if execution_live { now + 30_000 } else { 0 }).bind(&execution_id).execute(db.pool()).await.unwrap();
            let before = repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap();
            let lease = AgentExecutionLeaseToken::new("replacement-execution-owner".into());
            let claimed = repository.try_acquire_lease(&execution_id, before.version, lease.owner(),
                nomifun_common::now_ms() + 30_000).await.unwrap();
            assert_eq!(claimed.is_some(), !execution_live, "Execution/native valid: {execution_live}/{native_live}");
            if !execution_live {
                let before_observe = repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap();
                let attempt = &running.current_attempt.as_ref().unwrap().attempt;
                let observed = repository.reconcile_recovered_attempt(OWNER_ID, &execution_id,
                    &running.step.step_id, running.step.version, &attempt.attempt_id, attempt.version,
                    &lease, None, &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
                assert_eq!(observed.disposition, AgentExecutionAttemptRecoveryDisposition::OngoingTurnObserved);
                assert_eq!(observed.detail.step.version, running.step.version);
                assert_eq!(observed.detail.current_attempt.unwrap().attempt.version, attempt.version);
                let after = repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap();
                assert_eq!(after.version, before_observe.version, "a read handoff cannot become a second execution fact");
                assert_eq!(after.event_sequence, before_observe.event_sequence);
            }
            let links = repository.resolve_conversation_link(OWNER_ID, &original.conversation_id).await.unwrap();
            assert_eq!(links.len(), 1);
            assert!(links[0].active && links[0].cleanup_completed_at.is_none());
            let turn: (String, String, i64) = nomifun_db::sqlx::query_as(
                "SELECT state,execution_owner,execution_generation FROM agent_turns WHERE operation_id=?",
            ).bind(&original.canonical_operation_id).fetch_one(db.pool()).await.unwrap();
            assert_eq!(turn, ("running".into(), "native-original-owner".into(), 17));
            assert_eq!(repository.get_step_detail(OWNER_ID, &execution_id, &running.step.step_id).await.unwrap().unwrap()
                .current_attempt.unwrap().attempt.attempt_id, original.attempt_id);
            assert_eq!(repository.get_execution_detail(OWNER_ID, &execution_id).await.unwrap().unwrap().attempts.len(), 1);
            assert!(repository.list_pending_conversation_cleanups(Some(&execution_id), None, 100).await.unwrap().cleanups.is_empty());
        }
    }
}

#[tokio::test]
async fn recovery_handoff_keeps_native_pause_and_reconciliation_attached_without_cleanup() {
    for status in ["paused", "reconciliation"] {
        let (db, repository, execution_id, running, original) = ongoing_fixture(status, 0).await;
        let execution = repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap();
        let lease = AgentExecutionLeaseToken::new("handoff-observer".into());
        repository.try_acquire_lease(&execution_id, execution.version, lease.owner(), nomifun_common::now_ms() + 30_000)
            .await.unwrap().unwrap();
        let attempt = running.current_attempt.as_ref().unwrap();
        let observed = repository.reconcile_recovered_attempt(OWNER_ID, &execution_id, &running.step.step_id,
            running.step.version, &attempt.attempt.attempt_id, attempt.attempt.version, &lease,
            None, &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
        assert_eq!(observed.disposition, AgentExecutionAttemptRecoveryDisposition::OngoingTurnObserved);
        assert_eq!(observed.detail.current_attempt.unwrap().attempt.runtime_state, None);
        let head: (String, String) = nomifun_db::sqlx::query_as("SELECT status,active_turn_id FROM agent_session_heads WHERE session_id=?")
            .bind(&original.conversation_id).fetch_one(db.pool()).await.unwrap();
        assert_eq!(head, (status.into(), original.canonical_operation_id));
        assert!(repository.resolve_conversation_link(OWNER_ID, &original.conversation_id).await.unwrap()[0].active);
    }
}

#[tokio::test]
async fn recovery_handoff_cannot_attach_after_a_user_stop_fence() {
    let (_db, repository, execution_id, running, _) = ongoing_fixture("running", 0).await;
    let execution = repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap();
    let lease = AgentExecutionLeaseToken::new("stopped-handoff".into());
    repository.try_acquire_lease(&execution_id, execution.version, lease.owner(), nomifun_common::now_ms() + 30_000)
        .await.unwrap().unwrap();
    let execution = repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap();
    let stopped = repository.cancel_step(OWNER_ID, &execution_id, execution.version, &running.step.step_id,
        running.step.version, "task".parse().unwrap(), &NewAgentExecutionEvent {
            actor: nomifun_common::AgentExecutionActor::user(OWNER_ID), ..event(AgentExecutionEventKind::StepChanged)
        }).await.unwrap();
    let attempt = running.current_attempt.as_ref().unwrap();
    assert!(repository.reconcile_recovered_attempt(OWNER_ID, &execution_id, &running.step.step_id,
        running.step.version, &attempt.attempt.attempt_id, attempt.attempt.version, &lease,
        None, &event(AgentExecutionEventKind::AttemptChanged)).await.is_err());
    assert_eq!(stopped.attempts[0].attempt.status, "cancelled");
    assert_eq!(repository.get_execution_detail(OWNER_ID, &execution_id).await.unwrap().unwrap().attempts.len(), 1);
}
