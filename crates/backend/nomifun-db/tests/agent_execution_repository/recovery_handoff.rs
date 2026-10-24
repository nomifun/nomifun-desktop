use super::*;

async fn ongoing_fixture(
    head_status: &str, native_live: bool,
) -> (nomifun_db::Database, SqliteAgentExecutionRepository, String, nomifun_db::AgentExecutionStepDetailRow, RecoveredAgentExecutionAttemptOutput) {
    let db = database().await;
    let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
    let (execution, running, output) = recovery_fixture(&db, &repository).await;
    // Keep a real exact start fact and no terminal. No model, process, Runtime
    // or recovery command is invoked by this database handoff fixture.
    nomifun_db::sqlx::query("UPDATE agent_events SET kind='turn/started',inline_json='{}' WHERE event_id=?")
        .bind(&output.terminal_event_id).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("UPDATE agent_turns SET state='running',started_event_id=terminal_event_id,terminal_event_id=NULL,finished_at=NULL,execution_owner='native-original-owner',execution_generation=17,execution_lease_until=? WHERE operation_id=?")
        .bind(if native_live { nomifun_common::now_ms() + 90_000 } else { 0 }).bind(&output.canonical_operation_id).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("INSERT INTO agent_session_heads(session_id,status,active_turn_id,active_set_generation,last_seq,unread_count) VALUES(?,?,?,0,1,0)")
        .bind(&output.conversation_id).bind(head_status).bind(&output.canonical_operation_id).execute(db.pool()).await.unwrap();
    (db, repository, execution, running, output)
}

#[tokio::test]
async fn recovery_handoff_preserves_original_turn_across_all_four_lease_combinations() {
    for execution_live in [true, false] {
        for native_live in [true, false] {
            let (db, repository, execution_id, running, original) = ongoing_fixture("running", native_live).await;
            nomifun_db::sqlx::query("UPDATE agent_executions SET lease_owner='original-execution-owner',lease_expires_at=? WHERE execution_id=?")
                .bind(if execution_live { nomifun_common::now_ms() + 30_000 } else { 0 }).bind(&execution_id).execute(db.pool()).await.unwrap();
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
async fn recovery_handoff_rejects_projection_only_pause_or_reconciliation_without_canonical_source() {
    for status in ["paused", "reconciliation"] {
        let (db, repository, execution_id, running, original) = ongoing_fixture(status, false).await;
        let execution = repository.get_execution(OWNER_ID, &execution_id).await.unwrap().unwrap();
        let lease = AgentExecutionLeaseToken::new("handoff-observer".into());
        repository.try_acquire_lease(&execution_id, execution.version, lease.owner(), nomifun_common::now_ms() + 30_000)
            .await.unwrap().unwrap();
        let attempt = running.current_attempt.as_ref().unwrap();
        let observed = repository.reconcile_recovered_attempt(OWNER_ID, &execution_id, &running.step.step_id,
            running.step.version, &attempt.attempt.attempt_id, attempt.attempt.version, &lease,
            None, &event(AgentExecutionEventKind::AttemptChanged)).await;
        assert!(observed.is_err(),"a projected status without its exact source cannot authorize a handoff");
        assert_eq!(repository.get_step_detail(OWNER_ID,&execution_id,&running.step.step_id).await.unwrap().unwrap()
            .current_attempt.unwrap().attempt.runtime_state,None);
        let head: (String, String) = nomifun_db::sqlx::query_as("SELECT status,active_turn_id FROM agent_session_heads WHERE session_id=?")
            .bind(&original.conversation_id).fetch_one(db.pool()).await.unwrap();
        assert_eq!(head, (status.into(), original.canonical_operation_id));
        assert!(repository.resolve_conversation_link(OWNER_ID, &original.conversation_id).await.unwrap()[0].active);
    }
}

async fn original_admission_fixture() -> (nomifun_db::Database,SqliteAgentExecutionRepository,String,
    nomifun_db::AgentExecutionStepDetailRow,RecoveredAgentExecutionAttemptOutput,AgentExecutionLeaseToken,serde_json::Value) {
    use serde_json::json;
    let (db,repository,execution,running,original)=ongoing_fixture("running",false).await;
    let request=json!({"content":"exact original work","files":[],"inject_skills":[],"hidden":false,"origin":"agent_execution","channel_platform":null});
    let opening=nomifun_common::generate_id();let accepted=nomifun_common::generate_id();let identity=format!("user:{OWNER_ID}:{}:{}:initial-turn",original.conversation_id,original.attempt_id);
    nomifun_db::sqlx::query("UPDATE agent_events SET seq=3 WHERE session_id=? AND event_id=?")
        .bind(&original.conversation_id).bind(&original.terminal_event_id).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,1,?,'session_api',?,'session/opening',1,?,'{}')")
        .bind(&original.conversation_id).bind(&opening).bind(format!("fixture-opening:{}",original.conversation_id)).bind(&original.conversation_id).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json) VALUES(?,2,?,'session_api',?,'message/user-accepted',1,?,?,?)")
        .bind(&original.conversation_id).bind(&accepted).bind(format!("{identity}:message")).bind(&accepted).bind(&opening).bind(request.to_string()).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("UPDATE agent_events SET producer_id='session_api',idempotency_key=?,causation_event_id=?,inline_json=? WHERE session_id=? AND event_id=?")
        .bind(format!("{identity}:turn")).bind(&accepted).bind(json!({"operation_id":original.canonical_operation_id,"source_message_id":accepted}).to_string())
        .bind(&original.conversation_id).bind(&original.terminal_event_id).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("UPDATE agent_turns SET source_message_id=? WHERE session_id=? AND operation_id=?")
        .bind(accepted).bind(&original.conversation_id).bind(&original.canonical_operation_id).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("UPDATE agent_session_heads SET last_seq=3 WHERE session_id=?")
        .bind(&original.conversation_id).execute(db.pool()).await.unwrap();
    let effects=nomifun_db::AttemptConversationEffects {pending_conversation_effects:vec![
        nomifun_db::PendingConversationEffect::InitialInput {request:request.clone()},
        nomifun_db::PendingConversationEffect::Steer {operation_id:"exact-steer".into(),target_operation_id:original.canonical_operation_id.clone(),content:"original correction".into()},
        nomifun_db::PendingConversationEffect::StopTurn {operation_id:"exact-stop".into(),target_operation_id:original.canonical_operation_id.clone()},
    ],review_blocked:None};
    nomifun_db::sqlx::query("UPDATE agent_execution_attempts SET runtime_state=? WHERE execution_id=? AND attempt_id=?")
        .bind(effects.encode().unwrap()).bind(&execution).bind(&original.attempt_id).execute(db.pool()).await.unwrap();
    let current=repository.get_execution(OWNER_ID,&execution).await.unwrap().unwrap();
    let lease=AgentExecutionLeaseToken::new("exact-original-admission-observer".into());
    repository.try_acquire_lease(&execution,current.version,lease.owner(),nomifun_common::now_ms()+30_000).await.unwrap().unwrap();
    (db,repository,execution,running,original,lease,request)
}

async fn add_native_pause(db:&nomifun_db::Database,original:&RecoveredAgentExecutionAttemptOutput,reason:&str) {
    let pause=nomifun_agent_contracts::NativePauseState {revision:1,reason:reason.into(),checkpoint_revision:0,
        checkpoint_digest:None,execution_fence:1,cleanup_proven:false,paused_at_ms:11};
    let identity=format!("native-paused:{}:{}:1",original.conversation_id,original.canonical_operation_id);
    nomifun_db::sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json) VALUES(?,4,?,'runtime_supervisor',?,'turn/paused',1,?,?,?)")
        .bind(&original.conversation_id).bind(&identity).bind(&identity).bind(&original.canonical_operation_id)
        .bind(&original.terminal_event_id).bind(serde_json::json!({"pause":pause}).to_string()).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("UPDATE agent_turns SET native_pause_json=?,native_pause_revision=1,execution_fence=1 WHERE session_id=? AND operation_id=?")
        .bind(serde_json::to_string(&pause).unwrap()).bind(&original.conversation_id).bind(&original.canonical_operation_id).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("UPDATE agent_session_heads SET status='paused',last_seq=4 WHERE session_id=?")
        .bind(&original.conversation_id).execute(db.pool()).await.unwrap();
}

#[tokio::test]
async fn native_technical_pause_returns_original_task_to_review_without_losing_controls_or_link() {
    let (db,repository,execution,running,original,lease,_)=original_admission_fixture().await;
    add_native_pause(&db,&original,"EXECUTION_INTERVENTION_REQUIRED").await;
    let attempt=&running.current_attempt.as_ref().unwrap().attempt;
    let observed=repository.reconcile_recovered_attempt(OWNER_ID,&execution,&running.step.step_id,running.step.version,
        &attempt.attempt_id,attempt.version,&lease,None,&event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    assert_eq!(observed.disposition,AgentExecutionAttemptRecoveryDisposition::ReviewBlocked);
    assert_eq!(observed.detail.step.status,"waiting_input");
    let attempt=observed.detail.current_attempt.unwrap().attempt;
    let retained=nomifun_db::AttemptConversationEffects::decode(attempt.runtime_state.as_deref()).unwrap();
    let review=retained.review_blocked.as_ref().unwrap();
    assert_eq!(review.kind,"native_pause");assert_eq!(review.operation_id,original.canonical_operation_id);
    assert_eq!(review.reason,"EXECUTION_INTERVENTION_REQUIRED");assert_eq!(retained.pending_conversation_effects.len(),3);
    assert!(repository.resolve_conversation_link(OWNER_ID,&original.conversation_id).await.unwrap()[0].active);
    assert!(repository.list_pending_conversation_cleanups(Some(&execution),None,100).await.unwrap().cleanups.is_empty());
    let before=repository.get_execution(OWNER_ID,&execution).await.unwrap().unwrap();
    let replay=repository.reconcile_recovered_attempt(OWNER_ID,&execution,&running.step.step_id,observed.detail.step.version,
        &attempt.attempt_id,attempt.version,&lease,None,&event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    assert_eq!(replay.disposition,AgentExecutionAttemptRecoveryDisposition::ReviewBlocked);
    assert_eq!(replay.detail.step.version,observed.detail.step.version);
    assert_eq!(repository.get_execution(OWNER_ID,&execution).await.unwrap().unwrap().event_sequence,before.event_sequence,
        "recheck cannot clear a still-current technical pause or add repeated review facts");
    let turn:(String,Option<String>)=nomifun_db::sqlx::query_as("SELECT state,terminal_event_id FROM agent_turns WHERE session_id=? AND operation_id=?")
        .bind(&original.conversation_id).bind(&original.canonical_operation_id).fetch_one(db.pool()).await.unwrap();
    assert_eq!(turn,("running".into(),None));
    assert_eq!(repository.get_execution_detail(OWNER_ID,&execution).await.unwrap().unwrap().attempts.len(),1);
}

#[tokio::test]
async fn user_pause_keeps_original_task_attached_and_does_not_become_technical_review() {
    let (db,repository,execution,running,original,lease,_)=original_admission_fixture().await;
    add_native_pause(&db,&original,"EXECUTION_USER_REQUESTED").await;
    let attempt=&running.current_attempt.as_ref().unwrap().attempt;
    let observed=repository.reconcile_recovered_attempt(OWNER_ID,&execution,&running.step.step_id,running.step.version,
        &attempt.attempt_id,attempt.version,&lease,None,&event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    assert_eq!(observed.disposition,AgentExecutionAttemptRecoveryDisposition::OngoingTurnObserved);
    assert_eq!(observed.detail.step.status,"running");assert_eq!(observed.detail.step.version,running.step.version);
    assert!(nomifun_db::AttemptConversationEffects::decode(observed.detail.current_attempt.unwrap().attempt.runtime_state.as_deref()).unwrap().review_blocked.is_none());
}

#[tokio::test]
async fn initial_input_acknowledges_admission_before_terminal_and_preserves_later_control_fifo() {
    let (db,repository,execution,running,original,lease,_)=original_admission_fixture().await;
    let attempt=&running.current_attempt.as_ref().unwrap().attempt;
    assert!(repository.acknowledge_initial_input_admission(OWNER_ID,&execution,&running.step.step_id,&attempt.attempt_id,
        attempt.version,&lease,&event(AgentExecutionEventKind::StepChanged)).await.unwrap());
    let observed=repository.get_step_detail(OWNER_ID,&execution,&running.step.step_id).await.unwrap().unwrap();
    assert_eq!(observed.step.version,running.step.version);assert_eq!(observed.step.status,"running");
    let retained=nomifun_db::AttemptConversationEffects::decode(observed.current_attempt.unwrap().attempt.runtime_state.as_deref()).unwrap();
    assert!(matches!(retained.pending_conversation_effects.as_slice(),[
        nomifun_db::PendingConversationEffect::Steer {operation_id,..},nomifun_db::PendingConversationEffect::StopTurn {..}]
        if operation_id=="exact-steer"));
    assert!(!repository.acknowledge_initial_input_admission(OWNER_ID,&execution,&running.step.step_id,&attempt.attempt_id,
        attempt.version,&lease,&event(AgentExecutionEventKind::StepChanged)).await.unwrap(),"lost ACK replay must not deliver another input or remove the next control");
    let count:i64=nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM agent_execution_events WHERE execution_id=? AND json_extract(payload,'$.effect')='initial_input'")
        .bind(&execution).fetch_one(db.pool()).await.unwrap();assert_eq!(count,1);
    let turn:(String,Option<String>)=nomifun_db::sqlx::query_as("SELECT state,terminal_event_id FROM agent_turns WHERE session_id=? AND operation_id=?")
        .bind(&original.conversation_id).bind(&original.canonical_operation_id).fetch_one(db.pool()).await.unwrap();
    assert_eq!(turn,("running".into(),None));
}

#[tokio::test]
async fn initial_input_ack_rejects_changed_bytes_and_broken_original_admission_pair() {
    for mutation in ["bytes","key","producer","cause","start_kind"] {
        let (db,repository,execution,running,original,lease,_)=original_admission_fixture().await;
        let attempt=&running.current_attempt.as_ref().unwrap().attempt;
        match mutation {
            "bytes"=>{nomifun_db::sqlx::query("UPDATE agent_events SET inline_json=json_set(inline_json,'$.content','different bytes') WHERE session_id=? AND kind='message/user-accepted'").bind(&original.conversation_id).execute(db.pool()).await.unwrap();},
            "key"=>{nomifun_db::sqlx::query("UPDATE agent_events SET idempotency_key='foreign-original-key' WHERE session_id=? AND kind='message/user-accepted'").bind(&original.conversation_id).execute(db.pool()).await.unwrap();},
            "producer"=>{nomifun_db::sqlx::query("UPDATE agent_events SET producer_id='untrusted' WHERE session_id=? AND kind='message/user-accepted'").bind(&original.conversation_id).execute(db.pool()).await.unwrap();},
            "cause"=>{nomifun_db::sqlx::query("UPDATE agent_events SET causation_event_id=(SELECT event_id FROM agent_events opening WHERE opening.session_id=agent_events.session_id AND opening.kind='session/opening' LIMIT 1) WHERE session_id=? AND kind='turn/started'").bind(&original.conversation_id).execute(db.pool()).await.unwrap();},
            _=>{nomifun_db::sqlx::query("UPDATE agent_events SET kind='runtime/progress-recorded' WHERE session_id=? AND kind='turn/started'").bind(&original.conversation_id).execute(db.pool()).await.unwrap();},
        }
        assert!(repository.acknowledge_initial_input_admission(OWNER_ID,&execution,&running.step.step_id,&attempt.attempt_id,
            attempt.version,&lease,&event(AgentExecutionEventKind::StepChanged)).await.is_err(),"{mutation}");
        let observed=repository.get_step_detail(OWNER_ID,&execution,&running.step.step_id).await.unwrap().unwrap();
        assert_eq!(observed.current_attempt.as_ref().unwrap().attempt.version,attempt.version);
        assert_eq!(nomifun_db::AttemptConversationEffects::decode(observed.current_attempt.unwrap().attempt.runtime_state.as_deref()).unwrap().pending_conversation_effects.len(),3);
    }
}

#[tokio::test]
async fn recovery_handoff_cannot_attach_after_a_user_stop_fence() {
    let (_db, repository, execution_id, running, _) = ongoing_fixture("running", false).await;
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
