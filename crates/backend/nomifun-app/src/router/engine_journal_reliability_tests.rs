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
async fn cleanup_retry_preserves_completed_previous_assistant_step() {
    let (journal, pool) = if let Some(root) = std::env::var_os("NOMIFUN_RELIABILITY_EVIDENCE_DIR") {
        let data = std::path::PathBuf::from(root).join("previous-assistant-step").join("data");
        std::fs::create_dir_all(&data).unwrap();
        test_fixture_at(&data.join("journal.db")).await
    } else {
        test_fixture().await
    };
    journal.append(serde_json::to_string(&AgentEngineEvent::OutputTextDelta {
        step: 1, text: "first response".into(),
    }).unwrap(), None, EngineJournalWrite::Progress).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_previous_completion BEFORE INSERT ON agent_events WHEN NEW.kind='message/completed' BEGIN SELECT RAISE(FAIL,'fixture previous step completion failure'); END")
        .execute(&pool).await.unwrap();
    let second = serde_json::to_string(&AgentEngineEvent::OutputTextDelta {
        step: 2, text: "second response".into(),
    }).unwrap();
    let failure = journal.append(second.clone(), None, EngineJournalWrite::Cleanup).await.unwrap_err();
    assert!(failure.to_string().contains("fixture previous step completion failure"));
    assert_eq!(journal.sequence(), 1);
    let before = journal.0.store.current_cursor(&journal.0.session).await.unwrap();
    assert!(journal.append(serde_json::to_string(&AgentEngineEvent::OutputTextDelta {
        step: 2, text: "different response".into(),
    }).unwrap(), None, EngineJournalWrite::Cleanup).await.is_err());
    assert_eq!(journal.0.store.current_cursor(&journal.0.session).await.unwrap(), before);
    sqlx::query("DROP TRIGGER reject_previous_completion").execute(&pool).await.unwrap();
    journal.append(second, None, EngineJournalWrite::Cleanup).await.unwrap();
    let first_id = canonical_assistant_step_message_id(journal.0.root.as_ref(), 1).unwrap();
    let completion: Vec<String> = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE session_id=? AND kind='message/completed' AND correlation_id=?")
        .bind(journal.0.session.as_ref()).bind(first_id).fetch_all(&pool).await.unwrap();
    assert_eq!(completion.len(), 1, "cleanup retry lost the previous response completion");
    let completed: Value = serde_json::from_str(&completion[0]).unwrap();
    assert_eq!(completed["part_count"], 1);
    assert_eq!(completed["content_digest"], digest_bytes(b"first response").as_ref());
    let parts: Vec<String> = sqlx::query_scalar("SELECT json_extract(inline_json,'$.content') FROM agent_events WHERE session_id=? AND kind='message/content-part' ORDER BY seq")
        .bind(journal.0.session.as_ref()).fetch_all(&pool).await.unwrap();
    assert_eq!(parts, vec!["first response", "second response"]);
    assert_eq!(journal.sequence(), 2);
}

#[tokio::test]
async fn failed_cleanup_write_allows_only_its_exact_receipt_retry() {
    let (journal,pool)=test_fixture().await;
    let cleanup=json!({"event":"host_cleanup_proven","source":"exact-owner"}).to_string();
    sqlx::query("CREATE TRIGGER reject_cleanup_journal BEFORE INSERT ON agent_events WHEN NEW.kind='runtime/progress-recorded' AND json_extract(NEW.inline_json,'$.event.event')='host_cleanup_proven' BEGIN SELECT RAISE(FAIL,'fixture cleanup write failure'); END")
        .execute(&pool).await.unwrap();
    assert!(journal.append(cleanup.clone(),None,EngineJournalWrite::Cleanup).await.is_err());
    assert_eq!(journal.sequence(),0);
    let before=journal.0.store.current_cursor(&journal.0.session).await.unwrap();
    assert!(journal.append(json!({"event":"host_cleanup_proven","source":"other-owner"}).to_string(),None,
        EngineJournalWrite::Cleanup).await.is_err());
    assert!(journal.append(serde_json::to_string(&AgentEngineEvent::ModelStepStarted {
        step:1,operation_id:"must-not-run".into()}).unwrap(),None,EngineJournalWrite::Progress).await.is_err());
    assert_eq!(journal.0.store.current_cursor(&journal.0.session).await.unwrap(),before);
    sqlx::query("DROP TRIGGER reject_cleanup_journal").execute(&pool).await.unwrap();
    journal.append(cleanup,None,EngineJournalWrite::Cleanup).await.unwrap();
    assert_eq!(journal.sequence(),1);
    journal.append(serde_json::to_string(&AgentEngineEvent::TurnCancelled {model_steps:2}).unwrap(),None,
        EngineJournalWrite::Terminal).await.unwrap();
    let state:String=sqlx::query_scalar("SELECT state FROM agent_turns WHERE session_id=?")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    assert_eq!(state,"cancelled");
}

#[tokio::test]
async fn partially_committed_terminal_retries_without_a_new_journal_record() {
    let (journal,pool)=test_fixture().await;
    journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_turn_cancelled BEFORE INSERT ON agent_events WHEN NEW.kind='turn/cancelled' BEGIN SELECT RAISE(FAIL,'fixture terminal commit failure'); END")
        .execute(&pool).await.unwrap();
    let terminal=serde_json::to_string(&AgentEngineEvent::TurnCancelled {model_steps:2}).unwrap();
    assert!(journal.append(terminal.clone(),None,EngineJournalWrite::Terminal).await.is_err());
    let before=journal.0.store.current_cursor(&journal.0.session).await.unwrap();
    assert!(journal.append(serde_json::to_string(&AgentEngineEvent::TurnCancelled {model_steps:3}).unwrap(),None,
        EngineJournalWrite::Terminal).await.is_err());
    assert_eq!(journal.0.store.current_cursor(&journal.0.session).await.unwrap(),before);
    sqlx::query("DROP TRIGGER reject_turn_cancelled").execute(&pool).await.unwrap();
    journal.append(terminal,None,EngineJournalWrite::Terminal).await.unwrap();
    let journal_records:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded'")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    let terminals:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/cancelled'")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    assert_eq!(journal_records,2);
    assert_eq!(terminals,1);
    assert_eq!(journal.sequence(),2);
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
