//! Domain-only read fixtures; these never manufacture native cleanup proof.
use super::*;
use super::scoped_stop::Fixture;
use nomifun_common::{AgentExecutionActor,AgentExecutionCancellationScope};
use nomifun_db::{AgentExecutionTaskRetirementFacts,ConversationCleanupTurnTarget,PendingConversationEffect,AttemptConversationEffects};

async fn read(f:&Fixture,task:&nomifun_db::AgentExecutionStepDetailRow)->Option<AgentExecutionTaskRetirementFacts> {
    let attempt=task.current_attempt.as_ref().unwrap();
    f.repository.read_task_retirement_facts(&f.owner,attempt.conversation_id.as_deref().unwrap(),&f.execution_id,
        &task.step.step_id,&attempt.attempt.attempt_id).await.unwrap()
}

async fn add_source(f:&Fixture,task:&nomifun_db::AgentExecutionStepDetailRow,key:&str) {
    let detail=f.detail().await;
    f.repository.append_event(&f.owner,&f.execution_id,detail.execution.version,&NewAgentExecutionEvent {
        step_id:Some(task.step.step_id.clone()),attempt_id:Some(task.current_attempt.as_ref().unwrap().attempt.attempt_id.clone()),
        payload:serde_json::json!({"attempt_status":"completed","step_status":"completed","turn_operation_key":key}).to_string(),
        ..event(AgentExecutionEventKind::AttemptChanged)}).await.unwrap();
}

async fn start_on(f:&Fixture,title:&str,session:&str)->nomifun_db::AgentExecutionStepDetailRow {
    let task=f.task(title).await;
    let queued=f.repository.create_attempt(&f.owner,&f.execution_id,&task.step.step_id,task.step.version,None,
        &CreateAgentExecutionAttemptParams {participant_id:task.step.assigned_participant_id.clone(),start_immediately:false,trigger_reason:"same-session-owner".into(),
            effective_config:"{}".into(),retry_after:None,runtime_state:None},&event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    let attempt=&queued.current_attempt.as_ref().unwrap().attempt;
    f.repository.start_attempt(&f.owner,&f.execution_id,&task.step.step_id,queued.step.version,&attempt.attempt_id,attempt.version,session,
        AgentExecutionAttemptSessionKind::ChildAttempt,None, None,&event(AgentExecutionEventKind::AttemptChanged)).await.unwrap()
}

#[tokio::test]
async fn soft_deleted_task_retirement_reads_exact_original_facts_without_acknowledging_or_revealing_ordinary_rows() {
    let f=Fixture::memory(&["original","unrelated"],&[]).await;let task=f.start("original").await;f.complete("original").await;
    let attempt=task.current_attempt.as_ref().unwrap();let session=attempt.conversation_id.as_deref().unwrap();
    add_source(&f,&task,"original-continuation-key").await;
    let detail=f.detail().await;
    f.repository.append_event(&f.owner,&f.execution_id,detail.execution.version,&NewAgentExecutionEvent {
        step_id:Some(detail.steps.iter().find(|step|step.title=="unrelated").unwrap().step_id.clone()),
        payload:serde_json::json!({"unrelated_output":"not retirement evidence".repeat(1000)}).to_string(),
        ..event(AgentExecutionEventKind::StepChanged)}).await.unwrap();
    let detail=f.detail().await;
    f.repository.delete_execution(&f.owner,&f.execution_id,detail.execution.version,&NewAgentExecutionEvent {
        actor:AgentExecutionActor::user(&f.owner),payload:serde_json::json!({"deleted":true}).to_string(),..event(AgentExecutionEventKind::Deleted)}).await.unwrap();
    assert!(f.repository.get_execution(&f.owner,&f.execution_id).await.unwrap().is_none());
    assert!(f.repository.get_step_detail(&f.owner,&f.execution_id,&task.step.step_id).await.unwrap().is_none());
    assert!(f.repository.list_events(&f.owner,&f.execution_id,0,100).await.unwrap().is_empty());
    assert!(!f.repository.resolve_conversation_link(&f.owner,session).await.unwrap().is_empty());
    let before:(i64,i64,i64)=nomifun_db::sqlx::query_as("SELECT version,event_sequence,(SELECT COUNT(*) FROM agent_execution_events WHERE execution_id=execution.execution_id) FROM agent_executions execution WHERE execution_id=?")
        .bind(&f.execution_id).fetch_one(f.db.pool()).await.unwrap();
    let facts=read(&f,&task).await.unwrap();assert!(facts.execution.deleted_at.is_some());
    assert_eq!(facts.original_attempt.attempt.attempt_id,attempt.attempt.attempt_id);
    assert_eq!(facts.detail.current_attempt.as_ref().unwrap().attempt.attempt_id,attempt.attempt.attempt_id);
    assert_eq!(facts.target,ConversationCleanupTurnTarget::Exact {operation_id:format!("turn:user:{}:{session}:original-continuation-key",f.owner)});
    assert_eq!(facts.events.len(),2,"only the exact original source and Deleted event are selected");
    assert!(facts.events.iter().any(|event|event.event_type=="deleted"));
    let facts_again=read(&f,&task).await.unwrap();assert_eq!(facts_again.events.len(),facts.events.len());
    let after:(i64,i64,i64)=nomifun_db::sqlx::query_as("SELECT version,event_sequence,(SELECT COUNT(*) FROM agent_execution_events WHERE execution_id=execution.execution_id) FROM agent_executions execution WHERE execution_id=?")
        .bind(&f.execution_id).fetch_one(f.db.pool()).await.unwrap();assert_eq!(before,after);
    let acked:bool=nomifun_db::sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM conversation_execution_links WHERE execution_id=? AND attempt_id=? AND cleanup_completed_at IS NOT NULL)")
        .bind(&f.execution_id).bind(&attempt.attempt.attempt_id).fetch_one(f.db.pool()).await.unwrap();assert!(!acked,"the retirement read never acknowledges cleanup");
    assert!(f.repository.read_task_retirement_facts(FOREIGN_CANONICAL_LEAD_ID,session,&f.execution_id,&task.step.step_id,&attempt.attempt.attempt_id).await.unwrap().is_none());
}

#[tokio::test]
async fn task_retirement_keeps_historical_and_current_owners_separate_and_reads_only_formal_superseded_evidence() {
    let f=Fixture::memory(&["original","peer"],&[]).await;let task=f.start("original").await;f.complete("original").await;
    let attempt=task.current_attempt.as_ref().unwrap();let session=attempt.conversation_id.as_deref().unwrap();
    assert!(read(&f,&task).await.is_some());
    let peer=start_on(&f,"peer",session).await;assert!(read(&f,&task).await.is_none(),"another active Attempt retains this Session's workspace");
    nomifun_db::sqlx::query("UPDATE conversation_execution_links SET relation='automation' WHERE execution_id=? AND attempt_id=?")
        .bind(&f.execution_id).bind(&peer.current_attempt.as_ref().unwrap().attempt.attempt_id).execute(f.db.pool()).await.unwrap();
    assert!(read(&f,&task).await.is_none(),"an active automation owner in this same Session retains the identical protection");
    f.complete("peer").await;assert!(read(&f,&task).await.is_some());
    let foreign=nomifun_common::generate_id();insert_canonical_session(&f.db,&foreign,&f.owner,"live").await;
    assert!(f.repository.read_task_retirement_facts(&f.owner,&foreign,&f.execution_id,&task.step.step_id,&attempt.attempt.attempt_id).await.unwrap().is_none());
    let detail=f.detail().await;let original=detail.steps.iter().find(|step|step.step_id==task.step.step_id).unwrap();
    let reset=f.repository.reset_steps_for_retry(&f.owner,&f.execution_id,detail.execution.version,&[nomifun_db::RetryAgentExecutionStep {
        step_id:original.step_id.clone(),expected_step_version:original.version}],&event(AgentExecutionEventKind::StepChanged)).await.unwrap();
    let step=reset.steps.iter().find(|step|step.step_id==task.step.step_id).unwrap();
    let queued=f.repository.create_attempt(&f.owner,&f.execution_id,&step.step_id,step.version,None,&CreateAgentExecutionAttemptParams {
        participant_id:step.assigned_participant_id.clone(),start_immediately:false,trigger_reason:"manual_retry".into(),effective_config:"{}".into(),retry_after:None,runtime_state:None},
        &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    let historical=read(&f,&task).await.unwrap();
    let current=historical.detail.current_attempt.as_ref().unwrap();
    assert_eq!(historical.original_attempt.attempt.attempt_id,attempt.attempt.attempt_id);
    assert_ne!(current.attempt.attempt_id,attempt.attempt.attempt_id);
    assert_eq!(current.attempt.status,"queued");assert!(current.conversation_id.is_none());
    assert_eq!(historical.detail.step.status,"pending","historical cleanup facts do not declare the task mission settled");
    let next_session=nomifun_common::generate_id();insert_canonical_session(&f.db,&next_session,&f.owner,"live").await;
    let next=queued.current_attempt.as_ref().unwrap();
    let running=f.repository.start_attempt(&f.owner,&f.execution_id,&queued.step.step_id,queued.step.version,&next.attempt.attempt_id,next.attempt.version,
        &next_session,AgentExecutionAttemptSessionKind::ChildAttempt,None, None,&event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    let next=running.current_attempt.as_ref().unwrap();
    f.repository.settle_attempt(&f.owner,&f.execution_id,&running.step.step_id,running.step.version,&next.attempt.attempt_id,next.attempt.version,None,
        &nomifun_db::SettleAgentExecutionAttemptParams {expected_active_session_turn:None,expected_terminal_turn:None,
            attempt_status:nomifun_common::ExecutionAttemptStatus::Completed,step_status:ExecutionStepStatus::Completed,execution_status:None,
            question:Some(None),error:Some(None),output_summary:Some(Some("the successor result".into())),output_files:None,tokens:None,
            retry_after:Some(None),runtime_state:Some(None),started_at:None,finished_at:None,loop_repeat_reset:None},
        &NewAgentExecutionEvent {payload:serde_json::json!({"attempt_status":"completed","step_status":"completed","turn_operation_key":"successor-formal-turn"}).to_string(),
            ..event(AgentExecutionEventKind::AttemptChanged)}).await.unwrap();
    let final_facts=read(&f,&task).await.unwrap();assert_eq!(final_facts.original_attempt.attempt.attempt_id,attempt.attempt.attempt_id);
    assert_eq!(final_facts.detail.current_attempt.as_ref().unwrap().attempt.attempt_id,next.attempt.attempt_id);
    assert_eq!(final_facts.detail.current_attempt.as_ref().unwrap().conversation_id.as_deref(),Some(next_session.as_str()));
    assert_eq!(final_facts.target,ConversationCleanupTurnTarget::Exact {operation_id:format!("turn:user:{}:{session}:{}:initial-turn",f.owner,attempt.attempt.attempt_id)});
    assert!(final_facts.events.iter().any(|event|event.attempt_id.as_deref()==Some(next.attempt.attempt_id.as_str())
        &&serde_json::from_str::<serde_json::Value>(&event.payload).unwrap().get("turn_operation_key").and_then(serde_json::Value::as_str)==Some("successor-formal-turn")));
    let detail=f.detail().await;
    let replaced=f.repository.reconcile_plan(&f.owner,&f.execution_id,detail.execution.version,&ReconcileAgentExecutionPlanParams {
        goal:None,adaptation_policy:None,decision_policy:None,delegation_policy:None,keep_step_ids:vec![task.step.step_id.clone()],
        new_participants:vec![],retire_participant_ids:vec![],new_steps:vec![],new_dependencies:vec![],execution_status:AgentExecutionStatus::Running},
        &NewAgentExecutionEvent {actor:AgentExecutionActor::user(&f.owner),payload:serde_json::json!({"change":"adjusted","intent":"retain the completed replacement task"}).to_string(),
            ..event(AgentExecutionEventKind::PlanChanged)}).await.unwrap();
    assert!(f.repository.get_step_detail(&f.owner,&f.execution_id,&peer.step.step_id).await.unwrap().is_none(),"ordinary business visibility remains unchanged");
    let retired=read(&f,&peer).await.unwrap();assert_eq!(retired.detail.step.superseded_in_revision,Some(replaced.execution.plan_revision));
    assert!(retired.events.iter().any(|event|event.event_type=="plan_changed"&&{
        let payload:serde_json::Value=serde_json::from_str(&event.payload).unwrap();
        payload["plan_revision"]==replaced.execution.plan_revision&&payload["superseded_step_ids"].as_array().unwrap().contains(&serde_json::json!(peer.step.step_id))
    }),"the superseded cleanup read returns its actual same-writer revision and selected Step witness");
}

#[tokio::test]
async fn task_retirement_keeps_ancestor_selected_stop_and_pending_original_target_without_faking_queue_settlement() {
    let f=Fixture::memory(&["parent"],&[]).await;f.start("parent").await;f.append("parent","child").await;
    let child=f.start("child").await;let attempt=child.current_attempt.as_ref().unwrap();let session=attempt.conversation_id.as_deref().unwrap();
    let target=format!("turn:user:{}:{session}:the-original-controlled-turn",f.owner);
    let effects=AttemptConversationEffects {pending_conversation_effects:vec![PendingConversationEffect::StopTurn {
        operation_id:"original-stop-effect".into(),target_operation_id:target.clone()}],review_blocked:None};
    // This isolates the read boundary while retaining an unacknowledged
    // original control intent. It is not a native Stop or cleanup receipt.
    let encoded=effects.encode().unwrap();nomifun_db::sqlx::query("UPDATE agent_execution_attempts SET runtime_state=? WHERE attempt_id=?")
        .bind(&encoded).bind(&attempt.attempt.attempt_id).execute(f.db.pool()).await.unwrap();
    let detail=f.detail().await;let parent=detail.steps.iter().find(|step|step.title=="parent").unwrap();
    f.repository.cancel_step(&f.owner,&f.execution_id,detail.execution.version,&parent.step_id,parent.version,AgentExecutionCancellationScope::DelegatedWork,
        &NewAgentExecutionEvent {actor:AgentExecutionActor::user(&f.owner),..event(AgentExecutionEventKind::StepChanged)}).await.unwrap();
    let facts=read(&f,&child).await.unwrap();assert_eq!(facts.target,ConversationCleanupTurnTarget::Exact {operation_id:target});
    assert_eq!(facts.original_attempt.attempt.runtime_state.as_deref(),Some(encoded.as_str()));
    assert_eq!(facts.detail.current_attempt.as_ref().unwrap().attempt.runtime_state.as_deref(),Some(encoded.as_str()));
    let stop=facts.events.iter().find(|event|event.event_type=="step_changed").unwrap();
    assert_eq!(stop.step_id.as_deref(),Some(parent.step_id.as_str()));assert_ne!(stop.step_id.as_deref(),Some(child.step.step_id.as_str()));
    assert!(serde_json::from_str::<serde_json::Value>(&stop.payload).unwrap()["cancelled_step_ids"].as_array().unwrap()
        .contains(&serde_json::json!(child.step.step_id)));
    assert!(!facts.events.iter().any(|event|serde_json::from_str::<serde_json::Value>(&event.payload).unwrap().get("change").and_then(serde_json::Value::as_str)==Some("conversation_effect_delivered")));
    assert_eq!(f.task("parent").await.step.status,"running");
}

#[tokio::test]
async fn task_retirement_reads_completed_automation_without_giving_other_leads_browser_authority() {
    let f=Fixture::memory(&["original"],&[]).await;let task=f.start("original").await;f.complete("original").await;
    let attempt=task.current_attempt.as_ref().unwrap();let session=attempt.conversation_id.as_deref().unwrap();
    // The reader consumes both official link kinds. Their creation/admission
    // distinction is tested separately by the real AutomationLead writer.
    nomifun_db::sqlx::query("UPDATE conversation_execution_links SET relation='automation' WHERE execution_id=? AND attempt_id=?")
        .bind(&f.execution_id).bind(&attempt.attempt.attempt_id).execute(f.db.pool()).await.unwrap();
    nomifun_db::sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,active,created_at,updated_at) VALUES(?,?,'lead',1,1,1)")
        .bind(session).bind(&f.execution_id).execute(f.db.pool()).await.unwrap();
    assert!(read(&f,&task).await.is_some(),"the completed root's task purpose ends while its aggregate may continue independent work");
    let detail=f.detail().await;
    f.repository.append_event(&f.owner,&f.execution_id,detail.execution.version,&NewAgentExecutionEvent {
        payload:serde_json::json!({"reason":"original_automation_source_stopped"}).to_string(),
        ..event(AgentExecutionEventKind::StatusChanged)}).await.unwrap();
    assert!(read(&f,&task).await.unwrap().events.iter().any(|event|
        serde_json::from_str::<serde_json::Value>(&event.payload).unwrap().get("reason").and_then(serde_json::Value::as_str)==Some("original_automation_source_stopped")),
        "return the exact original source-stop candidate; the consumer must still verify its Requirement source");
    let detail=f.detail().await;f.repository.cancel_execution(&f.owner,&f.execution_id,detail.execution.version,&NewAgentExecutionEvent {
        actor:AgentExecutionActor::user(&f.owner),payload:serde_json::json!({"status":"cancelled","reason":"cancelled_by_caller"}).to_string(),
        ..event(AgentExecutionEventKind::StatusChanged)}).await.unwrap();
    let facts=read(&f,&task).await.unwrap();assert!(facts.events.iter().any(|event|event.event_type=="status_changed"));
    let other=create_execution(&f.repository).await;
    nomifun_db::sqlx::query("INSERT INTO conversation_execution_links(conversation_id,execution_id,relation,active,created_at,updated_at) VALUES(?,?,'lead',0,1,1)")
        .bind(session).bind(&other.execution_id).execute(f.db.pool()).await.unwrap();
    assert!(read(&f,&task).await.is_some(),"another aggregate's lead navigation does not grant Browser purpose in this Session");
    f.repository.cancel_execution(&f.owner,&other.execution_id,other.version,&NewAgentExecutionEvent {
        payload:serde_json::json!({"status":"cancelled","reason":"cancelled_by_caller"}).to_string(),..event(AgentExecutionEventKind::StatusChanged)}).await.unwrap();
    assert!(read(&f,&task).await.is_some());
}
