use super::*;

async fn held_external_writer()->(tempfile::TempDir,EngineTurnJournal,nomifun_db::SqlitePool,nomifun_db::SqlitePool,nomifun_db::sqlx::Transaction<'static,nomifun_db::sqlx::Sqlite>) {
    let root=tempfile::tempdir().unwrap();let path=root.path().join("journal-contention.db");
    let (journal,pool)=test_fixture_at(&path).await;
    let mut connections=Vec::new();
    for _ in 0..pool.options().get_max_connections() {
        let mut connection=pool.acquire().await.unwrap();
        sqlx::query("PRAGMA busy_timeout=1").execute(&mut *connection).await.unwrap();
        connections.push(connection);
    }
    drop(connections);
    let external=sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(&path)).await.unwrap();
    let writer=external.begin_with("BEGIN IMMEDIATE").await.unwrap();
    (root,journal,pool,external,writer)
}

fn failed_outcome()->StrictJsonValue {
    StrictJsonValue(serde_json::to_value(nomifun_ai_agent::engine_sdk::EngineTurnOutcome::failed("original execution failure")).unwrap())
}

fn failed_event()->String {
    serde_json::to_string(&AgentEngineEvent::TurnFailed {model_steps:0,message:"original execution failure".into(),failure:None}).unwrap()
}

#[tokio::test]
async fn pre_driver_cleanup_diagnostic_is_bounded_without_truncating_the_original_outcome() {
    let (journal,_pool)=test_fixture().await;
    let original=failed_outcome();
    let diagnostic="原恢复准备失败".repeat(5000);
    let receipt=journal.handoff_failed_cleanup(original.clone(),&diagnostic).await.unwrap();
    assert_eq!(receipt.outcome(),&original);
    journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await.unwrap();
    journal.append(failed_event(),None,EngineJournalWrite::Terminal).await.unwrap();
    assert!(journal.0.store.pending_native_turn_settlements(0,500).await.unwrap().is_empty());
}

#[tokio::test]
async fn successful_handoff_retries_the_original_owner_result_without_admitting_native_work() {
    let (journal,pool)=test_fixture().await;
    sqlx::query("CREATE TRIGGER reject_original_projection BEFORE INSERT ON agent_events WHEN NEW.kind='tool/call-started' BEGIN SELECT RAISE(FAIL,'fixture projection write failure'); END")
        .execute(&pool).await.unwrap();
    let dispatch=json!({"event":"host_tool_dispatch","dispatch":{"operation_id":"original-owner-operation","call_id":"original-call", "capability_id":"fixture","action_id":"fixture/read","model_name":"read_fixture"}}).to_string();
    assert!(journal.append(dispatch,None,EngineJournalWrite::Progress).await.is_err());
    assert_eq!(journal.sequence(),0);
    let handoff=journal.handoff_failed_cleanup(failed_outcome(),"fixture projection write failure").await.unwrap();
    sqlx::query("DROP TRIGGER reject_original_projection").execute(&pool).await.unwrap();
    journal.retry_pending_owner_observations().await.unwrap();
    let settled=json!({"event":"host_tool_settled","operation_id":"original-owner-operation","call_id":"original-call", "owner_invoked":false,
        "result":nomifun_agent_runtime::AgentToolResult::text("original-call".into(),"not invoked",true),"error":null}).to_string();
    journal.append_owner_settlement(settled.clone()).await.unwrap();
    let sequence=journal.sequence();
    journal.append_owner_settlement(settled).await.unwrap();
    assert_eq!(journal.sequence(),sequence,"owner ACK replay must not append another producer record");
    assert!(journal.append(json!({"event":"model_step_started","step":1,"operation_id":"forbidden"}).to_string(),None,EngineJournalWrite::Progress).await.is_err());
    journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await.unwrap();
    journal.append(failed_event(),None,EngineJournalWrite::Terminal).await.unwrap();
    assert!(journal.0.store.pending_native_turn_settlements(0,500).await.unwrap().is_empty());
    assert_eq!(journal.0.store.read_turn_receipt(&journal.0.session,&journal.0.operation).await.unwrap().status,TurnReceiptStatus::Failed);
    assert!(journal.0.store.read_native_turn_settlement(&owner(&journal),&journal.0.session,&journal.0.operation).await.unwrap().is_some_and(|receipt|receipt.event_id==handoff.event_id));
    let models:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='model_step_started'")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    assert_eq!(models,0);
}

#[tokio::test]
async fn cleanup_proven_before_terminal_fault_is_reused_by_the_exact_handoff_retry() {
    let (journal,pool)=test_fixture().await;
    journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_original_terminal BEFORE INSERT ON agent_events WHEN NEW.kind='turn/failed' BEGIN SELECT RAISE(FAIL,'fixture terminal write failure'); END")
        .execute(&pool).await.unwrap();
    assert!(journal.append(failed_event(),None,EngineJournalWrite::Terminal).await.is_err());
    let receipt=journal.handoff_failed_cleanup(failed_outcome(),"fixture terminal write failure").await.unwrap();
    assert!(journal.0.store.native_turn_settlement_cleanup_proven(&receipt).await.unwrap());
    sqlx::query("DROP TRIGGER reject_original_terminal").execute(&pool).await.unwrap();
    journal.retry_pending_owner_observations().await.unwrap();
    assert!(journal.0.cursor.lock().await.terminal);
    let cleanup:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='host_cleanup_proven'")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    assert_eq!(cleanup,1,"the original successful cleanup is not repeated");
    assert_eq!(journal.0.store.read_turn_receipt(&journal.0.session,&journal.0.operation).await.unwrap().status,TurnReceiptStatus::Failed);
}

#[tokio::test]
async fn cold_settlement_uses_the_original_canonical_parts_and_saved_exit_without_a_runtime() {
    let (journal,_pool)=test_fixture().await;
    journal.append(serde_json::to_string(&AgentEngineEvent::OutputTextDelta {step:1,text:"retained original response".into(),source:None}).unwrap(),None,EngineJournalWrite::Progress).await.unwrap();
    let receipt=journal.handoff_failed_cleanup(failed_outcome(),"initial cleanup rejection").await.unwrap();
    journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await.unwrap();
    let store=journal.0.store.clone();let context=journal.0.error_context.clone();let session=journal.0.session.clone();let operation=journal.0.operation.clone();
    drop(journal);
    assert!(EngineTurnJournal::settle_cold_native_turn(&store,&receipt,&context).await.unwrap());
    assert_eq!(store.read_turn_receipt(&session,&operation).await.unwrap().status,TurnReceiptStatus::Failed);
    let facts=store.chat_causality_facts(&session,&operation).await.unwrap();
    let message=facts.events.iter().find(|event|event.kind.0=="message/completed").unwrap();
    assert_eq!(facts.event_payloads[message.event_id.as_ref()]["part_count"],1);
    assert_eq!(facts.event_payloads[message.event_id.as_ref()]["content_digest"],digest_bytes(b"retained original response").as_ref());
    assert!(store.pending_native_turn_settlements(0,500).await.unwrap().is_empty());
}

#[tokio::test]
async fn cold_success_restores_exact_outcome_and_original_parts_once() {
    use nomifun_ai_agent::engine_sdk::{EngineTurnOutcome, EngineTurnTerminal};
    use nomifun_chat_model_broker::ChatFinishReason;

    let (journal,_pool)=test_fixture().await;
    let chunks=["原始成功答复：\n", "eight original child results retained 🦀"];
    for text in chunks {
        journal.append(serde_json::to_string(&AgentEngineEvent::OutputTextDelta {
            step:3,text:text.into(),source:None,
        }).unwrap(),None,EngineJournalWrite::Progress).await.unwrap();
    }
    let original=EngineTurnOutcome {model_steps:3,
        terminal:EngineTurnTerminal::Completed {finish_reason:ChatFinishReason::Completed}};
    let expected=serde_json::to_value(&original).unwrap();
    let before=journal.0.store.chat_causality_facts(&journal.0.session,&journal.0.operation).await.unwrap();
    let original_parts=before.events.iter().filter(|event|event.kind.0=="message/content-part")
        .map(|event|(event.event_id.clone(),serde_json::to_vec(&before.event_payloads[event.event_id.as_ref()]).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(original_parts.len(),2);
    let receipt=journal.handoff_failed_cleanup(StrictJsonValue(expected.clone()),"original success awaiting cleanup receipt").await.unwrap();
    assert_eq!(receipt.outcome.0,expected);
    journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await.unwrap();
    let store=journal.0.store.clone();let context=journal.0.error_context.clone();
    let session=journal.0.session.clone();let operation=journal.0.operation.clone();
    drop(journal);

    assert!(EngineTurnJournal::settle_cold_native_turn(&store,&receipt,&context).await.unwrap());
    assert!(EngineTurnJournal::settle_cold_native_turn(&store,&receipt,&context).await.unwrap());
    assert_eq!(store.read_turn_receipt(&session,&operation).await.unwrap().status,TurnReceiptStatus::Completed);
    let facts=store.chat_causality_facts(&session,&operation).await.unwrap();
    let parts=facts.events.iter().filter(|event|event.kind.0=="message/content-part")
        .map(|event|(event.event_id.clone(),serde_json::to_vec(&facts.event_payloads[event.event_id.as_ref()]).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(parts,original_parts,"cold settlement must keep the exact original part identities and bytes");
    let completed=facts.events.iter().filter(|event|event.kind.0=="message/completed").collect::<Vec<_>>();
    assert_eq!(completed.len(),1);
    let message=&facts.event_payloads[completed[0].event_id.as_ref()];
    assert_eq!(message["part_count"],2);
    assert_eq!(message["content_digest"],digest_bytes(chunks.concat().as_bytes()).as_ref());
    let terminal=facts.events.iter().filter(|event|event.kind.0=="turn/completed").collect::<Vec<_>>();
    assert_eq!(terminal.len(),1);
    let terminal=&facts.event_payloads[terminal[0].event_id.as_ref()];
    assert_eq!(terminal["model_steps"],expected["model_steps"]);
    assert_eq!(terminal["finish_reason"],expected["terminal"]["finish_reason"]);
    assert!(!facts.events.iter().any(|event|matches!(event.kind.0.as_str(),"turn/failed"|"turn/cancelled"|"tool/call-started")));
    let native=facts.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded")
        .map(|event|&facts.event_payloads[event.event_id.as_ref()]["event"]).collect::<Vec<_>>();
    assert_eq!(native.iter().filter(|event|event["event"]=="turn_completed").count(),1);
    assert!(!native.iter().any(|event|matches!(event["event"].as_str(),
        Some("model_step_started"|"tool_started"|"host_tool_dispatch"|"host_resource_dispatch"))));
    assert!(store.pending_native_turn_settlements(0,500).await.unwrap().is_empty());
}

#[tokio::test]
async fn cold_resumed_settlement_preserves_prior_pause_and_checks_only_its_exact_generation_exit() {
    use nomifun_agent_session::{NativeExecutionClaim,NativeResumePreparation,NativeResumeRequest};
    for conflicting_current_exit in [false,true] {
        let (mut journal,_pool)=test_fixture().await;
        let mut checkpoint=checkpoint(&journal,false);checkpoint.segments=None;
        journal.append(serde_json::to_string(&AgentEngineEvent::TurnStarted {binding:checkpoint.binding.clone(),turn_operation_id:journal.0.operation.clone()}).unwrap(),None,EngineJournalWrite::Progress).await.unwrap();
        let saved=journal.save_execution_checkpoint(checkpoint.clone(),owner(&journal)).await.unwrap().unwrap();
        journal.0.store.request_native_pause(&owner(&journal),&journal.0.session,&journal.0.operation,"original-user-pause","inspect before continuing").await.unwrap();
        journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await.unwrap();
        let paused=AgentEngineEvent::TurnPaused {model_steps:0,reason:"EXECUTION_USER_REQUESTED".into()};
        journal.append(serde_json::to_string(&paused).unwrap(),None,EngineJournalWrite::Terminal).await.unwrap();
        let pause=journal.0.store.native_pause_state(&journal.0.session,&journal.0.operation).await.unwrap().unwrap();
        assert!(pause.cleanup_proven);
        let original_generation=journal.generation();
        let original=journal.0.store.native_recovery_facts(&journal.0.session,&journal.0.operation).await.unwrap();
        let original_pause=original.events.iter().find(|record|original.event_payloads.get(record.event_id.as_ref())
            .is_some_and(|payload|payload.pointer("/event/event").and_then(Value::as_str)==Some("turn_paused"))).unwrap().clone();
        let request=NativeResumeRequest {operation_id:journal.0.operation.clone(),idempotency_key:"explicit-original-resume".into(),
            expected_pause_revision:pause.revision,expected_checkpoint_revision:saved.revision,expected_checkpoint_digest:saved.digest,
            budget:Default::default(),cleanup_attestation:None};
        journal.0.store.commit_native_resume(&owner(&journal),&journal.0.session,&request,NativeResumePreparation {
            expected_head_seq:original.head.last_seq,expected_fence:original.execution_fence,snapshot:journal.0.snapshot.clone(),
            active_set_generation:0,checkpoint_state:StrictJsonValue(serde_json::to_value(checkpoint).unwrap()),observations:vec![],
        }).await.unwrap();
        let saved=journal.0.store.load_native_checkpoint(&owner(&journal),&journal.0.session,&journal.0.operation).await.unwrap().unwrap();
        let facts=journal.0.store.native_recovery_facts(&journal.0.session,&journal.0.operation).await.unwrap();
        let lease=journal.0.store.claim_native_execution(NativeExecutionClaim {
            owner:owner(&journal),agent_session_id:journal.0.session.clone(),operation_id:journal.0.operation.clone(),snapshot:journal.0.snapshot.clone(),
            active_set_generation:0,holder:"exact-resumed-settlement-owner".into(),expected_fence:facts.execution_fence,
            checkpoint:Some((saved.revision,saved.digest,saved.through_seq)),
        }).await.unwrap();
        assert!(lease.generation()>original_generation&&original_pause.seq<lease.generation());
        let sequence=facts.events.iter().filter(|record|record.kind.0=="runtime/progress-recorded").count() as u64;
        // The test reacquires the actual canonical claim, keeping its full
        // producer history. Only the process-local writer cursor is new.
        let writer=Arc::get_mut(&mut journal.0).unwrap();
        writer.generation=i64::try_from(lease.generation()).unwrap();writer.lease=lease;
        *writer.cursor.get_mut()=Cursor {sequence,window_start_sequence:sequence,..Default::default()};
        writer.sequence.store(sequence,Ordering::Release);
        if conflicting_current_exit {
            // A contradictory native exit in this generation must still fail
            // closed; the historical-pause exemption cannot hide it.
            journal.append(serde_json::to_string(&paused).unwrap(),None,EngineJournalWrite::Progress).await.unwrap();
        }
        let receipt=journal.handoff_failed_cleanup(failed_outcome(),"resumed original terminal remains unconfirmed").await.unwrap();
        assert_eq!(receipt.generation(),journal.generation());
        journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await.unwrap();
        let store=journal.0.store.clone();let context=journal.0.error_context.clone();
        let session=journal.0.session.clone();let operation=journal.0.operation.clone();
        drop(journal);
        let settled=EngineTurnJournal::settle_cold_native_turn(&store,&receipt,&context).await;
        if conflicting_current_exit {
            assert!(settled.is_err(),"a different exit in this exact generation is not historical");
            assert_eq!(store.read_turn_receipt(&session,&operation).await.unwrap().status,TurnReceiptStatus::Running);
            assert_eq!(store.pending_native_turn_settlements(0,500).await.unwrap().len(),1);
        } else {
            assert!(settled.unwrap(),"a prior genuine user pause cannot block this resumed generation's saved exit");
            assert!(EngineTurnJournal::settle_cold_native_turn(&store,&receipt,&context).await.unwrap());
            assert_eq!(store.read_turn_receipt(&session,&operation).await.unwrap().status,TurnReceiptStatus::Failed);
            assert!(store.pending_native_turn_settlements(0,500).await.unwrap().is_empty());
        }
        let facts=store.chat_causality_facts(&session,&operation).await.unwrap();
        assert_eq!(facts.events.iter().find(|record|record.event_id==original_pause.event_id),Some(&original_pause),"the original pause identity and bytes remain canonical history");
        assert_eq!(facts.event_payloads[original_pause.event_id.as_ref()],original.event_payloads[original_pause.event_id.as_ref()]);
        assert_eq!(facts.events.iter().filter(|record|record.kind.0=="turn/resume-authorized").count(),1,"cold settlement never starts another generation");
        assert!(!facts.event_payloads.values().any(|payload|matches!(payload.pointer("/event/event").and_then(Value::as_str),Some("model_step_started"|"tool_started"|"host_tool_dispatch"|"host_resource_dispatch"))));
    }
}

#[tokio::test]
async fn user_pause_arriving_after_handoff_wins_without_closing_or_restarting_the_turn() {
    let (journal,_pool)=test_fixture().await;
    let receipt=journal.handoff_failed_cleanup(failed_outcome(),"initial cleanup rejection").await.unwrap();
    journal.0.store.request_native_pause(&owner(&journal),&journal.0.session,&journal.0.operation,"late-pause","inspect current work").await.unwrap();
    assert!(journal.0.store.native_turn_settlement_requires_user_pause(&receipt).await.unwrap());
    journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await.unwrap();
    assert!(journal.append(failed_event(),None,EngineJournalWrite::Terminal).await.is_err());
    journal.append(serde_json::to_string(&AgentEngineEvent::TurnPaused {model_steps:0,reason:"EXECUTION_USER_REQUESTED".into()}).unwrap(),None,EngineJournalWrite::Terminal).await.unwrap();
    let pause=journal.0.store.native_pause_state(&journal.0.session,&journal.0.operation).await.unwrap().unwrap();
    assert_eq!(pause.reason,"EXECUTION_USER_REQUESTED");assert!(pause.cleanup_proven);
    assert_eq!(journal.0.store.read_turn_receipt(&journal.0.session,&journal.0.operation).await.unwrap().status,TurnReceiptStatus::Running);
    assert!(journal.0.store.pending_native_turn_settlements(0,500).await.unwrap().is_empty());
    let facts=journal.0.store.chat_causality_facts(&journal.0.session,&journal.0.operation).await.unwrap();
    assert!(!facts.events.iter().any(|event|matches!(event.kind.0.as_str(),"turn/failed"|"turn/completed"|"turn/resume-authorized")));
    assert!(!facts.event_payloads.values().any(|value|value.pointer("/event/event").and_then(Value::as_str)==Some("turn_failed")));
}

#[tokio::test]
async fn typed_busy_retries_only_the_same_progress_fact_and_keeps_the_original_runtime_stack() {
    let (_root,journal,pool,external,writer)=held_external_writer().await;
    let observed=journal.0.contention_observed.notified();tokio::pin!(observed);observed.as_mut().enable();
    let original=journal.clone();
    let write=tokio::spawn(async move {original.append(serde_json::to_string(&AgentEngineEvent::WorkStatus {status:Default::default()}).unwrap(),None,EngineJournalWrite::Progress).await});
    tokio::time::timeout(std::time::Duration::from_secs(5),observed).await.unwrap();
    writer.rollback().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5),write).await.unwrap().unwrap().unwrap();
    assert_eq!(journal.sequence(),1);
    assert!(journal.0.store.pending_native_turn_settlements(0,500).await.unwrap().is_empty(),"transient persistence does not exit the Runtime into technical pause");
    assert_eq!(journal.0.store.head(&journal.0.session).await.unwrap().status,"running");
    let records:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded'")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();assert_eq!(records,1);
    external.close().await;pool.close().await;
}

#[tokio::test]
async fn typed_busy_checkpoint_reuses_the_same_revision_state_and_metadata_without_replaying_work() {
    let (_root,journal,pool,external,writer)=held_external_writer().await;
    let observed=journal.0.contention_observed.notified();tokio::pin!(observed);observed.as_mut().enable();
    let original=journal.clone();let state=checkpoint(&journal,false);let expected=serde_json::to_value(&state).unwrap();let principal=owner(&journal);
    let write=tokio::spawn(async move {original.save_execution_checkpoint(state,principal).await});
    tokio::time::timeout(std::time::Duration::from_secs(5),observed).await.unwrap();
    writer.rollback().await.unwrap();
    let receipt=tokio::time::timeout(std::time::Duration::from_secs(5),write).await.unwrap().unwrap().unwrap().unwrap();
    assert_eq!(receipt.revision,1);assert_eq!(journal.sequence(),1);
    let saved=journal.0.store.load_native_checkpoint(&owner(&journal),&journal.0.session,&journal.0.operation).await.unwrap().unwrap();
    assert_eq!(saved.state.0,expected);assert_eq!(saved.revision,1);assert_eq!(saved.through_seq,receipt.through_seq);
    assert!(journal.0.store.pending_native_turn_settlements(0,500).await.unwrap().is_empty());
    external.close().await;pool.close().await;
}

#[tokio::test]
async fn user_stop_preempts_typed_busy_backoff_without_fabricating_the_uncommitted_fact() {
    let (_root,journal,pool,external,writer)=held_external_writer().await;
    let observed=journal.0.contention_observed.notified();tokio::pin!(observed);observed.as_mut().enable();
    let original=journal.clone();
    let write=tokio::spawn(async move {original.append(serde_json::to_string(&AgentEngineEvent::WorkStatus {status:Default::default()}).unwrap(),None,EngineJournalWrite::Progress).await});
    tokio::time::timeout(std::time::Duration::from_secs(5),observed).await.unwrap();
    journal.0.cancellation.cancel();
    let error=tokio::time::timeout(std::time::Duration::from_secs(2),write).await.unwrap().unwrap().unwrap_err();
    assert!(matches!(error,AppError::DatabaseUnavailable {..}),"typed contention must not become prose-based retry policy: {error}");
    writer.rollback().await.unwrap();
    journal.0.store.cancel_active_turn(&journal.0.session,"original-user-stop".into(),"session_api".into()).await.unwrap();
    journal.retry_pending_owner_observations().await.unwrap();
    assert_eq!(journal.sequence(),0);
    let records:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded'")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();assert_eq!(records,0);
    journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await.unwrap();
    external.close().await;pool.close().await;
}

#[tokio::test]
async fn unattached_recovery_owner_retries_its_original_cleanup_and_terminal_without_duplicate_facts() {
    for terminal_fault in [false,true] {
        let (journal,pool)=test_fixture().await;
        let trigger=if terminal_fault {
            "CREATE TRIGGER reject_unattached_exit BEFORE INSERT ON agent_events WHEN NEW.kind='turn/failed' BEGIN SELECT RAISE(FAIL,'fixture unattached terminal failure'); END"
        } else {
            "CREATE TRIGGER reject_unattached_exit BEFORE INSERT ON agent_events WHEN NEW.kind='runtime/progress-recorded' AND json_extract(NEW.inline_json,'$.event.event')='host_cleanup_proven' BEGIN SELECT RAISE(FAIL,'fixture unattached cleanup failure'); END"
        };
        sqlx::query(trigger).execute(&pool).await.unwrap();
        assert!(journal.fail_unattached_recovery("original attachment failed before resource opening").await.is_err());
        assert_eq!(journal.0.attachment.load(Ordering::Acquire),2);
        assert!(!journal.is_attached());
        sqlx::query("DROP TRIGGER reject_unattached_exit").execute(&pool).await.unwrap();
        let (first,second)=tokio::join!(
            journal.fail_unattached_recovery("original attachment failed before resource opening"),
            journal.fail_unattached_recovery("original attachment failed before resource opening"));
        assert_eq!(first.unwrap(),UnattachedRecoverySettlement::Settled);
        assert_eq!(second.unwrap(),UnattachedRecoverySettlement::Settled);
        assert_eq!(journal.0.store.read_turn_receipt(&journal.0.session,&journal.0.operation).await.unwrap().status,TurnReceiptStatus::Failed);
        let cleanup:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='host_cleanup_proven'")
            .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
        let exits:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='turn/failed'")
            .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
        let acknowledgements:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/native-settlement-settled'")
            .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
        assert_eq!((cleanup,exits,acknowledgements),(1,1,1));
        assert!(journal.0.store.pending_native_turn_settlements(0,500).await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn real_runtime_attachment_never_obtains_unopened_resource_cleanup_proof() {
    let (journal,_pool)=test_fixture().await;
    journal.attach_runtime(CancellationToken::new()).unwrap();
    assert!(EngineTurnJournal::cached_is_attached(&journal.0));
    let before=journal.0.store.current_cursor(&journal.0.session).await.unwrap();
    assert_eq!(journal.fail_unattached_recovery("attachment already succeeded").await.unwrap(),UnattachedRecoverySettlement::Attached);
    assert_eq!(journal.0.store.current_cursor(&journal.0.session).await.unwrap(),before);
    assert!(journal.0.store.read_native_turn_settlement(&owner(&journal),&journal.0.session,&journal.0.operation).await.unwrap().is_none());
}

#[tokio::test]
async fn unattached_recovery_settlement_preserves_user_pause_ack_loss_and_stop_priority() {
    for stop in [false,true] {
        let (journal,pool)=test_fixture().await;
        if stop {
            journal.0.store.cancel_active_turn(&journal.0.session,"stop-unattached-owner".into(),"session_api".into()).await.unwrap();
        } else {
            journal.0.store.request_native_pause(&owner(&journal),&journal.0.session,&journal.0.operation,"pause-unattached-owner","inspect before retry").await.unwrap();
            journal.lose_next_terminal_ack_for_test();
            assert!(journal.fail_unattached_recovery("original attach failure").await.is_err());
        }
        assert_eq!(journal.fail_unattached_recovery("original attach failure").await.unwrap(),UnattachedRecoverySettlement::Settled);
        let receipt=journal.0.store.read_turn_receipt(&journal.0.session,&journal.0.operation).await.unwrap();
        if stop {assert_eq!(receipt.status,TurnReceiptStatus::Cancelled);}
        else {
            assert_eq!(receipt.status,TurnReceiptStatus::Running);
            let pause=journal.0.store.native_pause_state(&journal.0.session,&journal.0.operation).await.unwrap().unwrap();
            assert_eq!(pause.reason,"EXECUTION_USER_REQUESTED");assert!(pause.cleanup_proven);
        }
        assert!(journal.0.store.pending_native_turn_settlements(0,500).await.unwrap().is_empty());
        let forbidden:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind IN ('turn/failed','turn/completed','turn/resume-authorized','context/model-visible-applied','tool/call-started')")
            .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
        assert_eq!(forbidden,0);
    }
}

#[tokio::test]
async fn unattached_recovery_retains_the_exact_failed_message_completion_before_acknowledging_cleanup() {
    use nomifun_agent_runtime::AgentTurnRecovery;
    let (mut journal,pool)=test_fixture().await;
    let mut checkpoint=checkpoint(&journal,false);checkpoint.segments=None;
    journal.append(serde_json::to_string(&AgentEngineEvent::TurnStarted {binding:checkpoint.binding.clone(),turn_operation_id:journal.0.operation.clone()}).unwrap(),None,EngineJournalWrite::Progress).await.unwrap();
    let saved=journal.save_execution_checkpoint(checkpoint,owner(&journal)).await.unwrap().unwrap();
    journal.append(serde_json::to_string(&AgentEngineEvent::ModelStepStarted {step:1,operation_id:"turn:model:1".into(),correction_source:None}).unwrap(),None,EngineJournalWrite::Progress).await.unwrap();
    journal.append(serde_json::to_string(&AgentEngineEvent::OutputTextDelta {step:1,text:"original unsealed answer".into(),source:None}).unwrap(),None,EngineJournalWrite::Progress).await.unwrap();
    let saved=journal.0.store.load_native_checkpoint(&owner(&journal),&journal.0.session,&journal.0.operation).await.unwrap().filter(|checkpoint|checkpoint.revision==saved.revision).unwrap();
    let facts=journal.0.store.native_recovery_facts(&journal.0.session,&journal.0.operation).await.unwrap();
    let mut prefix=Vec::new();let mut tail=Vec::new();
    for event in facts.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded") {
        let event_value=facts.event_payloads[event.event_id.as_ref()]["event"].clone();
        let native:AgentEngineEvent=serde_json::from_value(event_value).unwrap();
        if event.seq<=saved.through_seq {prefix.push(native);} else {tail.push(native);}
    }
    sqlx::query("UPDATE agent_turns SET execution_lease_until=0 WHERE session_id=? AND operation_id=?")
        .bind(journal.0.session.as_ref()).bind(journal.0.operation.as_ref()).execute(&pool).await.unwrap();
    let lease=journal.0.store.claim_native_execution(nomifun_agent_session::NativeExecutionClaim {
        owner:owner(&journal),agent_session_id:journal.0.session.clone(),operation_id:journal.0.operation.clone(),snapshot:journal.0.snapshot.clone(),
        active_set_generation:0,holder:"recovered-original-message-owner".into(),expected_fence:journal.0.lease.fence(),
        checkpoint:Some((saved.revision,saved.digest.clone(),saved.through_seq)),
    }).await.unwrap();
    let recovery=AgentTurnRecovery::new(serde_json::from_value(saved.state.0).unwrap(),saved.revision,lease.fence(),prefix,tail,vec![]).unwrap();
    assert_eq!(recovery.last_model_step(),1);
    let original=Arc::get_mut(&mut journal.0).expect("only the recovered fixture owns this exact journal");
    original.generation=i64::try_from(lease.generation()).unwrap();original.lease=lease;original.recovery=Some(Arc::new(recovery));
    // A freshly recovered writer has not opened a new assistant segment.
    original.cursor.get_mut().assistant_step=None;
    sqlx::query("CREATE TRIGGER reject_original_completion BEFORE INSERT ON agent_events WHEN NEW.kind='message/completed' BEGIN SELECT RAISE(FAIL,'fixture original completion failure'); END")
        .execute(&pool).await.unwrap();
    assert!(journal.restore_public_projection().await.is_err());
    let original=journal.0.projection_retry.lock().await.clone().expect("the original immutable completion remains owned");
    assert!(journal.fail_unattached_recovery("original public completion could not persist").await.is_err());
    assert!(journal.0.projection_retry.lock().await.is_some());
    assert!(journal.0.store.read_native_turn_settlement(&owner(&journal),&journal.0.session,&journal.0.operation).await.unwrap().is_none(),"cleanup handoff must not skip an original pending writeback");
    sqlx::query("DROP TRIGGER reject_original_completion").execute(&pool).await.unwrap();
    assert_eq!(journal.fail_unattached_recovery("original public completion could not persist").await.unwrap(),UnattachedRecoverySettlement::Settled);
    assert!(journal.0.projection_retry.lock().await.is_none());
    let facts=journal.0.store.chat_causality_facts(&journal.0.session,&journal.0.operation).await.unwrap();
    let completion=facts.events.iter().find(|event|event.event_id==original.event_id).unwrap();
    let SessionEventPayloadRef::InlineJson(expected)=original.semantic_event.payload else {panic!("fixture completion must be inline");};
    assert_eq!(facts.event_payloads[completion.event_id.as_ref()],expected.0);
    assert_eq!(facts.events.iter().filter(|event|event.event_id==original.event_id).count(),1);
}
