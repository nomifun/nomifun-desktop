use super::*;
use nomifun_common::DelegationRecordKind as Kind;
use nomifun_db::{read_delegation_records, SettleAgentExecutionAttemptParams};
use serde_json::json;

struct Fixture { db:nomifun_db::Database, repo:SqliteAgentExecutionRepository, execution:String, roots:Vec<String>, steps:Vec<String> }

async fn source_session(db:&nomifun_db::Database,session:&str,owner:&str)->String {
    let exists:bool=nomifun_db::sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_sessions WHERE agent_session_id=?)")
        .bind(session).fetch_one(db.pool()).await.unwrap();
    if !exists {insert_canonical_session(db,session,owner,"live").await;}
    let root=nomifun_common::generate_id();
    for (seq,id,kind,cause,payload) in [
        (1,root.clone(),"message/user-accepted",None,json!({"content":"original request"})),
        (2,format!("{session}:started"),"turn/started",Some(root.clone()),json!({})),
        (3,format!("{session}:tool"),"tool/call-started",Some(format!("{session}:started")),json!({"operation_id":"delegate-operation","call_id":"delegate-call","capability_id":"agent.collaboration","action_id":"agent/delegate"})),
        (4,format!("{session}:effect"),"effect/started",Some(format!("{session}:tool")),json!({})),
    ] {
        nomifun_db::sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json) VALUES(?,?,?,'records-fixture',?,?,1,?,?,?)")
            .bind(session).bind(seq).bind(&id).bind(&id).bind(kind).bind(&id).bind(cause).bind(payload.to_string()).execute(db.pool()).await.unwrap();
    }
    nomifun_db::sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,source_message_id,state,started_event_id,accepted_at,started_at) VALUES(?,'source-turn','source-turn','source-turn',?,'running',?,1,2)")
        .bind(session).bind(&root).bind(format!("{session}:started")).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("INSERT INTO agent_effects(effect_id,session_id,turn_id,operation_id,owner_domain,capability_module,action_id,input_digest,strategy,state,started_event_id,created_at) VALUES(?,?,'source-turn','delegate-operation','wave5.product','agent.collaboration','agent/delegate',?,'managed_effect','pending',?,1)")
        .bind(format!("{session}:effect-id")).bind(session).bind("a".repeat(64)).bind(format!("{session}:effect")).execute(db.pool()).await.unwrap();
    nomifun_db::sqlx::query("INSERT INTO agent_session_heads(session_id,status,active_turn_id,active_set_generation,last_seq,unread_count) VALUES(?,'running','source-turn',0,4,0)")
        .bind(session).execute(db.pool()).await.unwrap();
    root
}

async fn fixture()->Fixture {
    let db=database().await;
    let root=source_session(&db,CANONICAL_LEAD_ID,OWNER_ID).await;
    let repo=SqliteAgentExecutionRepository::new(db.pool().clone());
    let participant_id=nomifun_common::generate_id();
    let mut params=execution_params();
    params.lead_conversation_id=Some(CANONICAL_LEAD_ID.into());
    params.initial_plan_input=json!({"mode":"automatic","source":{"agent_session_id":CANONICAL_LEAD_ID,"turn_operation_id":"source-turn","tool_operation_id":"delegate-operation","effect_id":format!("{CANONICAL_LEAD_ID}:effect-id")}}).to_string();
    let execution=repo.create_execution_with_participants(OWNER_ID,&params,&[participant(participant_id.clone())],&event(AgentExecutionEventKind::Created)).await.unwrap();
    let steps=vec![nomifun_common::generate_id(),nomifun_common::generate_id()];
    let mut planned=event(AgentExecutionEventKind::PlanChanged);planned.payload=json!({"change":"initial_plan"}).to_string();
    repo.reconcile_plan(OWNER_ID,&execution.execution_id,execution.version,&ReconcileAgentExecutionPlanParams {
        goal:None,adaptation_policy:None,decision_policy:None,delegation_policy:None,keep_step_ids:vec![],new_participants:vec![],retire_participant_ids:vec![],
        new_steps:steps.iter().enumerate().map(|(i,id)|step(id.clone(),Some(participant_id.clone()),&format!("Task {i}"))).collect(),new_dependencies:vec![],execution_status:AgentExecutionStatus::Running,
    },&planned).await.unwrap();
    Fixture{db,repo,execution:execution.execution_id,roots:vec![root],steps}
}

async fn begin(f:&Fixture,index:usize)->nomifun_db::AgentExecutionStepDetailRow {
    let step=f.repo.get_step_detail(OWNER_ID,&f.execution,&f.steps[index]).await.unwrap().unwrap();
    let created=f.repo.create_attempt(OWNER_ID,&f.execution,&f.steps[index],step.step.version,None,&CreateAgentExecutionAttemptParams {
        participant_id:step.step.assigned_participant_id,start_immediately:false,trigger_reason:"initial".into(),effective_config:"{}".into(),retry_after:None,runtime_state:None,
    },&event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    let attempt=created.current_attempt.unwrap().attempt;
    let child=nomifun_common::generate_id();insert_canonical_session(&f.db,&child,OWNER_ID,"live").await;
    let started=f.repo.start_attempt(OWNER_ID,&f.execution,&f.steps[index],created.step.version,&attempt.attempt_id,attempt.version,&child,AgentExecutionAttemptSessionKind::ChildAttempt,None, None,&event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    started
}

async fn finish(f:&Fixture,index:usize,payload:serde_json::Value)->String {
    let started=begin(f,index).await;
    let attempt=started.current_attempt.unwrap().attempt;
    let child:String=nomifun_db::sqlx::query_scalar("SELECT conversation_id FROM conversation_execution_links WHERE execution_id=? AND attempt_id=? AND relation='attempt' LIMIT 1")
        .bind(&f.execution).bind(&attempt.attempt_id).fetch_one(f.db.pool()).await.unwrap();
    let operation=format!("turn:user:{OWNER_ID}:{child}:{}:initial-turn",attempt.attempt_id);
    let receipt=format!("{child}:successful-terminal");
    nomifun_db::sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,1,?,'records-fixture',?,'turn/completed',1,?,'{}')")
        .bind(&child).bind(&receipt).bind(&receipt).bind(&operation).execute(f.db.pool()).await.unwrap();
    nomifun_db::sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,terminal_event_id,accepted_at,started_at,finished_at) VALUES(?,?,?,?,'completed',?,1,1,2)")
        .bind(&child).bind(&operation).bind(&operation).bind(&operation).bind(&receipt).execute(f.db.pool()).await.unwrap();
    let mut payload=payload;payload["canonical_operation_id"]=operation.into();payload["terminal_event_id"]=receipt.into();
    let mut terminal=event(AgentExecutionEventKind::AttemptChanged);terminal.payload=payload.to_string();
    f.repo.settle_attempt(OWNER_ID,&f.execution,&f.steps[index],started.step.version,&attempt.attempt_id,attempt.version,None,&SettleAgentExecutionAttemptParams {
        expected_terminal_turn:None,expected_active_session_turn:None,attempt_status:nomifun_common::ExecutionAttemptStatus::Completed,step_status:ExecutionStepStatus::Completed,execution_status:None,
        question:Some(None),error:Some(None),output_summary:Some(Some("verified result".into())),output_files:Some("[]".into()),tokens:Some(None),retry_after:Some(None),runtime_state:Some(None),started_at:None,finished_at:Some(Some(10)),loop_repeat_reset:None,
    },&terminal).await.unwrap();
    attempt.attempt_id
}

#[tokio::test]
async fn committed_records_survive_same_parent_head_and_freeze_pagination() {
    let f=fixture().await;
    let (initial,head,cursor)=read_delegation_records(f.db.pool(),OWNER_ID,CANONICAL_LEAD_ID,Some(&f.roots),0,None,None,1).await.unwrap();
    assert_eq!(head,4);assert_eq!(initial.len(),1);assert_eq!(initial[0].kind,Kind::Delegated);assert_eq!(initial[0].source.tool_call_id,"delegate-call");
    let cursor=cursor.unwrap();
    let one=finish(&f,0,json!({"attempt_status":"completed","step_status":"completed"})).await;
    let two=finish(&f,1,json!({"attempt_status":"completed","step_status":"completed"})).await;
    let (old,_,_)=read_delegation_records(f.db.pool(),OWNER_ID,CANONICAL_LEAD_ID,Some(&f.roots),0,None,Some(&cursor),100).await.unwrap();
    assert!(old.iter().all(|row|row.kind==Kind::Delegated),"a frozen older page cannot acquire later arrivals");
    let (all,head,_)=read_delegation_records(f.db.pool(),OWNER_ID,CANONICAL_LEAD_ID,Some(&f.roots),0,None,None,100).await.unwrap();
    assert_eq!(head,4);assert_eq!(all.len(),4);assert!(all.iter().all(|row|row.anchor.seq==4));
    assert_eq!(all.iter().filter(|row|row.kind==Kind::Completed).map(|row|row.attempt_id.clone().unwrap()).collect::<std::collections::BTreeSet<_>>(),[one,two].into_iter().collect());
    assert_eq!(all[0].kind,Kind::Completed,"newest page must expose completions before older starts");
    assert!(read_delegation_records(f.db.pool(),"foreign-owner",CANONICAL_LEAD_ID,None,0,None,None,100).await.is_err());
}

#[tokio::test]
async fn completion_stays_in_original_request_window_after_later_parent_turn() {
    let f=fixture().await;
    let later=nomifun_common::generate_id();
    nomifun_db::sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,5,?,'records-fixture',?,'message/user-accepted',1,?,'{}')")
        .bind(CANONICAL_LEAD_ID).bind(&later).bind(&later).bind(&later).execute(f.db.pool()).await.unwrap();
    nomifun_db::sqlx::query("UPDATE agent_session_heads SET last_seq=5 WHERE session_id=?").bind(CANONICAL_LEAD_ID).execute(f.db.pool()).await.unwrap();
    finish(&f,0,json!({"attempt_status":"completed","step_status":"completed"})).await;
    let (all,_,_)=read_delegation_records(f.db.pool(),OWNER_ID,CANONICAL_LEAD_ID,Some(&f.roots),0,Some(1),None,100).await.unwrap();
    let done=all.iter().find(|row|row.kind==Kind::Completed).unwrap();assert_eq!(done.anchor.seq,5);assert_eq!(done.source.source_message_id,f.roots[0]);
    let (new_turn,_,_)=read_delegation_records(f.db.pool(),OWNER_ID,CANONICAL_LEAD_ID,Some(&[later]),0,None,None,100).await.unwrap();assert!(new_turn.is_empty());
    let (old_window,_,_)=read_delegation_records(f.db.pool(),OWNER_ID,CANONICAL_LEAD_ID,None,0,Some(1),None,100).await.unwrap();assert_eq!(old_window.len(),3,"source-window paging must retain a completion anchored outside that window");
}

#[tokio::test]
async fn recovery_and_adoption_use_the_committed_settlement_not_later_snapshots() {
    let f=fixture().await;
    let running=begin(&f,0).await;
    let active=running.current_attempt.as_ref().unwrap();
    let child=active.conversation_id.as_ref().unwrap();
    let operation=format!("turn:user:{OWNER_ID}:{child}:{}:initial-turn",active.attempt.attempt_id);
    let terminal=format!("{child}:original-terminal");
    nomifun_db::sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,1,?,'records-fixture',?,'turn/completed',1,?,'{}')")
        .bind(child).bind(&terminal).bind(&terminal).bind(&operation).execute(f.db.pool()).await.unwrap();
    nomifun_db::sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,terminal_event_id,accepted_at,started_at,finished_at) VALUES(?,?,?,?,'completed',?,1,1,2)")
        .bind(child).bind(&operation).bind(&operation).bind(&operation).bind(&terminal).execute(f.db.pool()).await.unwrap();
    let execution=f.repo.get_execution(OWNER_ID,&f.execution).await.unwrap().unwrap();
    let lease=AgentExecutionLeaseToken::new("delegation-recovery-owner".into());
    let expires=nomifun_common::now_ms()+30_000;
    f.repo.try_acquire_lease(&f.execution,execution.version,lease.owner(),expires).await.unwrap().unwrap();
    f.repo.reconcile_recovered_attempt(OWNER_ID,&f.execution,&running.step.step_id,running.step.version,&active.attempt.attempt_id,active.attempt.version,&lease,
        Some(&RecoveredAgentExecutionAttemptOutput {attempt_id:active.attempt.attempt_id.clone(),conversation_id:child.clone(),canonical_operation_id:operation,
            terminal_event_id:terminal,user_input_request:None,disposition:nomifun_db::AgentExecutionAttemptDisposition::Delivered,text:Some("canonical result".into()),output_files:vec![],error:None,tokens:None}),
        &NewAgentExecutionEvent{step_id:Some(running.step.step_id.clone()),attempt_id:Some(active.attempt.attempt_id.clone()),payload:json!({"reconciliation":"initial_turn_receipt","reason":"process_restart"}).to_string(),..event(AgentExecutionEventKind::AttemptChanged)}).await.unwrap();
    f.repo.release_lease(&f.execution,lease.owner(),expires).await.unwrap().unwrap();
    finish(&f,1,json!({"attempt_status":"completed","step_status":"completed"})).await;
    let adopted=f.repo.get_step_detail(OWNER_ID,&f.execution,&f.steps[1]).await.unwrap().unwrap();
    let execution=f.repo.get_execution(OWNER_ID,&f.execution).await.unwrap().unwrap();
    let previous:serde_json::Value=serde_json::from_str(&f.repo.list_events(OWNER_ID,&f.execution,0,100).await.unwrap().last().unwrap().payload).unwrap();
    f.repo.adopt_step_output(OWNER_ID,&f.execution,execution.version,&adopted.step.step_id,adopted.step.version,&nomifun_db::AdoptAgentExecutionStepOutputParams {
        output_summary:"verified adopted result".into(),output_files:"[]".into(),tokens:None,runtime_state:None,
    },&NewAgentExecutionEvent {step_id:Some(adopted.step.step_id.clone()),payload:json!({"change":"output_adopted","canonical_operation_id":previous["canonical_operation_id"],"terminal_event_id":previous["terminal_event_id"]}).to_string(),..event(AgentExecutionEventKind::AttemptChanged)}).await.unwrap();
    // A subsequent retry reset cannot rewrite earlier committed completion.
    let detail=f.repo.get_execution_detail(OWNER_ID,&f.execution).await.unwrap().unwrap();
    let task=detail.steps.iter().find(|step|step.step_id==f.steps[0]).unwrap();
    f.repo.reset_steps_for_retry(OWNER_ID,&f.execution,detail.execution.version,&[nomifun_db::RetryAgentExecutionStep {
        step_id:task.step_id.clone(),expected_step_version:task.version,
    }],&NewAgentExecutionEvent{step_id:Some(task.step_id.clone()),payload:json!({"change":"retry_requested"}).to_string(),..event(AgentExecutionEventKind::StepChanged)}).await.unwrap();
    let (all,_,_)=read_delegation_records(f.db.pool(),OWNER_ID,CANONICAL_LEAD_ID,Some(&f.roots),0,None,None,100).await.unwrap();
    assert_eq!(all.iter().filter(|row|row.kind==Kind::Completed).count(),3);assert!(all.iter().all(|row|!row.source_scoped_order));
}

#[tokio::test]
async fn historical_records_remain_read_only_and_do_not_require_a_live_latest_link() {
    let f=fixture().await;
    finish(&f,0,json!({"attempt_status":"completed","step_status":"completed"})).await;
    nomifun_db::sqlx::query("UPDATE agent_execution_events SET payload=json_remove(payload,'$.delegation_presentation') WHERE execution_id=?")
        .bind(&f.execution).execute(f.db.pool()).await.unwrap();
    nomifun_db::sqlx::query("UPDATE conversation_execution_links SET active=0 WHERE execution_id=?")
        .bind(&f.execution).execute(f.db.pool()).await.unwrap();
    let count_before:i64=nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM agent_execution_events").fetch_one(f.db.pool()).await.unwrap();
    let (all,_,_)=read_delegation_records(f.db.pool(),OWNER_ID,CANONICAL_LEAD_ID,Some(&f.roots),0,None,None,100).await.unwrap();
    assert_eq!(all.len(),3);assert!(all.iter().all(|row|row.source_scoped_order));
    assert_eq!(all.iter().filter(|row|row.kind==Kind::Completed).count(),1);
    let count_after:i64=nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM agent_execution_events").fetch_one(f.db.pool()).await.unwrap();
    assert_eq!(count_before,count_after,"history presentation must never append an event or resume work");
}

#[tokio::test]
async fn nested_delegation_keeps_the_actual_child_tool_source() {
    let f=fixture().await;
    let running=begin(&f,0).await;
    let attempt=running.current_attempt.as_ref().unwrap();
    let child=attempt.conversation_id.as_ref().unwrap();
    let child_root=source_session(&f.db,child,OWNER_ID).await;
    let nested=nomifun_common::generate_id();
    let event=NewAgentExecutionEvent {
        actor:nomifun_common::AgentExecutionActor::agent(child,Some(attempt.attempt.attempt_id.clone())),
        payload:json!({"change":"delegated_steps_appended","delegation_source":{"agent_session_id":child,"turn_operation_id":"source-turn","tool_operation_id":"delegate-operation","effect_id":format!("{child}:effect-id")}}).to_string(),
        ..event(AgentExecutionEventKind::PlanChanged)
    };
    f.repo.append_steps_from_attempt(OWNER_ID,&f.execution,&nomifun_db::AppendAgentExecutionStepsFromAttemptParams {
        operation_id:"one-trusted-semantic-command".into(),caller_conversation_id:child.clone(),caller_step_id:running.step.step_id.clone(),caller_attempt_id:attempt.attempt.attempt_id.clone(),
        expected_caller_step_version:running.step.version,expected_caller_attempt_version:attempt.attempt.version,
        new_steps:vec![step(nested.clone(),running.step.assigned_participant_id.clone(),"Nested task")],new_dependencies:vec![],
    },&event).await.unwrap();
    let (nested_rows,_,_)=read_delegation_records(f.db.pool(),OWNER_ID,child,Some(&[child_root.clone()]),0,None,None,100).await.unwrap();
    assert_eq!(nested_rows.len(),1);assert_eq!(nested_rows[0].step_id,nested);assert_eq!(nested_rows[0].source.source_message_id,child_root);
    let (parent_rows,_,_)=read_delegation_records(f.db.pool(),OWNER_ID,CANONICAL_LEAD_ID,Some(&f.roots),0,None,None,100).await.unwrap();
    assert!(parent_rows.iter().all(|row|row.step_id!=nested),"a nested tool invocation must never borrow the root's unrelated source call");
}

#[tokio::test]
async fn keyset_pages_select_lifecycle_rows_before_limiting_a_long_control_history() {
    let f=fixture().await;
    finish(&f,0,json!({"attempt_status":"completed","step_status":"completed"})).await;
    let tail=f.repo.get_execution(OWNER_ID,&f.execution).await.unwrap().unwrap().event_sequence;
    // These large private control facts are not lifecycle records. The read
    // query filters them inside SQLite, rather than hydrating an entire event
    // history and discovering a completion only after an application-side cap.
    nomifun_db::sqlx::query("WITH RECURSIVE n(value) AS (VALUES(1) UNION ALL SELECT value+1 FROM n WHERE value<2048) INSERT INTO agent_execution_events(execution_id,sequence,event_type,actor_type,on_behalf_of_user_id,payload,created_at) SELECT ?,?+value,'step_changed','system',?,?,1 FROM n")
        .bind(&f.execution).bind(tail).bind(OWNER_ID).bind(json!({"change":"conversation_effect_delivered","effect":"steer","private_control_body":"x".repeat(4096)}).to_string())
        .execute(f.db.pool()).await.unwrap();
    nomifun_db::sqlx::query("UPDATE agent_executions SET event_sequence=event_sequence+2048 WHERE execution_id=?")
        .bind(&f.execution).execute(f.db.pool()).await.unwrap();
    let (newest,_,cursor)=read_delegation_records(f.db.pool(),OWNER_ID,CANONICAL_LEAD_ID,Some(&f.roots),0,None,None,1).await.unwrap();
    assert_eq!(newest.len(),1);assert_eq!(newest[0].kind,Kind::Completed);assert!(cursor.is_some());
    let (older,_,cursor)=read_delegation_records(f.db.pool(),OWNER_ID,CANONICAL_LEAD_ID,Some(&f.roots),0,None,cursor.as_deref(),1).await.unwrap();
    assert_eq!(older.len(),1);assert_eq!(older[0].kind,Kind::Delegated);
    let (oldest,_,cursor)=read_delegation_records(f.db.pool(),OWNER_ID,CANONICAL_LEAD_ID,Some(&f.roots),0,None,cursor.as_deref(),1).await.unwrap();
    assert_eq!(oldest.len(),1);assert_eq!(oldest[0].kind,Kind::Delegated);assert!(cursor.is_none());
}
