use super::*;
use nomifun_ai_agent::engine_sdk::EngineTurnOutcome;
use nomifun_agent_runtime::{AgentToolResult, AgentTurnFailure};
use nomifun_agent_runtime::{AgentExecutionCheckpoint, AgentExecutionSegmentState, AgentExecutionStopReason, AgentSegmentPolicy, EngineBinding};

fn failed_outcome() -> StrictJsonValue {
    StrictJsonValue(serde_json::to_value(EngineTurnOutcome::failed("original execution stopped",
        AgentTurnFailure::Runtime { stage: nomifun_agent_contracts::AgentFailureStage::Recovery })).unwrap())
}

fn dispatch(index: usize) -> Value {
    json!({"event":"host_tool_dispatch","dispatch":{
        "operation_id":format!("original-owner-{index}"),"call_id":format!("original-call-{index}"),
        "capability_id":"workspace.files","action_id":"workspace.files/read","model_name":"read_file"
    }})
}

fn settlement(index: usize, body: &str) -> Value {
    json!({"event":"host_tool_settled","operation_id":format!("original-owner-{index}"),
        "call_id":format!("original-call-{index}"),
        "result":AgentToolResult::text(format!("original-call-{index}").into(),body,false),"error":null})
}

#[tokio::test]
async fn original_owner_returns_cross_the_soft_window_and_finish_after_stop() {
    let (journal,pool) = test_fixture().await;
    for index in 0..10 {
        journal.append(dispatch(index).to_string(),None,EngineJournalWrite::Progress).await.unwrap();
    }
    journal.0.store.cancel_exact_turn(&journal.0.session,&journal.0.operation,
        "stop-original-batch".into(),"session_api".into()).await.unwrap();
    journal.handoff_failed_cleanup(failed_outcome(),"the original batch still owes its results").await.unwrap();
    let body = "original returned bytes\n".repeat(40_000);
    assert!(body.len() < nomifun_agent_session::MAX_SINGLE_PAYLOAD_BYTES);
    for index in 0..10 {
        journal.append_owner_settlement(settlement(index,&body).to_string()).await.unwrap();
    }
    {
        let cursor = journal.0.cursor.lock().await;
        assert!(cursor.bytes > 8*1024*1024,"settlement crosses the old fixed cleanup window");
        assert!((cursor.total_bytes as u64) < cursor.budget.journal_bytes);
        assert!(!cursor.uncertain);
        assert!(cursor.pending_write.is_none());
    }
    let before = journal.sequence();
    assert!(journal.append(json!({"event":"model_step_started","step":1,"operation_id":"forbidden-model"}).to_string(),
        None,EngineJournalWrite::Progress).await.is_err());
    assert!(journal.append(dispatch(10).to_string(),None,EngineJournalWrite::Progress).await.is_err());
    assert!(journal.append_owner_settlement(dispatch(10).to_string()).await.is_err());
    assert_eq!(journal.sequence(),before,"settlement space cannot authorize new model or tool work");

    let facts = journal.0.store.chat_causality_facts(&journal.0.session,&journal.0.operation).await.unwrap();
    let returns = facts.events.iter().filter_map(|event|facts.event_payloads.get(event.event_id.as_ref()))
        .filter_map(|payload|payload.get("event"))
        .filter(|event|event["event"]=="host_tool_settled").collect::<Vec<_>>();
    assert_eq!(returns.len(),10);
    for (index,recorded) in returns.iter().enumerate() {
        assert_eq!(**recorded,settlement(index,&body),"every exact pre-stop owner return remains intact");
    }
    journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await.unwrap();
    journal.append(serde_json::to_string(&AgentEngineEvent::TurnCancelled {model_steps:0}).unwrap(),
        None,EngineJournalWrite::Terminal).await.unwrap();
    assert!(journal.0.store.pending_native_turn_settlements(0,500).await.unwrap().is_empty());
    assert_eq!(journal.0.store.read_turn_receipt(&journal.0.session,&journal.0.operation).await.unwrap().status,
        TurnReceiptStatus::Cancelled);
    pool.close().await;
}

#[tokio::test]
async fn original_settlement_remains_bounded_by_the_unchanged_cumulative_allowance() {
    let (journal,pool) = test_fixture().await;
    for index in 0..23 {
        journal.append(dispatch(index).to_string(),None,EngineJournalWrite::Progress).await.unwrap();
    }
    journal.handoff_failed_cleanup(failed_outcome(),"original batch result settlement").await.unwrap();
    // Repeated source bytes share one Payload, while every original event still
    // consumes its full logical journal allowance.
    let body = "x".repeat(900*1024);
    for index in 0..22 {
        journal.append_owner_settlement(settlement(index,&body).to_string()).await.unwrap();
    }
    let before = journal.sequence();
    let exceeded = journal.append_owner_settlement(settlement(22,&body).to_string()).await.unwrap_err();
    assert!(exceeded.to_string().contains("bounded evidence journal exhausted"));
    assert_eq!(journal.sequence(),before);
    {
        let cursor = journal.0.cursor.lock().await;
        assert_eq!(cursor.budget.journal_bytes,16*1024*1024);
        assert!((cursor.total_bytes as u64) <= cursor.budget.journal_bytes+4*1024*1024);
        assert!(!cursor.uncertain,"a known exhausted allowance is not an unknown database write");
        assert!(cursor.pending_write.is_none());
    }
    let unique_bodies: i64 = nomifun_db::sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_payloads WHERE session_id=? AND media_type='application/vnd.nomifun.tool-output+json'")
        .bind(journal.0.session.as_ref()).fetch_one(&pool).await.unwrap();
    assert_eq!(unique_bodies,1,"physical deduplication cannot discount logical journal accounting");
    pool.close().await;
}

#[tokio::test]
async fn pending_return_capacity_defers_new_calls_and_short_receipts_release_the_difference() {
    let (journal,pool) = test_fixture().await;
    let mut admitted = Vec::new();
    for index in 0..64 {
        let call: ToolCallId = format!("original-call-{index}").into();
        if !journal.reserve_tool_return_capacity(&call).await.unwrap() { break; }
        journal.append(dispatch(index).to_string(),None,EngineJournalWrite::Progress).await.unwrap();
        admitted.push(index);
    }
    assert!(!admitted.is_empty() && admitted.len() < 64);
    assert_eq!(journal.execution_pressure().await.unwrap().stop,Some(AgentExecutionStopReason::TurnJournalBudget));
    journal.refresh_budget().await.unwrap();
    assert_eq!(journal.execution_pressure().await.unwrap().stop,Some(AgentExecutionStopReason::TurnJournalBudget),
        "rereading unchanged capacity cannot erase a real outstanding obligation");
    for index in admitted {
        let before = journal.0.cursor.lock().await.reserved_return_bytes();
        journal.append_owner_settlement(settlement(index,"exact short owner return").to_string()).await.unwrap();
        assert_eq!(journal.0.cursor.lock().await.reserved_return_bytes(),before-TOOL_RETURN_CAPACITY_BYTES);
        let result = AgentToolResult::text(format!("original-call-{index}").into(),"exact short owner return",false);
        journal.append(serde_json::to_string(&AgentEngineEvent::ToolCompleted {step:1,result,runtime_hold:None}).unwrap(),
            None,EngineJournalWrite::Settlement).await.unwrap();
        assert_eq!(journal.0.cursor.lock().await.reserved_return_bytes(),before-2*TOOL_RETURN_CAPACITY_BYTES);
    }
    assert!(journal.0.cursor.lock().await.tool_return_capacity.is_empty());
    assert_eq!(journal.execution_pressure().await.unwrap().stop,None,
        "actual completed short returns release capacity without requiring a budget increase");
    assert!(journal.reserve_tool_return_capacity(&"next-uninvoked-call".into()).await.unwrap());
    pool.close().await;
}

#[tokio::test]
async fn failed_return_write_retains_capacity_until_its_original_acknowledgement() {
    let (journal,pool) = test_fixture().await;
    assert!(journal.reserve_tool_return_capacity(&"original-call-0".into()).await.unwrap());
    journal.append(dispatch(0).to_string(),None,EngineJournalWrite::Progress).await.unwrap();
    nomifun_db::sqlx::query("CREATE TRIGGER reject_capacity_return BEFORE INSERT ON agent_events WHEN NEW.kind='runtime/progress-recorded' AND json_extract(NEW.inline_json,'$.event.event')='host_tool_settled' BEGIN SELECT RAISE(FAIL,'fixture owner return unconfirmed'); END")
        .execute(&pool).await.unwrap();
    let original = settlement(0,"exact retained owner result").to_string();
    assert!(journal.append_owner_settlement(original.clone()).await.is_err());
    assert_eq!(journal.0.cursor.lock().await.reserved_return_bytes(),2*TOOL_RETURN_CAPACITY_BYTES);
    assert!(journal.reserve_tool_return_capacity(&"unrelated-call".into()).await.is_err());
    nomifun_db::sqlx::query("DROP TRIGGER reject_capacity_return").execute(&pool).await.unwrap();
    journal.append_owner_settlement(original.clone()).await.unwrap();
    assert_eq!(journal.0.cursor.lock().await.reserved_return_bytes(),TOOL_RETURN_CAPACITY_BYTES);
    journal.append_owner_settlement(original).await.unwrap();
    assert_eq!(journal.0.cursor.lock().await.reserved_return_bytes(),TOOL_RETURN_CAPACITY_BYTES,
        "duplicate owner ACK cannot release the Runtime's separate observation capacity");
    pool.close().await;
}

fn checkpoint(journal: &EngineTurnJournal, segment: u16) -> AgentExecutionCheckpoint {
    let steps = u16::from(segment > 1);
    AgentExecutionCheckpoint {
        version:1, binding:EngineBinding::new(journal.0.session.clone(),"native-binding".into(),"test".into(),
            "a".repeat(64).into(),journal.0.snapshot.clone()).unwrap(),
        turn_operation_id:journal.0.operation.clone(),active_set_generation:0,model_steps:steps,
        tool_call_count:if steps==0 {0} else {3},accepted_input_count:1,applied_steering_receipts:vec![],
        plan:Default::default(),work:Default::default(),patch_recovery:Default::default(),
        control_rejections:Default::default(),pending_user_input:None,awaiting_children:None,
        collaboration:Default::default(),completion_review:Default::default(),
        segments:Some(AgentExecutionSegmentState {policy:AgentSegmentPolicy {max_segments:2,max_no_progress_segments:2},
            model_steps_per_segment:1,segment,segment_start_step:steps,no_progress_segments:0,
            progress_at_segment_start:0,progress_fingerprints:vec![]}),
    }
}

#[tokio::test]
async fn admitted_large_results_complete_before_the_quiescent_window_renews() {
    let (journal,pool) = test_fixture().await;
    let owner = nomifun_agent_contracts::PrincipalRef {principal_kind:"user".into(),principal_id:journal.0.user.clone()};
    journal.append(serde_json::to_string(&AgentEngineEvent::TurnStarted {binding:checkpoint(&journal,1).binding,
        turn_operation_id:journal.0.operation.clone()}).unwrap(),None,EngineJournalWrite::Progress).await.unwrap();
    journal.save_execution_checkpoint(checkpoint(&journal,1),owner.clone()).await.unwrap().unwrap();
    journal.append(serde_json::to_string(&AgentEngineEvent::ModelStepStarted {step:1,operation_id:"turn:model:1".into(),
        correction_source:None}).unwrap(),None,EngineJournalWrite::Progress).await.unwrap();
    for index in 0..3 {
        let call = nomifun_chat_model_broker::ChatToolCall {call_id:format!("original-call-{index}").into(),name:"read_file".into(),
            arguments:StrictJsonValue(json!({"path":format!("file-{index}.txt")})),provider_metadata:None};
        journal.append(serde_json::to_string(&AgentEngineEvent::ToolCallCompleted {step:1,call:call.clone()}).unwrap(),
            None,EngineJournalWrite::Progress).await.unwrap();
        assert!(journal.reserve_tool_return_capacity(&call.call_id).await.unwrap());
        journal.append(serde_json::to_string(&AgentEngineEvent::ToolStarted {step:1,call_id:call.call_id,
            capability_id:"workspace.files".into(),action_id:"workspace.files/read".into()}).unwrap(),
            None,EngineJournalWrite::Progress).await.unwrap();
        journal.append(dispatch(index).to_string(),None,EngineJournalWrite::Progress).await.unwrap();
    }
    assert!(journal.save_execution_checkpoint(checkpoint(&journal,2),owner.clone()).await.unwrap().is_none());
    let body = "x".repeat(900*1024);
    for index in 0..3 {
        journal.append_owner_settlement(settlement(index,&body).to_string()).await.unwrap();
        journal.append(serde_json::to_string(&AgentEngineEvent::ToolCompleted {step:1,runtime_hold:None,
            result:AgentToolResult::text(format!("original-call-{index}").into(),&body,false)}).unwrap(),
            None,EngineJournalWrite::Settlement).await.unwrap();
    }
    journal.append(serde_json::to_string(&AgentEngineEvent::ToolResultsOrdered {step:1,
        call_ids:(0..3).map(|index|format!("original-call-{index}").into()).collect()}).unwrap(),
        None,EngineJournalWrite::Settlement).await.unwrap();
    assert!(journal.0.cursor.lock().await.bytes > nomifun_agent_contracts::MAX_NATIVE_JOURNAL_WINDOW_BYTES);
    assert!(journal.execution_pressure().await.unwrap().renew_window);
    journal.save_execution_checkpoint(checkpoint(&journal,2),owner).await.unwrap().unwrap();
    assert_eq!(journal.0.cursor.lock().await.bytes,0);
    journal.append(serde_json::to_string(&AgentEngineEvent::ModelStepStarted {step:2,operation_id:"turn:model:2".into(),
        correction_source:None}).unwrap(),None,EngineJournalWrite::Progress).await.unwrap();
    assert!(!journal.0.cursor.lock().await.uncertain);
    pool.close().await;
}

#[test]
fn session_payload_capacity_has_its_own_admission_boundary() {
    let mut cursor = Cursor::default();
    cursor.budget.journal_bytes = 32*1024*1024;
    let payload_bytes = cursor.budget.session_payload_bytes-CONTROL_SETTLEMENT_RESERVE_BYTES-512*1024;
    assert_eq!(cursor.next_tool_capacity_stop(payload_bytes),Some(AgentExecutionStopReason::SessionPayloadBudget));
    assert_eq!(cursor.next_tool_capacity_stop(0),None);
}
