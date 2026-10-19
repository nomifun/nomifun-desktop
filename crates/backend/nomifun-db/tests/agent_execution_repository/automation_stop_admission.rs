use super::*;
use nomifun_db::IRequirementRepository;

fn automation_params(claim:&nomifun_db::models::RequirementRow)->nomifun_db::CreateAgentExecutionParams {
    let mut params=execution_params();params.lead_conversation_id=Some(CANONICAL_LEAD_ID.into());
    params.initial_plan_input=serde_json::json!({"mode":"automation","source":{"requirement_id":claim.requirement_id,
        "claim_generation":claim.claim_generation,"operation_id":format!("original-autowork:{}",claim.requirement_id)},"plan":{"steps":[]}}).to_string();
    params
}

async fn stop_claim(db:&nomifun_db::Database,claim:&nomifun_db::models::RequirementRow) {
    nomifun_db::SqliteRequirementRepository::new(db.pool().clone()).resolve_claim_exact(&claim.requirement_id,
        claim.claim_generation,claim.claim_token.as_deref().unwrap(),Some(CANONICAL_LEAD_ID),None,
        &nomifun_db::RequirementClaimResolution::Cancelled {completion_note:Some("explicit user Stop".into())},3).await.unwrap().unwrap();
}

#[tokio::test]
async fn original_autowork_stop_fences_creation_and_an_already_queued_attempt_before_start() {
    let db=database().await;
    insert_canonical_session(&db,CANONICAL_LEAD_ID,OWNER_ID,"live").await;
    nomifun_db::sqlx::query("INSERT INTO agent_session_heads(session_id,status,active_turn_id,active_set_generation,last_seq,unread_count) VALUES (?,'ready',NULL,0,0,0)")
        .bind(CANONICAL_LEAD_ID).execute(db.pool()).await.unwrap();
    let repository=SqliteAgentExecutionRepository::new(db.pool().clone());
    let stopped_before_create=claimed_autowork_requirement(&db,CANONICAL_LEAD_ID).await;
    stop_claim(&db,&stopped_before_create).await;
    assert!(repository.create_execution_with_participants(OWNER_ID,&automation_params(&stopped_before_create),
        &[participant(nomifun_common::generate_id())],&event(AgentExecutionEventKind::Created)).await.is_err());
    let count:i64=nomifun_db::sqlx::query_scalar("SELECT COUNT(*) FROM agent_executions").fetch_one(db.pool()).await.unwrap();
    assert_eq!(count,0,"a claim-to-create race cannot leave an admitted aggregate after Stop");
    let claim=claimed_autowork_requirement(&db,CANONICAL_LEAD_ID).await;
    let participant_id=nomifun_common::generate_id();
    let created=repository.create_execution_with_participants(OWNER_ID,&automation_params(&claim),
        &[participant(participant_id.clone())],&event(AgentExecutionEventKind::Created)).await.unwrap();
    let mut selected=step(nomifun_common::generate_id(),Some(participant_id.clone()),"original work");selected.tool_policy=AgentToolPolicy::ReadOnly;
    let planned=repository.reconcile_plan(OWNER_ID,&created.execution_id,created.version,&ReconcileAgentExecutionPlanParams {
        goal:None,adaptation_policy:None,decision_policy:None,delegation_policy:None,keep_step_ids:Vec::new(),new_participants:Vec::new(),
        retire_participant_ids:Vec::new(),new_steps:vec![selected],new_dependencies:Vec::new(),execution_status:AgentExecutionStatus::Running},
        &event(AgentExecutionEventKind::PlanChanged)).await.unwrap();
    let root=&planned.steps[0];
    let queued=repository.create_attempt(OWNER_ID,&created.execution_id,&root.step_id,root.version,None,
        &CreateAgentExecutionAttemptParams {participant_id:Some(participant_id),start_immediately:false,trigger_reason:"initial".into(),
            effective_config:"{}".into(),retry_after:None,runtime_state:None},&event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    let attempt=&queued.current_attempt.as_ref().unwrap().attempt;
    stop_claim(&db,&claim).await;
    assert!(repository.start_attempt(OWNER_ID,&created.execution_id,&root.step_id,queued.step.version,&attempt.attempt_id,attempt.version,
        CANONICAL_LEAD_ID,AgentExecutionAttemptSessionKind::AutomationLead,None,&event(AgentExecutionEventKind::AttemptChanged)).await.is_err());
    let after=repository.get_step_detail(OWNER_ID,&created.execution_id,&root.step_id).await.unwrap().unwrap();
    assert_eq!(after.step.version,queued.step.version);assert_eq!(after.current_attempt.unwrap().attempt.status,"queued");
    assert!(repository.automation_source_cancelled(OWNER_ID,&created.execution_id).await.unwrap(),
        "restart can distinguish the exact committed source Stop from a runtime query failure");
    assert_eq!(repository.list_conversation_links(OWNER_ID,&created.execution_id).await.unwrap().len(),1,"no native input link was admitted");
    nomifun_db::sqlx::query("UPDATE requirements SET claim_generation=claim_generation+1 WHERE requirement_id=?")
        .bind(&claim.requirement_id).execute(db.pool()).await.unwrap();
    assert!(!repository.automation_source_cancelled(OWNER_ID,&created.execution_id).await.unwrap(),
        "a different source generation cannot manufacture cancellation proof for this aggregate");
}
