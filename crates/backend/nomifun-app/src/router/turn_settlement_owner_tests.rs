//! Production SDK + canonical Host coverage for a successful cleanup handoff.
use super::*;
use crate::router::nomi_core_session::NomiCoreSessionOwner;

#[tokio::test]
async fn successful_pause_keeps_original_cleanup_until_storage_recovers() {
    retained_cleanup_recovers(false, false).await;
}

#[tokio::test]
async fn failed_first_handoff_keeps_its_exact_broken_runtime_until_storage_recovers() {
    retained_cleanup_recovers(true, false).await;
}

#[tokio::test]
async fn retry_panic_keeps_the_original_session_owner_and_result() {
    retained_cleanup_recovers(false, true).await;
}

#[tokio::test]
async fn preclaim_database_failure_still_retains_the_original_runtime_without_a_durable_handoff() {
    let fixture=Fixture::new("settlement-owner-preclaim-recovers").await;
    let session:AgentSessionId=fixture.host.options.conversation_id.clone().into();
    sqlx::query("CREATE TRIGGER reject_original_claim BEFORE UPDATE OF execution_owner ON agent_turns WHEN NEW.execution_owner IS NOT NULL BEGIN SELECT RAISE(FAIL,'fixture original native claim storage failure'); END")
        .execute(fixture.pool()).await.unwrap();
    fixture.runtime.send_message(fixture.message.clone()).await.unwrap();
    let result=tokio::time::timeout(Duration::from_secs(8),fixture.runtime.cancel()).await.unwrap();
    assert!(result.is_err());
    assert!(fixture.host.active.lock().await.is_none(),"this failure occurs before any Journal was admitted");
    assert!(fixture.services.agent_runtime_sessions.get_runtime(session.as_ref()).is_none(),"the registry hides the broken transport");
    let claimed:i64=sqlx::query_scalar("SELECT execution_generation FROM agent_turns WHERE session_id=?")
        .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(claimed,0);
    sqlx::query("DROP TRIGGER reject_original_claim").execute(fixture.pool()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(12),async {
        while fixture.state().await!="cancelled" {tokio::time::sleep(Duration::from_millis(20)).await;}
    }).await.expect("the preclaim Session owner must finish without another input or model call");
    fixture.assert_settled().await;
    let claims:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/execution-claimed'")
        .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(claims,1,"only the eventual original cleanup admission can commit");
    fixture.finish().await;
}

#[tokio::test]
async fn failed_original_progress_prefix_remains_owned_until_initialization_and_durable_stop_can_settle() {
    for durable_stop in [false,true] {
        let fixture=Fixture::new(if durable_stop {"settlement-prefix-durable-stop"} else {"settlement-prefix-storage-recovery"}).await;
        fixture.host.admit_preparation(&fixture.message,CancellationToken::new()).await.unwrap();
        let journal=fixture.host.active.lock().await.as_ref().unwrap().journal.clone();
        sqlx::query("CREATE TRIGGER reject_original_prefix BEFORE INSERT ON agent_events WHEN NEW.kind='runtime/progress-recorded' AND json_extract(NEW.inline_json,'$.event.event')='turn_started' BEGIN SELECT RAISE(FAIL,'fixture original prefix storage failure'); END")
            .execute(fixture.pool()).await.unwrap();
        assert!(fixture.host.record_event(&fixture.message,&AgentEngineEvent::TurnStarted {
            binding:fixture.host.engine_binding.clone(),turn_operation_id:"cleanup-retry-turn".into(),
        }).await.is_err(),"retain the actual failed Progress write before entering Cleanup");
        let session:AgentSessionId=fixture.host.options.conversation_id.clone().into();
        let store=fixture.owner.canonical().store();
        let stop=if durable_stop {
            Some(store.cancel_active_turn(&session,"stop-before-prefix-recovery".into(),"session_api".into()).await.unwrap().1.record.unwrap().event_id)
        } else {None};
        fixture.runtime.send_message(fixture.message.clone()).await.unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(8),fixture.runtime.cancel()).await.unwrap().is_err());
        assert_eq!(journal.sequence(),0);
        assert!(fixture.host.active.lock().await.is_some());
        assert!(store.read_native_turn_settlement(&fixture.host.principal,&session,&"cleanup-retry-turn".into()).await.unwrap().is_none(),"the failed original prefix must not be stranded behind a new pause fence");
        sqlx::query("DROP TRIGGER reject_original_prefix").execute(fixture.pool()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(12),async {
            loop {
                if fixture.host.active.lock().await.is_none() {break;}
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.expect("original prefix and cleanup must finish without another input");
        fixture.assert_settled().await;
        if let Some(stop)=stop {assert_eq!(store.read_turn_receipt(&session,&"cleanup-retry-turn".into()).await.unwrap().terminal_event.unwrap().event_id,stop);}
        let prefix:Vec<String>=sqlx::query_scalar("SELECT json_extract(inline_json,'$.event.event') FROM agent_events WHERE session_id=? AND kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event') IN ('turn_started','turn_input_scope') ORDER BY seq")
            .bind(session.as_ref()).fetch_all(fixture.pool()).await.unwrap();
        assert_eq!(prefix,vec!["turn_started","turn_input_scope"]);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn committed_user_pause_ack_loss_retries_the_same_original_exit_without_latest_pause_authority() {
    use nomifun_ai_agent::engine_sdk::{EngineSettlementControl,EngineTurnOutcome,EngineTurnTerminal};
    let fixture=Fixture::new("settlement-owner-user-pause-ack-loss").await;
    fixture.host.admit_preparation(&fixture.message,CancellationToken::new()).await.unwrap();
    let journal=fixture.host.active.lock().await.as_ref().unwrap().journal.clone();
    let original=EngineTurnOutcome::failed("original run exit retained for diagnosis");
    sqlx::query("CREATE TRIGGER reject_original_cleanup BEFORE INSERT ON agent_events WHEN NEW.kind='runtime/progress-recorded' AND json_extract(NEW.inline_json,'$.event.event')='host_cleanup_proven' BEGIN SELECT RAISE(FAIL,'fixture cleanup proof storage failure'); END")
        .execute(fixture.pool()).await.unwrap();
    let failure=fixture.host.cleanup_turn(&fixture.message,&original.terminal).await.unwrap_err();
    fixture.host.handoff_turn_settlement(&fixture.message,&original,&failure.to_string()).await.unwrap();
    let session:AgentSessionId=fixture.host.options.conversation_id.clone().into();
    let operation="cleanup-retry-turn".into();
    let store=fixture.owner.canonical().store();
    let receipt=store.read_native_turn_settlement(&fixture.host.principal,&session,&operation).await.unwrap().unwrap();
    store.request_native_pause(&fixture.host.principal,&session,&operation,"ack-loss-user-pause","inspect before continuing").await.unwrap();
    sqlx::query("DROP TRIGGER reject_original_cleanup").execute(fixture.pool()).await.unwrap();
    fixture.host.cleanup_turn(&fixture.message,&original.terminal).await.unwrap();
    let paused=fixture.host.retained_turn_outcome(&fixture.message,&original,EngineSettlementControl::PreserveResult).await.unwrap();
    assert!(matches!(&paused.terminal,EngineTurnTerminal::Paused {reason} if reason=="EXECUTION_USER_REQUESTED"));
    let event=nomifun_ai_agent::unified_runtime::runtime_terminal(&paused);
    journal.lose_next_terminal_ack_for_test();
    assert!(fixture.host.record_event(&fixture.message,&event).await.is_err());
    assert!(fixture.host.active.lock().await.is_some(),"the old owner retains its unconfirmed acknowledgement");
    assert!(!store.native_turn_settlement_current(&receipt).await.unwrap(),"committing cleanup=true advanced the original pause fence");
    assert!(store.native_turn_settlement_user_pause_committed(&receipt).await.unwrap());
    let confirmed=fixture.host.retained_turn_outcome(&fixture.message,&original,EngineSettlementControl::PreserveResult).await.unwrap();
    assert!(matches!(&confirmed.terminal,EngineTurnTerminal::Paused {reason} if reason=="EXECUTION_USER_REQUESTED"));
    fixture.host.record_event(&fixture.message,&nomifun_ai_agent::unified_runtime::runtime_terminal(&confirmed)).await.unwrap();
    assert!(fixture.host.active.lock().await.is_none());
    let pause=store.native_pause_state(&session,&operation).await.unwrap().unwrap();
    assert!(pause.cleanup_proven);assert_eq!(pause.reason,"EXECUTION_USER_REQUESTED");
    assert!(store.pending_native_turn_settlements(0,500).await.unwrap().is_empty());
    let exits:Vec<String>=sqlx::query_scalar("SELECT kind FROM agent_events WHERE session_id=? AND kind IN ('turn/failed','turn/completed','turn/cancelled','turn/resume-authorized')")
        .bind(session.as_ref()).fetch_all(fixture.pool()).await.unwrap();
    assert!(exits.is_empty(),"an original user pause is neither a business terminal nor a resume grant");
    let acknowledgements:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/native-settlement-settled'")
        .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(acknowledgements,1);
    fixture.finish().await;
}

async fn retained_cleanup_recovers(block_handoff: bool, panic_retry: bool) {
    let fixture = Fixture::new(if block_handoff { "settlement-owner-handoff-recovers" }
        else { "settlement-owner-storage-recovers" }).await;
    let session: AgentSessionId = fixture.host.options.conversation_id.clone().into();
    sqlx::query("CREATE TRIGGER reject_original_cleanup BEFORE INSERT ON agent_events WHEN NEW.kind='runtime/progress-recorded' AND json_extract(NEW.inline_json,'$.event.event')='host_cleanup_proven' BEGIN SELECT RAISE(FAIL,'fixture cleanup proof storage failure'); END")
        .execute(fixture.pool()).await.unwrap();
    if block_handoff {
        sqlx::query("CREATE TRIGGER reject_original_handoff BEFORE INSERT ON agent_events WHEN NEW.kind='runtime/native-settlement-requested' BEGIN SELECT RAISE(FAIL,'fixture handoff storage failure'); END")
            .execute(fixture.pool()).await.unwrap();
    }
    fixture.runtime.send_message(fixture.message.clone()).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(8), fixture.runtime.cancel()).await.unwrap();
    assert!(result.is_err(), "unconfirmed cleanup must remain visible");
    let store = fixture.owner.canonical().store();
    let pause = store.native_pause_state(&session, &"cleanup-retry-turn".into()).await.unwrap();
    let request = store.read_native_turn_settlement(&PrincipalRef { principal_kind: "user".into(),
        principal_id: fixture.host.options.user_id.clone() }, &session, &"cleanup-retry-turn".into()).await.unwrap();
    if block_handoff {
        assert!(pause.is_none()); assert!(request.is_none());
    } else {
        assert!(!pause.unwrap().cleanup_proven);
        assert_eq!(request.unwrap().outcome().0["terminal"]["status"], "cancelled");
    }
    assert!(fixture.host.active.lock().await.is_some(), "a pause is not release of original host custody");
    if panic_retry { NomiCoreSessionOwner::inject_settlement_retry_panic(session.as_ref()); }
    sqlx::query("DROP TRIGGER reject_original_cleanup").execute(fixture.pool()).await.unwrap();
    if block_handoff { sqlx::query("DROP TRIGGER reject_original_handoff").execute(fixture.pool()).await.unwrap(); }
    tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            if fixture.state().await == "cancelled" { break; }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("the original Session owner must finish without another user input");
    fixture.assert_settled().await;
    let failures: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_events WHERE session_id=? AND kind='runtime/native-settlement-requested'")
        .bind(session.as_ref()).fetch_one(fixture.pool()).await.unwrap();
    assert_eq!(failures, 1, "the same handoff must remain idempotent");
    fixture.finish().await;
}
