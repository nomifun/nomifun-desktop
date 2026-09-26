use super::*;
use nomifun_agent_runtime::{AgentExecutionCheckpoint, AgentExecutionSegmentState, AgentSegmentPolicy, EngineBinding};

fn checkpoint(journal:&EngineTurnJournal, renewed:bool) -> AgentExecutionCheckpoint {
    AgentExecutionCheckpoint { version:1,
        binding:EngineBinding::new(journal.0.session.clone(),"native-binding".into(),"test".into(),"a".repeat(64).into(),journal.0.snapshot.clone()).unwrap(),
        turn_operation_id:journal.0.operation.clone(),active_set_generation:0,model_steps:u16::from(renewed),tool_call_count:0,
        accepted_input_count:1,applied_steering_receipts:vec![],plan:Default::default(),work:Default::default(),patch_recovery:Default::default(),
        control_rejections:Default::default(),segments:Some(AgentExecutionSegmentState {
            policy:AgentSegmentPolicy { max_segments:2,max_no_progress_segments:2 },model_steps_per_segment:1,
            segment:if renewed {2} else {1},segment_start_step:u16::from(renewed),no_progress_segments:0,
            progress_at_segment_start:0,progress_fingerprints:vec![],
        }) }
}

fn owner(journal:&EngineTurnJournal) -> nomifun_agent_contracts::PrincipalRef {
    nomifun_agent_contracts::PrincipalRef { principal_kind:"user".into(),principal_id:journal.0.user.clone() }
}

#[tokio::test]
async fn journal_window_renews_only_after_checkpoint_ack_and_keeps_cumulative_usage() {
    let (journal,_) = test_fixture().await;
    journal.save_execution_checkpoint(checkpoint(&journal,false),owner(&journal)).await.unwrap().unwrap();
    journal.append(serde_json::to_string(&AgentEngineEvent::ModelStepStarted { step:1,operation_id:"model:1".into() }).unwrap(),None,EngineJournalWrite::Progress).await.unwrap();
    let (before_sequence,before_total) = { let cursor=journal.0.cursor.lock().await; assert!(cursor.bytes>0); (cursor.sequence,cursor.total_bytes) };
    let receipt=journal.save_execution_checkpoint(checkpoint(&journal,true),owner(&journal)).await.unwrap().unwrap();
    let cursor=journal.0.cursor.lock().await;
    assert_eq!(cursor.segment,2); assert_eq!(cursor.bytes,0);
    assert_eq!(cursor.window_start_sequence,before_sequence+1);
    assert!(cursor.total_bytes>before_total);
    assert_eq!(cursor.checkpoint_revision,receipt.revision);
    drop(cursor);
    let saved=journal.0.store.load_native_checkpoint(&owner(&journal),&journal.0.session,&journal.0.operation).await.unwrap().unwrap();
    assert_eq!(saved.through_seq,receipt.through_seq);
    assert_eq!(saved.state.0["model_steps"],1);
    assert_eq!(saved.state.0["segments"]["policy"]["max_segments"],2);
}

#[tokio::test]
async fn lost_checkpoint_commit_never_buys_a_new_window_or_erases_usage() {
    let (journal,pool)=test_fixture().await;
    journal.save_execution_checkpoint(checkpoint(&journal,false),owner(&journal)).await.unwrap().unwrap();
    let before_head=journal.0.store.current_cursor(&journal.0.session).await.unwrap();
    let before={ let cursor=journal.0.cursor.lock().await; (cursor.sequence,cursor.total_bytes,cursor.bytes,cursor.window_start_sequence) };
    // Deterministic persistence fault in the isolated test database. The
    // metadata event and checkpoint body must roll back as one transaction.
    sqlx::query("CREATE TRIGGER reject_native_checkpoint BEFORE UPDATE OF native_checkpoint_json ON agent_turns BEGIN SELECT RAISE(FAIL,'injected checkpoint commit loss'); END")
        .execute(&pool).await.unwrap();
    assert!(journal.save_execution_checkpoint(checkpoint(&journal,true),owner(&journal)).await.is_err());
    assert_eq!(journal.0.store.current_cursor(&journal.0.session).await.unwrap(),before_head);
    let cursor=journal.0.cursor.lock().await;
    assert_eq!((cursor.sequence,cursor.total_bytes,cursor.bytes,cursor.window_start_sequence),before);
    assert_eq!(cursor.segment,1); assert!(cursor.uncertain);
    drop(cursor);
    assert!(journal.append(serde_json::to_string(&AgentEngineEvent::ModelStepStarted { step:1,operation_id:"must-not-start".into() }).unwrap(),None,EngineJournalWrite::Progress).await.is_err());
    let saved=journal.0.store.load_native_checkpoint(&owner(&journal),&journal.0.session,&journal.0.operation).await.unwrap().unwrap();
    assert_eq!(saved.revision,1);
    assert_eq!(saved.state.0["segments"]["segment"],1);
}

#[tokio::test]
async fn pause_requires_cleanup_phase_and_never_appends_a_terminal_turn() {
    let (journal,pool)=test_fixture().await;
    journal.save_execution_checkpoint(checkpoint(&journal,false),owner(&journal)).await.unwrap().unwrap();
    let paused=serde_json::to_string(&AgentEngineEvent::TurnPaused { model_steps:0,reason:"EXECUTION_USER_REQUESTED".into() }).unwrap();
    assert!(journal.append(paused.clone(),None,EngineJournalWrite::Terminal).await.is_err());
    journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await.unwrap();
    journal.append(paused,None,EngineJournalWrite::Terminal).await.unwrap();
    let inspection=journal.0.store.inspect_latest_native_execution(&owner(&journal),&journal.0.session).await.unwrap().unwrap();
    assert_eq!(inspection.state,"paused"); assert_eq!(inspection.turn_state,"running");
    assert!(inspection.pause.unwrap().cleanup_proven && inspection.checkpoint_retained);
    let terminals:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind IN ('turn/completed','turn/failed','turn/cancelled')")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    assert_eq!(terminals,0);
}
