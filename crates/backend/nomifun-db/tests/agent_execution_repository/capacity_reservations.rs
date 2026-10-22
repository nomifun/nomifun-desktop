use super::*;
use nomifun_db::{execution_capacity_available_tx, execution_reservations_tx};

#[tokio::test]
async fn durable_cancelled_attempt_without_a_native_turn_cannot_start_its_original_operation_later() {
    let fixture=scoped_stop::Fixture::memory(&["cancel before native input"],&[]).await;
    let running=fixture.start("cancel before native input").await;
    let original=running.current_attempt.as_ref().unwrap();let session=original.conversation_id.as_ref().unwrap();
    let before=fixture.detail().await;
    fixture.repository.cancel_step(&fixture.owner,&fixture.execution_id,before.execution.version,&running.step.step_id,running.step.version,
        nomifun_common::AgentExecutionCancellationScope::Task,
        &NewAgentExecutionEvent {actor:nomifun_common::AgentExecutionActor::user(&fixture.owner),..event(AgentExecutionEventKind::StepChanged)}).await.unwrap();
    let mut tx=fixture.db.pool().begin().await.unwrap();
    let key=nomifun_db::execution_attempt_turn_key_tx(&mut tx,&fixture.owner,&fixture.execution_id,&running.step.step_id,&original.attempt.attempt_id).await.unwrap();
    assert_eq!(key,format!("{}:initial-turn",original.attempt.attempt_id));
    assert!(nomifun_db::execution_attempt_turn_key_tx(&mut tx,"foreign-owner",&fixture.execution_id,&running.step.step_id,&original.attempt.attempt_id).await.is_err());
    let operation=format!("turn:user:{}:{session}:{key}",fixture.owner);
    let absent:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM agent_turns WHERE session_id=? AND operation_id=?)")
        .bind(session).bind(&operation).fetch_one(&mut *tx).await.unwrap();assert!(absent);
    assert!(!nomifun_db::native_resume_target_eligible_tx(&mut tx,&fixture.owner,session,&operation).await.unwrap(),
        "the inactive historical Attempt and durable domain Stop fence remain authoritative");
    assert!(nomifun_db::native_execution_claim_admitted_tx(&mut tx,&fixture.owner,session,&operation).await.is_err(),
        "absence of the original Turn is safe only together with the same writer's irreversible admission fence");
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn exact_input_required_releases_capacity_for_every_tool_policy() {
    for policy in [AgentToolPolicy::Full, AgentToolPolicy::ReadShell, AgentToolPolicy::ReadOnly] {
        let db = database().await;
        let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
        let (execution, running, output) = user_input_fixture(&db, &repository, "ask_user").await;
        sqlx::query("UPDATE agent_execution_steps SET tool_policy=? WHERE step_id=?")
            .bind(policy.to_string()).bind(&running.step.step_id).execute(db.pool()).await.unwrap();
        let row = repository.get_execution(OWNER_ID, &execution).await.unwrap().unwrap();
        let lease = AgentExecutionLeaseToken::new("waiting-writer-proof".into());
        repository.try_acquire_lease(&execution, row.version, lease.owner(), nomifun_common::now_ms()+30_000).await.unwrap().unwrap();
        repository.reconcile_recovered_attempt(OWNER_ID, &execution, &running.step.step_id, running.step.version,
            &output.attempt_id, running.current_attempt.as_ref().unwrap().attempt.version, &lease, Some(&output),
            &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
        let row = repository.get_execution(OWNER_ID, &execution).await.unwrap().unwrap();
        let peer_id = nomifun_common::generate_id();
        let peer = step(peer_id.clone(), running.step.assigned_participant_id.clone(), "implementation peer");
        repository.append_steps(OWNER_ID, &execution, row.version, &nomifun_db::AppendAgentExecutionStepsParams {
            new_steps:vec![peer], new_dependencies:vec![],
        }, &event(AgentExecutionEventKind::PlanChanged)).await.unwrap();
        let mut tx = db.pool().begin().await.unwrap();
        let reservations = execution_reservations_tx(&mut tx, &execution).await.unwrap();
        assert_eq!(reservations.len(), 1);
        assert!(!reservations[0].capacity_held, "the actual closed InputRequired source releases its model slot");
        assert!(execution_capacity_available_tx(&mut tx, &execution, &peer_id, None).await.unwrap(),
            "tool authority does not reserve an entire shared workspace");
        tx.commit().await.unwrap();
        let review=nomifun_db::AttemptConversationEffects {pending_conversation_effects:vec![],review_blocked:Some(nomifun_db::RecoveryReviewBlock {
            kind:"external_effect".into(),operation_id:output.canonical_operation_id.clone(),receipt_state:"unknown".into(),reason:"native settlement requires review".into(),
        })};
        sqlx::query("UPDATE agent_execution_attempts SET runtime_state=? WHERE attempt_id=?")
            .bind(review.encode().unwrap()).bind(&output.attempt_id).execute(db.pool()).await.unwrap();
        let mut tx=db.pool().begin().await.unwrap();
        let reservations=execution_reservations_tx(&mut tx,&execution).await.unwrap();
        assert!(reservations[0].capacity_held,"review is not native cleanup proof, even beside an old valid question");
        assert!(!execution_capacity_available_tx(&mut tx, &execution, &peer_id, None).await.unwrap(),
            "an unresolved invocation still consumes its participant capacity");
        tx.commit().await.unwrap();
        sqlx::query("UPDATE agent_execution_attempts SET runtime_state=NULL WHERE attempt_id=?")
            .bind(&output.attempt_id).execute(db.pool()).await.unwrap();
        // A waiting display with an unconfirmed source cannot release model
        // capacity, even for ReadOnly tools and a previously valid terminal.
        sqlx::query("UPDATE agent_events SET kind_version=9 WHERE session_id=? AND kind='turn/completed'")
            .bind(&output.conversation_id).execute(db.pool()).await.unwrap();
        let mut tx = db.pool().begin().await.unwrap();
        let reservations = execution_reservations_tx(&mut tx, &execution).await.unwrap();
        assert!(reservations[0].capacity_held);
        tx.commit().await.unwrap(); db.close().await;
    }
}

#[tokio::test]
async fn active_full_and_shell_attempts_admit_peers_within_execution_and_participant_capacity() {
    for policy in [AgentToolPolicy::Full, AgentToolPolicy::ReadShell, AgentToolPolicy::ReadOnly] {
        let db = database().await;
        let repository = SqliteAgentExecutionRepository::new(db.pool().clone());
        let (execution, running, _) = recovery_fixture(&db, &repository).await;
        sqlx::query("UPDATE agent_execution_steps SET tool_policy=? WHERE step_id=?")
            .bind(policy.to_string()).bind(&running.step.step_id).execute(db.pool()).await.unwrap();
        let row = repository.get_execution(OWNER_ID, &execution).await.unwrap().unwrap();
        let peer_id = nomifun_common::generate_id();
        let third_id = nomifun_common::generate_id();
        let appended = repository.append_steps(OWNER_ID, &execution, row.version, &nomifun_db::AppendAgentExecutionStepsParams {
            new_steps:vec![step(peer_id.clone(), running.step.assigned_participant_id.clone(), "implementation peer"),
                step(third_id.clone(), running.step.assigned_participant_id.clone(), "queued third")],
            new_dependencies:vec![],
        }, &event(AgentExecutionEventKind::PlanChanged)).await.unwrap();
        let peer = appended.steps.iter().find(|step| step.step_id == peer_id).unwrap();
        let mut tx = db.pool().begin().await.unwrap();
        assert!(!execution_capacity_available_tx(&mut tx, &execution, &peer_id, None).await.unwrap(),
            "the configured participant concurrency of one still fences a second attempt");
        tx.commit().await.unwrap();
        sqlx::query("UPDATE agent_execution_participants SET constraints='{\"max_concurrency\":2}' WHERE execution_id=?")
            .bind(&execution).execute(db.pool()).await.unwrap();
        sqlx::query("UPDATE agent_executions SET max_parallel=1 WHERE execution_id=?")
            .bind(&execution).execute(db.pool()).await.unwrap();
        let mut tx = db.pool().begin().await.unwrap();
        assert!(!execution_capacity_available_tx(&mut tx, &execution, &peer_id, None).await.unwrap(),
            "the user's execution concurrency of one is preserved");
        tx.commit().await.unwrap();
        sqlx::query("UPDATE agent_executions SET max_parallel=2 WHERE execution_id=?")
            .bind(&execution).execute(db.pool()).await.unwrap();
        repository.create_attempt(OWNER_ID, &execution, &peer_id, peer.version, None,
            &CreateAgentExecutionAttemptParams {
                participant_id:peer.assigned_participant_id.clone(),start_immediately:false,
                trigger_reason:"initial".into(),effective_config:"{}".into(),retry_after:None,runtime_state:None,
            }, &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
        let mut tx = db.pool().begin().await.unwrap();
        let reservations = execution_reservations_tx(&mut tx, &execution).await.unwrap();
        assert_eq!(reservations.iter().filter(|reservation| reservation.capacity_held).count(), 2,
            "the active {policy} attempt and a full peer are admitted independently");
        assert!(!execution_capacity_available_tx(&mut tx, &execution, &third_id, None).await.unwrap(),
            "concurrent work must not exceed existing capacity");
        tx.commit().await.unwrap();
        db.close().await;
    }
}

#[tokio::test]
async fn exact_child_wait_yields_capacity_and_recovery_preserves_it_until_resolved() {
    use nomifun_db::{native_execution_claim_admitted_tx, native_resume_capacity_available_tx};
    let fixture = scoped_stop::Fixture::memory(&["waiting-parent", "full-child"], &[]).await;
    sqlx::query("UPDATE agent_executions SET max_parallel=1 WHERE execution_id=?")
        .bind(&fixture.execution_id).execute(fixture.db.pool()).await.unwrap();
    sqlx::query("UPDATE agent_execution_participants SET constraints='{\"max_concurrency\":1}' WHERE execution_id=?")
        .bind(&fixture.execution_id).execute(fixture.db.pool()).await.unwrap();
    sqlx::query("UPDATE agent_execution_steps SET tool_policy='full' WHERE execution_id=?")
        .bind(&fixture.execution_id).execute(fixture.db.pool()).await.unwrap();
    let parent = fixture.start("waiting-parent").await;
    let original = parent.current_attempt.as_ref().unwrap();
    let session = original.conversation_id.as_ref().unwrap();
    let operation = format!("turn:user:{}:{session}:{}:initial-turn", fixture.owner, original.attempt.attempt_id);
    let started = nomifun_common::generate_id();
    let progress = nomifun_common::generate_id();
    let wait = serde_json::json!({"execution_ids":[fixture.execution_id],"mode":"all"});
    sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,1,?,'session-api',?,'turn/started',1,?,'{}')")
        .bind(session).bind(&started).bind(&started).bind(&operation).execute(fixture.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json) VALUES(?,3,?,'runtime_supervisor',?,'runtime/progress-recorded',1,?,?,?)")
        .bind(session).bind(&progress).bind(&progress).bind(&operation).bind(&started)
        .bind(serde_json::json!({"producer_seq":3,"event":{"event":"awaiting_children","model_steps":1,"request":wait}}).to_string())
        .execute(fixture.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_session_heads(session_id,status,active_turn_id,active_set_generation,last_seq,unread_count) VALUES(?,'running',?,0,3,0)")
        .bind(session).bind(&operation).execute(fixture.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,started_event_id,accepted_at,started_at,execution_owner,execution_generation) VALUES(?,?,?,?,'running',?,1,1,'original-native-owner',2)")
        .bind(session).bind(&operation).bind(&operation).bind(&operation).bind(&started).execute(fixture.db.pool()).await.unwrap();
    let mut tx = fixture.db.pool().begin().await.unwrap();
    assert!(!execution_reservations_tx(&mut tx, &fixture.execution_id).await.unwrap()[0].capacity_held);
    assert!(!native_execution_claim_admitted_tx(&mut tx, &fixture.owner, session, &operation).await.unwrap(),
        "cold reconstruction needs the exact waiting checkpoint as well as the event");
    tx.commit().await.unwrap();
    let checkpoint = serde_json::json!({"turn_operation_id":operation,"awaiting_children":wait});
    let digest = nomifun_agent_contracts::digest_payload(&checkpoint).unwrap().0;
    sqlx::query("UPDATE agent_turns SET native_checkpoint_json=?,native_checkpoint_digest=?,native_checkpoint_revision=1,native_checkpoint_seq=2 WHERE session_id=?")
        .bind(checkpoint.to_string()).bind(&digest).bind(session).execute(fixture.db.pool()).await.unwrap();
    for (mutation, restore) in [
        ("UPDATE agent_events SET kind_version=9 WHERE event_id=?", "UPDATE agent_events SET kind_version=1 WHERE event_id=?"),
        ("UPDATE agent_events SET producer_id='foreign' WHERE event_id=?", "UPDATE agent_events SET producer_id='runtime_supervisor' WHERE event_id=?"),
        ("UPDATE agent_events SET correlation_id='foreign-turn' WHERE event_id=?", "UPDATE agent_events SET correlation_id=? WHERE event_id=?"),
        ("UPDATE agent_events SET causation_event_id=NULL WHERE event_id=?", "UPDATE agent_events SET causation_event_id=? WHERE event_id=?"),
    ] {
        sqlx::query(mutation).bind(&progress).execute(fixture.db.pool()).await.unwrap();
        let mut tx = fixture.db.pool().begin().await.unwrap();
        assert!(execution_reservations_tx(&mut tx, &fixture.execution_id).await.unwrap()[0].capacity_held,
            "a display or mismatched progress event cannot yield capacity: {mutation}");
        tx.commit().await.unwrap();
        let query = sqlx::query(restore);
        let query = if restore.contains("correlation_id=?") { query.bind(&operation) }
            else if restore.contains("causation_event_id=?") { query.bind(&started) } else { query };
        query.bind(&progress).execute(fixture.db.pool()).await.unwrap();
    }
    let mut tx = fixture.db.pool().begin().await.unwrap();
    assert!(native_execution_claim_admitted_tx(&mut tx, &fixture.owner, session, &operation).await.unwrap(),
        "the exact waiter can be reconstructed without taking a model slot");
    tx.commit().await.unwrap();
    let raw:String=sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND event_id=?")
        .bind(session).bind(&progress).fetch_one(fixture.db.pool()).await.unwrap();
    let value:serde_json::Value=serde_json::from_str(&raw).unwrap();
    let bytes=nomifun_agent_contracts::canonical_json_bytes(&value).unwrap();
    let payload=nomifun_common::generate_id();let payload_digest=nomifun_agent_contracts::digest_bytes(&bytes).0;
    sqlx::query("INSERT INTO agent_payloads(payload_id,session_id,media_type,byte_len,digest,body) VALUES(?,?,'application/json',?,?,?)")
        .bind(&payload).bind(session).bind(bytes.len() as i64).bind(&payload_digest)
        .bind(serde_json::to_vec(&nomifun_agent_contracts::SessionPayloadBody::Json(nomifun_agent_contracts::StrictJsonValue(value))).unwrap())
        .execute(fixture.db.pool()).await.unwrap();
    sqlx::query("UPDATE agent_events SET inline_json=NULL,payload_id=? WHERE session_id=? AND event_id=?")
        .bind(&payload).bind(session).bind(&progress).execute(fixture.db.pool()).await.unwrap();
    sqlx::query("UPDATE agent_payloads SET digest=? WHERE payload_id=?").bind("0".repeat(64)).bind(&payload).execute(fixture.db.pool()).await.unwrap();
    let mut tx=fixture.db.pool().begin().await.unwrap();
    assert!(execution_reservations_tx(&mut tx,&fixture.execution_id).await.is_err(),"a damaged stored wait cannot yield capacity");tx.rollback().await.unwrap();
    sqlx::query("UPDATE agent_payloads SET digest=? WHERE payload_id=?").bind(&payload_digest).bind(&payload).execute(fixture.db.pool()).await.unwrap();
    fixture.start("full-child").await;
    let mut tx = fixture.db.pool().begin().await.unwrap();
    let reservations = execution_reservations_tx(&mut tx, &fixture.execution_id).await.unwrap();
    assert_eq!(reservations.iter().filter(|reservation| reservation.capacity_held).count(), 1);
    assert!(!native_resume_capacity_available_tx(&mut tx, &fixture.owner, session, &operation).await.unwrap(),
        "model resumption must wait while the child owns the user's sole slot");
    tx.commit().await.unwrap();

    sqlx::query("UPDATE agent_turns SET execution_generation=4,execution_owner='recovered-native-owner',execution_lease_until=0 WHERE session_id=?")
        .bind(session).execute(fixture.db.pool()).await.unwrap();
    sqlx::query("UPDATE agent_session_heads SET last_seq=4 WHERE session_id=?")
        .bind(session).execute(fixture.db.pool()).await.unwrap();
    for checkpoint_seq in [2, 4] {
        sqlx::query("UPDATE agent_turns SET native_checkpoint_seq=? WHERE session_id=?")
            .bind(checkpoint_seq).bind(session).execute(fixture.db.pool()).await.unwrap();
        let mut tx = fixture.db.pool().begin().await.unwrap();
        assert!(!execution_reservations_tx(&mut tx, &fixture.execution_id).await.unwrap().iter()
            .find(|reservation| reservation.attempt_id == original.attempt.attempt_id).unwrap().capacity_held,
            "a new lease or another checkpoint of the same wait cannot steal the running child's slot");
        tx.commit().await.unwrap();
    }
    sqlx::query("UPDATE agent_turns SET native_checkpoint_digest=? WHERE session_id=?")
        .bind("0".repeat(64)).bind(session).execute(fixture.db.pool()).await.unwrap();
    let mut tx = fixture.db.pool().begin().await.unwrap();
    assert!(execution_reservations_tx(&mut tx, &fixture.execution_id).await.is_err(),
        "damaged recovery proof fails closed");
    tx.rollback().await.unwrap();
    sqlx::query("UPDATE agent_turns SET native_checkpoint_digest=? WHERE session_id=?")
        .bind(&digest).bind(session).execute(fixture.db.pool()).await.unwrap();
    fixture.complete("full-child").await;
    let mut tx = fixture.db.pool().begin().await.unwrap();
    assert!(native_resume_capacity_available_tx(&mut tx, &fixture.owner, session, &operation).await.unwrap());
    tx.commit().await.unwrap();
    let resolved = nomifun_common::generate_id();
    sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json) VALUES(?,5,?,'runtime_supervisor',?,'runtime/progress-recorded',1,?,?,?)")
        .bind(session).bind(&resolved).bind(&resolved).bind(&operation).bind(&started)
        .bind(serde_json::json!({"producer_seq":5,"event":{"event":"children_wait_resolved","model_steps":1,"cursor":"children-settled"}}).to_string())
        .execute(fixture.db.pool()).await.unwrap();
    sqlx::query("UPDATE agent_session_heads SET last_seq=5 WHERE session_id=?")
        .bind(session).execute(fixture.db.pool()).await.unwrap();
    let mut tx = fixture.db.pool().begin().await.unwrap();
    assert!(execution_reservations_tx(&mut tx, &fixture.execution_id).await.unwrap()[0].capacity_held,
        "the resolved event supersedes the earlier waiting checkpoint");
    tx.commit().await.unwrap();
    fixture.db.close().await;
}
