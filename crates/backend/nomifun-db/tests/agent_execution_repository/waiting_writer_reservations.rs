use super::*;
use nomifun_db::{execution_capacity_available_tx, execution_reservations_tx};

#[tokio::test]
async fn input_required_writer_keeps_workspace_responsibility_without_consuming_a_model_slot() {
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
        let mut peer = step(peer_id.clone(), running.step.assigned_participant_id.clone(), "read-only peer");
        peer.tool_policy = AgentToolPolicy::ReadOnly;
        repository.append_steps(OWNER_ID, &execution, row.version, &nomifun_db::AppendAgentExecutionStepsParams {
            new_steps:vec![peer], new_dependencies:vec![],
        }, &event(AgentExecutionEventKind::PlanChanged)).await.unwrap();
        let mut tx = db.pool().begin().await.unwrap();
        let reservations = execution_reservations_tx(&mut tx, &execution).await.unwrap();
        assert_eq!(reservations.len(), 1);
        assert!(!reservations[0].capacity_held, "the actual closed InputRequired source releases its model slot");
        assert_eq!(reservations[0].workspace_held, policy != AgentToolPolicy::ReadOnly);
        assert_eq!(execution_capacity_available_tx(&mut tx, &execution, &peer_id, None).await.unwrap(), policy == AgentToolPolicy::ReadOnly,
            "closed Turn does not prove an arbitrary authorized background writer is gone; safe ReadOnly waiting still permits peers");
        tx.commit().await.unwrap();
        let review=nomifun_db::AttemptConversationEffects {pending_conversation_effects:vec![],review_blocked:Some(nomifun_db::RecoveryReviewBlock {
            kind:"external_effect".into(),operation_id:output.canonical_operation_id.clone(),receipt_state:"unknown".into(),reason:"native settlement requires review".into(),
        })};
        sqlx::query("UPDATE agent_execution_attempts SET runtime_state=? WHERE attempt_id=?")
            .bind(review.encode().unwrap()).bind(&output.attempt_id).execute(db.pool()).await.unwrap();
        let mut tx=db.pool().begin().await.unwrap();
        let reservations=execution_reservations_tx(&mut tx,&execution).await.unwrap();
        assert!(reservations[0].capacity_held&&reservations[0].workspace_held,"review is not native cleanup proof, even beside an old valid question");
        tx.commit().await.unwrap();
        sqlx::query("UPDATE agent_execution_attempts SET runtime_state=NULL WHERE attempt_id=?")
            .bind(&output.attempt_id).execute(db.pool()).await.unwrap();
        // A waiting display with an unconfirmed source cannot release either
        // reservation, even for ReadOnly tools and a previously valid terminal.
        sqlx::query("UPDATE agent_events SET kind_version=9 WHERE session_id=? AND kind='turn/completed'")
            .bind(&output.conversation_id).execute(db.pool()).await.unwrap();
        let mut tx = db.pool().begin().await.unwrap();
        let reservations = execution_reservations_tx(&mut tx, &execution).await.unwrap();
        assert!(reservations[0].capacity_held && reservations[0].workspace_held);
        tx.commit().await.unwrap(); db.close().await;
    }
}
