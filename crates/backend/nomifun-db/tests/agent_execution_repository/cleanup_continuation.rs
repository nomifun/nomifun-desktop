//! Canonical/domain identity fixtures only: no model or native task is run.
use super::*;
use nomifun_db::{ConversationCleanupTurnTarget,SettleAgentExecutionAttemptParams,AgentExecutionTerminalTurnGuard};

struct ClosedContinuation {
    db:nomifun_db::Database,
    repository:SqliteAgentExecutionRepository,
    execution:String,
    step:String,
    attempt:String,
    session:String,
    operation:String,
    terminal:String,
    initial_terminal:String,
    source_sequence:i64,
}

fn waiting_params(source:AgentExecutionTerminalTurnGuard,question:String)->SettleAgentExecutionAttemptParams {
    SettleAgentExecutionAttemptParams {expected_active_session_turn:None,expected_terminal_turn:Some(source),
        attempt_status:nomifun_common::ExecutionAttemptStatus::WaitingInput,step_status:ExecutionStepStatus::WaitingInput,
        execution_status:Some(AgentExecutionStatus::WaitingInput),question:Some(Some(question)),error:Some(None),output_summary:None,
        output_files:None,tokens:None,retry_after:Some(None),runtime_state:Some(None),started_at:None,finished_at:None,loop_repeat_reset:None}
}

async fn fixture(cancelled:bool)->ClosedContinuation {
    let db=database().await;let repository=SqliteAgentExecutionRepository::new(db.pool().clone());
    let(execution,running,initial)=user_input_fixture(&db,&repository,"automatic").await;
    let question=initial.user_input_request.clone().unwrap();
    let waiting=repository.settle_attempt(OWNER_ID,&execution,&running.step.step_id,running.step.version,&initial.attempt_id,
        running.current_attempt.as_ref().unwrap().attempt.version,None,&waiting_params(AgentExecutionTerminalTurnGuard::InputRequired {
            conversation_id:initial.conversation_id.clone(),canonical_operation_id:initial.canonical_operation_id.clone(),
            terminal_event_id:initial.terminal_event_id.clone(),user_input_request:question.clone()},question.question.clone()),
        &NewAgentExecutionEvent {payload:serde_json::json!({"canonical_operation_id":initial.canonical_operation_id,
            "terminal_event_id":initial.terminal_event_id,"question":question.question,"user_input_request":question}).to_string(),
            ..event(AgentExecutionEventKind::DecisionRequested)}).await.unwrap();
    let continuation=nomifun_common::generate_id();let mut effects=nomifun_db::AttemptConversationEffects::default();
    effects.push_decision(continuation.clone(),"the accepted first answer".into()).unwrap();
    let current=repository.get_execution(OWNER_ID,&execution).await.unwrap().unwrap();let old=waiting.current_attempt.as_ref().unwrap();
    let resumed=repository.resume_waiting_attempt(OWNER_ID,&execution,current.version,&waiting.step.step_id,waiting.step.version,&initial.attempt_id,old.attempt.version,
        &nomifun_db::AttemptConversationEffectParams {runtime_state:Some(effects.encode().unwrap())},&event(AgentExecutionEventKind::DecisionAnswered)).await.unwrap();
    let operation=format!("turn:user:{OWNER_ID}:{}:{continuation}",initial.conversation_id);
    let started=nomifun_common::generate_id();let terminal=nomifun_common::generate_id();
    let second=nomifun_agent_contracts::AgentUserInputRequest {request_id:"second-original-question".into(),question:"Which original class?".into(),remaining_work:"Use the answer in this same task.".into()};
    let terminal_body=if cancelled {serde_json::json!({})}else {serde_json::json!({"user_input_request":second})};
    for (seq,id,kind,value) in [(3,&started,"turn/started",serde_json::json!({})),
        (4,&terminal,if cancelled {"turn/cancelled"}else {"turn/completed"},terminal_body)] {
        nomifun_db::sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,?,?,'continuation-fixture',?,?,1,?,?)")
            .bind(&initial.conversation_id).bind(seq).bind(id).bind(id).bind(kind).bind(&operation).bind(value.to_string()).execute(db.pool()).await.unwrap();
    }
    nomifun_db::sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,started_event_id,terminal_event_id,accepted_at,started_at,finished_at) VALUES(?,?,?,?,?,?,?,3,3,4)")
        .bind(&initial.conversation_id).bind(&operation).bind(&operation).bind(&operation).bind(if cancelled {"cancelled"}else {"completed"})
        .bind(started).bind(&terminal).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("UPDATE agent_session_heads SET last_seq=4 WHERE session_id=?").bind(&initial.conversation_id).execute(db.pool()).await.unwrap();
    let params=if cancelled {SettleAgentExecutionAttemptParams {expected_active_session_turn:None,
        expected_terminal_turn:Some(AgentExecutionTerminalTurnGuard::Cancelled {conversation_id:initial.conversation_id.clone(),canonical_operation_id:operation.clone(),terminal_event_id:terminal.clone()}),
        attempt_status:nomifun_common::ExecutionAttemptStatus::Cancelled,step_status:ExecutionStepStatus::Cancelled,execution_status:None,
        question:Some(None),error:Some(None),output_summary:None,output_files:None,tokens:None,retry_after:Some(None),runtime_state:Some(None),
        started_at:None,finished_at:Some(Some(nomifun_common::now_ms())),loop_repeat_reset:None}}
        else {waiting_params(AgentExecutionTerminalTurnGuard::InputRequired {conversation_id:initial.conversation_id.clone(),canonical_operation_id:operation.clone(),
            terminal_event_id:terminal.clone(),user_input_request:second.clone()},second.question.clone())};
    let payload=if cancelled {serde_json::json!({"attempt_status":"cancelled","step_status":"cancelled","canonical_operation_id":operation,"terminal_event_id":terminal})}
        else {serde_json::json!({"canonical_operation_id":operation,"terminal_event_id":terminal,"question":second.question,"user_input_request":second})};
    let settled=repository.settle_attempt(OWNER_ID,&execution,&resumed.detail.step.step_id,resumed.detail.step.version,&initial.attempt_id,
        resumed.detail.current_attempt.as_ref().unwrap().attempt.version,None,&params,&NewAgentExecutionEvent {payload:payload.to_string(),
            ..event(if cancelled {AgentExecutionEventKind::AttemptChanged}else {AgentExecutionEventKind::DecisionRequested})}).await.unwrap();
    assert!(settled.current_attempt.as_ref().unwrap().attempt.runtime_state.is_none());
    let source_sequence=repository.get_execution(OWNER_ID,&execution).await.unwrap().unwrap().event_sequence;
    if !cancelled {
        let current=repository.get_execution(OWNER_ID,&execution).await.unwrap().unwrap();
        repository.cancel_step(OWNER_ID,&execution,current.version,&settled.step.step_id,settled.step.version,
            nomifun_common::AgentExecutionCancellationScope::Task,&NewAgentExecutionEvent {actor:nomifun_common::AgentExecutionActor::user(OWNER_ID),
                ..event(AgentExecutionEventKind::StepChanged)}).await.unwrap();
    }
    ClosedContinuation {db,repository,execution,step:settled.step.step_id,attempt:initial.attempt_id,session:initial.conversation_id,
        operation,terminal,initial_terminal:initial.terminal_event_id,source_sequence}
}

#[tokio::test]
async fn cleared_question_and_cancelled_settlements_keep_the_exact_original_continuation_target() {
    for cancelled in [false,true] {
        let f=fixture(cancelled).await;
        let before:(i64,i64)=nomifun_db::sqlx::query_as("SELECT event_sequence,(SELECT COUNT(*) FROM agent_turns WHERE session_id=?) FROM agent_executions WHERE execution_id=?")
            .bind(&f.session).bind(&f.execution).fetch_one(f.db.pool()).await.unwrap();
        let page=f.repository.list_pending_conversation_cleanups(Some(&f.execution),None,100).await.unwrap();assert_eq!(page.cleanups.len(),1);
        assert_eq!(page.cleanups[0].target,ConversationCleanupTurnTarget::Exact {operation_id:f.operation.clone()});
        assert_ne!(f.operation,format!("turn:user:{OWNER_ID}:{}:{}:initial-turn",f.session,f.attempt));
        assert!(f.repository.validate_conversation_cleanup(&page.cleanups[0]).await.unwrap());
        let facts=f.repository.read_task_retirement_facts(OWNER_ID,&f.session,&f.execution,&f.step,&f.attempt).await.unwrap().unwrap();
        assert_eq!(facts.target,page.cleanups[0].target);assert!(facts.original_attempt.attempt.runtime_state.is_none());
        let original:(String,String)=nomifun_db::sqlx::query_as("SELECT state,terminal_event_id FROM agent_turns WHERE session_id=? AND operation_id=?")
            .bind(&f.session).bind(&f.operation).fetch_one(f.db.pool()).await.unwrap();
        assert_eq!(original,(if cancelled {"cancelled".into()}else {"completed".into()},f.terminal));
        let after:(i64,i64)=nomifun_db::sqlx::query_as("SELECT event_sequence,(SELECT COUNT(*) FROM agent_turns WHERE session_id=?) FROM agent_executions WHERE execution_id=?")
            .bind(&f.session).bind(&f.execution).fetch_one(f.db.pool()).await.unwrap();assert_eq!(before,after);
        assert!(f.repository.resolve_conversation_link(OWNER_ID,&f.session).await.unwrap().iter().all(|link|link.cleanup_completed_at.is_none()));
    }
}

#[tokio::test]
async fn foreign_malformed_or_mismatched_latest_cleanup_source_stays_unconfirmed_instead_of_guessing_initial() {
    for changed in ["actor","canonical_session","terminal","disagreeing_key","reference_type","missing_terminal","malformed","unsupported_terminal"] {
        let f=fixture(false).await;
        match changed {
            "actor"=>{nomifun_db::sqlx::query("UPDATE agent_execution_events SET actor_type='user',actor_id=? WHERE execution_id=? AND sequence=?")
                .bind(OWNER_ID).bind(&f.execution).bind(f.source_sequence).execute(f.db.pool()).await.unwrap();},
            "canonical_session"=>{nomifun_db::sqlx::query("UPDATE agent_execution_events SET payload=json_set(payload,'$.canonical_operation_id',?) WHERE execution_id=? AND sequence=?")
                .bind(format!("turn:user:{OWNER_ID}:{FOREIGN_CANONICAL_LEAD_ID}:foreign-continuation")).bind(&f.execution).bind(f.source_sequence).execute(f.db.pool()).await.unwrap();},
            "terminal"=>{nomifun_db::sqlx::query("UPDATE agent_execution_events SET payload=json_set(payload,'$.terminal_event_id',?) WHERE execution_id=? AND sequence=?")
                .bind(&f.initial_terminal).bind(&f.execution).bind(f.source_sequence).execute(f.db.pool()).await.unwrap();},
            "disagreeing_key"=>{nomifun_db::sqlx::query("UPDATE agent_execution_events SET payload=json_set(payload,'$.turn_operation_key','another-invocation') WHERE execution_id=? AND sequence=?")
                .bind(&f.execution).bind(f.source_sequence).execute(f.db.pool()).await.unwrap();},
            "reference_type"=>{nomifun_db::sqlx::query("UPDATE agent_execution_events SET payload=json_set(payload,'$.canonical_operation_id',17) WHERE execution_id=? AND sequence=?")
                .bind(&f.execution).bind(f.source_sequence).execute(f.db.pool()).await.unwrap();},
            "missing_terminal"=>{nomifun_db::sqlx::query("UPDATE agent_execution_events SET payload=json_remove(payload,'$.terminal_event_id') WHERE execution_id=? AND sequence=?")
                .bind(&f.execution).bind(f.source_sequence).execute(f.db.pool()).await.unwrap();},
            "malformed"=>{nomifun_db::sqlx::query("UPDATE agent_execution_events SET payload='[]' WHERE execution_id=? AND sequence=?")
                .bind(&f.execution).bind(f.source_sequence).execute(f.db.pool()).await.unwrap();},
            _=>{nomifun_db::sqlx::query("UPDATE agent_events SET kind_version=9 WHERE event_id=?").bind(&f.terminal).execute(f.db.pool()).await.unwrap();},
        }
        let page=f.repository.list_pending_conversation_cleanups(Some(&f.execution),None,100).await.unwrap();assert_eq!(page.cleanups.len(),1);
        assert!(matches!(page.cleanups[0].target,ConversationCleanupTurnTarget::Unconfirmed {..}),"{changed}: {:?}",page.cleanups[0].target);
        assert!(f.repository.validate_conversation_cleanup(&page.cleanups[0]).await.is_err(),"{changed}");
        assert!(f.repository.resolve_conversation_link(OWNER_ID,&f.session).await.unwrap().iter().all(|link|link.cleanup_completed_at.is_none()));
    }
}
