use super::*;
use nomifun_db::{RetryAgentExecutionStep,AgentExecutionClosedTurn,ReplaceAgentExecutionStepWithExistingParams};

fn command_event(kind:AgentExecutionEventKind,operation:&str,request:serde_json::Value)->NewAgentExecutionEvent {
    NewAgentExecutionEvent {actor:nomifun_common::AgentExecutionActor::user(OWNER_ID),
        payload:serde_json::json!({"change":"retry_requested","operation_id":operation,"command_request":request}).to_string(),..event(kind)}
}

#[tokio::test]
async fn linked_initial_input_cannot_rewrite_the_original_queued_body_or_drop_selections() {
    for mutation in ["content","files","inject_skills"] {
        let f=scoped_stop::Fixture::memory(&["original input"],&[]).await;let task=f.task("original input").await;
        let original=serde_json::json!({"content":"immutable task","files":["original.txt"],"inject_skills":["original-skill"],"origin":"agent_execution","hidden":true,"channel_platform":null});
        let state=serde_json::json!({"pending_conversation_effects":[{"kind":"initial_input","request":original}]}).to_string();
        let queued=f.repository.create_attempt(&f.owner,&f.execution_id,&task.step.step_id,task.step.version,None,&CreateAgentExecutionAttemptParams {
            participant_id:task.step.assigned_participant_id.clone(),start_immediately:false,trigger_reason:"initial".into(),effective_config:"{}".into(),retry_after:None,runtime_state:Some(state.clone())},
            &event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
        let attempt=queued.current_attempt.as_ref().unwrap();let session=nomifun_common::generate_id();insert_canonical_session(&f.db,&session,&f.owner,"live").await;
        let mut changed=original.clone();changed[mutation]=if mutation=="content" {"another task".into()} else {serde_json::json!([])};
        let input=nomifun_db::PendingConversationEffect::InitialInput {request:changed};
        assert!(f.repository.start_attempt(&f.owner,&f.execution_id,&task.step.step_id,queued.step.version,&attempt.attempt.attempt_id,attempt.attempt.version,
            &session,AgentExecutionAttemptSessionKind::ChildAttempt,None,Some(&input),&event(AgentExecutionEventKind::AttemptChanged)).await.is_err());
        let preserved=f.task("original input").await.current_attempt.unwrap();assert_eq!(preserved.attempt.status,"queued");assert_eq!(preserved.attempt.runtime_state.as_deref(),Some(state.as_str()));
        assert!(f.repository.list_conversation_links(&f.owner,&f.execution_id).await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn exact_retry_replay_never_reopens_a_successor_or_reexecutes_a_sibling() {
    let f=scoped_stop::Fixture::memory(&["retry child","completed sibling"],&[]).await;
    f.start("retry child").await;f.complete("retry child").await;
    f.start("completed sibling").await;f.complete("completed sibling").await;
    let original=f.task("retry child").await;let sibling=f.task("completed sibling").await;
    let version=f.detail().await.execution.version;
    let request=vec![RetryAgentExecutionStep {step_id:original.step.step_id.clone(),expected_step_version:original.step.version}];
    let command=command_event(AgentExecutionEventKind::StepChanged,"exact-original-retry",serde_json::json!({"step":original.step.step_id,"version":original.step.version}));
    let accepted=f.repository.reset_steps_for_retry(&f.owner,&f.execution_id,version,&request,&command).await.unwrap();
    let replay=f.repository.reset_steps_for_retry(&f.owner,&f.execution_id,version,&request,&command).await.unwrap();
    assert_eq!(accepted.execution.event_sequence,replay.execution.event_sequence);
    let successor=f.start("retry child").await;f.complete("retry child").await;
    let settled=f.task("retry child").await;
    f.repository.reset_steps_for_retry(&f.owner,&f.execution_id,version,&request,&command).await.unwrap();
    let after=f.task("retry child").await;
    assert_eq!(after.step.status,"completed");assert_eq!(after.step.version,settled.step.version);
    assert_eq!(after.current_attempt.unwrap().attempt.attempt_id,successor.current_attempt.unwrap().attempt.attempt_id);
    assert_eq!(f.task("completed sibling").await.step.version,sibling.step.version);
    let mut changed=command.clone();changed.payload=serde_json::json!({"operation_id":"exact-original-retry","command_request":{"step":"another"}}).to_string();
    assert!(f.repository.reset_steps_for_retry(&f.owner,&f.execution_id,version,&request,&changed).await.is_err());
}

#[tokio::test]
async fn parent_retry_uses_original_native_source_and_preserves_logical_task_ownership() {
    use nomifun_agent_contracts::{AgentCollaborationMessage,AgentCollaborationMessageIntent,AgentCollaborationTaskControl,AgentCollaborationTaskAction};
    let (f,parent,_)=source_continuation::fixture(&["same child"]).await;
    f.start("same child").await;f.complete("same child").await;
    let task=f.task("same child").await;let execution=f.detail().await.execution;
    let original_source=f.repository.step_source(&f.owner,&f.execution_id,&task.step.step_id).await.unwrap().unwrap();
    let (source,operation)=source_continuation::message_source(&f,&parent,"source-turn",&task.step.step_id,"retry-key","retry the original child").await;
    let command=AgentCollaborationMessage {execution_id:f.execution_id.clone(),step_id:task.step.step_id.clone(),content:"retry the original child".into(),idempotency_key:"retry-key".into(),
        intent:AgentCollaborationMessageIntent::TaskControl {control:AgentCollaborationTaskControl {expected_execution_version:execution.version,expected_step_version:task.step.version,action:AgentCollaborationTaskAction::Retry {}}}};
    sqlx::query("UPDATE agent_events SET inline_json=json_set(inline_json,'$.event.call.arguments',json(?)) WHERE session_id=? AND event_id=?")
        .bind(serde_json::to_string(&command).unwrap()).bind(&parent).bind(&source.command_event_id).execute(f.db.pool()).await.unwrap();
    let event=NewAgentExecutionEvent {actor:nomifun_common::AgentExecutionActor::agent(&parent,None),step_id:Some(task.step.step_id.clone()),payload:serde_json::json!({
        "change":"retry_requested","operation_id":operation,"content_digest":command.content_digest().unwrap(),"message_source":source,
        "command_request":{"kind":"native_task_control","digest":nomifun_agent_contracts::digest_payload(&command).unwrap()}}).to_string(),..event(AgentExecutionEventKind::StepChanged)};
    let target=[RetryAgentExecutionStep {step_id:task.step.step_id.clone(),expected_step_version:task.step.version}];
    let accepted=f.repository.reset_steps_for_retry(&f.owner,&f.execution_id,execution.version,&target,&event).await.unwrap();
    let replay=f.repository.reset_steps_for_retry(&f.owner,&f.execution_id,execution.version,&target,&event).await.unwrap();
    assert_eq!(accepted.execution.event_sequence,replay.execution.event_sequence);
    let after=f.start("same child").await;
    assert_eq!(after.step.step_id,task.step.step_id);
    assert_ne!(after.current_attempt.unwrap().attempt.attempt_id,task.current_attempt.unwrap().attempt.attempt_id);
    let after_source=f.repository.step_source(&f.owner,&f.execution_id,&task.step.step_id).await.unwrap().unwrap();
    assert_eq!(after_source.agent_session_id,original_source.agent_session_id);
    assert_eq!(after_source.turn_operation_id,original_source.turn_operation_id);
}

#[tokio::test]
async fn terminal_committing_after_reader_requires_reread_without_retiring_original_link() {
    let db=database().await;let repository=SqliteAgentExecutionRepository::new(db.pool().clone());
    let (execution,running,output)=recovery_fixture(&db,&repository).await;
    let row=repository.get_execution(OWNER_ID,&execution).await.unwrap().unwrap();let lease=AgentExecutionLeaseToken::new("race-owner".into());
    repository.try_acquire_lease(&execution,row.version,lease.owner(),nomifun_common::now_ms()+30_000).await.unwrap().unwrap();
    let attempt=&running.current_attempt.as_ref().unwrap().attempt;
    let reread=repository.reconcile_recovered_attempt(OWNER_ID,&execution,&running.step.step_id,running.step.version,&attempt.attempt_id,attempt.version,
        &lease,None,&event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    assert_eq!(reread.disposition,AgentExecutionAttemptRecoveryDisposition::ReReadRequired);
    assert_eq!(reread.detail.step.version,running.step.version);
    assert!(repository.resolve_conversation_link(OWNER_ID,&output.conversation_id).await.unwrap()[0].active);
    let adopted=repository.reconcile_recovered_attempt(OWNER_ID,&execution,&running.step.step_id,running.step.version,&attempt.attempt_id,attempt.version,
        &lease,Some(&output),&event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    assert_eq!(adopted.detail.step.status,"completed");
}

#[tokio::test]
async fn original_review_can_adopt_a_later_verified_terminal_through_its_inactive_link() {
    let db=database().await;let repository=SqliteAgentExecutionRepository::new(db.pool().clone());
    let (execution,running,output)=recovery_fixture(&db,&repository).await;
    let state=serde_json::json!({"pending_conversation_effects":[],"review_blocked":{"kind":"interrupted_turn","operation_id":output.canonical_operation_id,"receipt_state":"missing","reason":"old missing receipt"}}).to_string();
    sqlx::query("UPDATE agent_execution_attempts SET status='waiting_input',runtime_state=? WHERE attempt_id=?").bind(state).bind(&output.attempt_id).execute(db.pool()).await.unwrap();
    sqlx::query("UPDATE agent_execution_steps SET status='waiting_input' WHERE step_id=?").bind(&running.step.step_id).execute(db.pool()).await.unwrap();
    sqlx::query("UPDATE conversation_execution_links SET active=0 WHERE attempt_id=?").bind(&output.attempt_id).execute(db.pool()).await.unwrap();
    let row=repository.get_execution(OWNER_ID,&execution).await.unwrap().unwrap();let lease=AgentExecutionLeaseToken::new("review-owner".into());
    repository.try_acquire_lease(&execution,row.version,lease.owner(),nomifun_common::now_ms()+30_000).await.unwrap().unwrap();
    let adopted=repository.reconcile_recovered_attempt(OWNER_ID,&execution,&running.step.step_id,running.step.version,&output.attempt_id,
        running.current_attempt.unwrap().attempt.version,&lease,Some(&output),&event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    assert_eq!(adopted.detail.step.status,"completed");assert_eq!(adopted.detail.current_attempt.unwrap().attempt.attempt_id,output.attempt_id);
    assert_eq!(repository.get_execution_detail(OWNER_ID,&execution).await.unwrap().unwrap().attempts.len(),1);
}

#[tokio::test]
async fn missing_original_admission_fences_old_sender_and_retains_same_attempt_input() {
    for half_accepted in [false,true] {
        let db=database().await;let repository=SqliteAgentExecutionRepository::new(db.pool().clone());
        let (execution,running,output)=recovery_fixture(&db,&repository).await;
        sqlx::query("DELETE FROM agent_turns WHERE session_id=?").bind(&output.conversation_id).execute(db.pool()).await.unwrap();
        sqlx::query("DELETE FROM agent_events WHERE session_id=?").bind(&output.conversation_id).execute(db.pool()).await.unwrap();
        sqlx::query("INSERT INTO agent_session_heads(session_id,status,last_seq,active_set_generation,unread_count) VALUES(?,'ready',0,0,0)")
            .bind(&output.conversation_id).execute(db.pool()).await.unwrap();
        let state=serde_json::json!({"pending_conversation_effects":[{"kind":"initial_input","request":{"content":"exact original content"}}]}).to_string();
        sqlx::query("UPDATE agent_execution_attempts SET runtime_state=? WHERE attempt_id=?").bind(&state).bind(&output.attempt_id).execute(db.pool()).await.unwrap();
        let key=format!("{}:initial-turn",output.attempt_id);
        if half_accepted {
            let event=nomifun_common::generate_id();
            sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,1,?,'session_api',?,'message/user-accepted',1,?,'{}')")
                .bind(&output.conversation_id).bind(&event).bind(format!("user:{OWNER_ID}:{}:{key}:message",output.conversation_id)).bind(&event).execute(db.pool()).await.unwrap();
        }
        let row=repository.get_execution(OWNER_ID,&execution).await.unwrap().unwrap();let lease=AgentExecutionLeaseToken::new("new-input-owner".into());
        repository.try_acquire_lease(&execution,row.version,lease.owner(),nomifun_common::now_ms()+30_000).await.unwrap().unwrap();
        let attempt=&running.current_attempt.as_ref().unwrap().attempt;
        let recovered=repository.reconcile_recovered_attempt(OWNER_ID,&execution,&running.step.step_id,running.step.version,&attempt.attempt_id,attempt.version,
            &lease,None,&event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
        if half_accepted {
            assert_eq!(recovered.disposition,AgentExecutionAttemptRecoveryDisposition::ReviewBlocked,"partial canonical facts cannot authorize new work");
        } else {
            assert_eq!(recovered.disposition,AgentExecutionAttemptRecoveryDisposition::InitialDeliveryPending);
            assert_eq!(recovered.detail.current_attempt.as_ref().unwrap().attempt.attempt_id,attempt.attempt_id);
            assert!(recovered.detail.step.version>running.step.version);
            let mut tx=db.pool().begin().await.unwrap();
            let stale=nomifun_db::AgentExecutionTurnAuthority {execution_id:execution.clone(),step_id:running.step.step_id.clone(),attempt_id:attempt.attempt_id.clone(),
                expected_step_version:running.step.version,expected_attempt_version:attempt.version,lease_owner:lease.owner().into()};
            assert!(nomifun_db::validate_execution_turn_authority_tx(&mut tx,OWNER_ID,&output.conversation_id,&key,&stale).await.is_err());
            tx.rollback().await.unwrap();
        }
    }
}

#[tokio::test]
async fn formal_existing_replacement_preserves_results_rewires_only_unstarted_dependencies_and_replays_once() {
    let f=scoped_stop::Fixture::memory(&["old","replacement","next"],&[("old","next")]).await;
    f.start("old").await;
    let replacement=f.start("replacement").await;f.complete("replacement").await;
    let current=f.task("replacement").await;let old=f.task("old").await;
    let attempt=replacement.current_attempt.unwrap();let session=attempt.conversation_id.unwrap();
    let operation=format!("turn:user:{}:{session}:{}:initial-turn",f.owner,attempt.attempt.attempt_id);let terminal=nomifun_common::generate_id();
    sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,1,?,'runtime_supervisor',?,'turn/completed',1,?,'{}')")
        .bind(&session).bind(&terminal).bind(&terminal).bind(&operation).execute(f.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,terminal_event_id,accepted_at,finished_at) VALUES(?,?,?,?,'completed',?,1,2)")
        .bind(&session).bind(&operation).bind(&operation).bind(&operation).bind(&terminal).execute(f.db.pool()).await.unwrap();
    let version=f.detail().await.execution.version;
    let params=ReplaceAgentExecutionStepWithExistingParams {old_step_id:old.step.step_id.clone(),expected_old_step_version:old.step.version,
        replacement_step_id:current.step.step_id.clone(),expected_replacement_step_version:current.step.version,
        replacement_terminal:AgentExecutionClosedTurn {conversation_id:session,canonical_operation_id:operation,terminal_event_id:terminal}};
    let command=command_event(AgentExecutionEventKind::PlanChanged,"replace-once",serde_json::json!({"old":old.step.step_id,"replacement":current.step.step_id}));
    let changed=f.repository.replace_step_with_existing(&f.owner,&f.execution_id,version,&params,&command).await.unwrap();
    assert_eq!(changed.steps.iter().find(|step|step.step_id==old.step.step_id).unwrap().status,"cancelled");
    assert_eq!(f.task("replacement").await.step.version,current.step.version);
    assert!(changed.dependencies.iter().filter(|edge|edge.superseded_in_revision.is_none()).all(|edge|edge.blocker_step_id==current.step.step_id));
    assert_eq!(changed.attempts.len(),2);
    let replay=f.repository.replace_step_with_existing(&f.owner,&f.execution_id,version,&params,&command).await.unwrap();
    assert_eq!(replay.execution.event_sequence,changed.execution.event_sequence);
}

#[tokio::test]
async fn replacement_rejects_started_downstream_and_unresolved_original_effects_atomically() {
    for obstacle in ["started_downstream","unknown_effect"] {
        let f=scoped_stop::Fixture::memory(&["old","replacement","next"],&[("old","next")]).await;
        let old=f.start("old").await;
        let replacement=f.start("replacement").await;f.complete("replacement").await;
        if obstacle=="started_downstream" {f.start("next").await;}
        let proof=replacement.current_attempt.unwrap();let session=proof.conversation_id.unwrap();
        let operation=format!("turn:user:{}:{session}:{}:initial-turn",f.owner,proof.attempt.attempt_id);let terminal=nomifun_common::generate_id();
        sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,1,?,'runtime_supervisor',?,'turn/completed',1,?,'{}')")
            .bind(&session).bind(&terminal).bind(&terminal).bind(&operation).execute(f.db.pool()).await.unwrap();
        sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,terminal_event_id,accepted_at,finished_at) VALUES(?,?,?,?,'completed',?,1,2)")
            .bind(&session).bind(&operation).bind(&operation).bind(&operation).bind(&terminal).execute(f.db.pool()).await.unwrap();
        if obstacle=="unknown_effect" {
            let old_session=old.current_attempt.as_ref().unwrap().conversation_id.as_ref().unwrap();
            let start=nomifun_common::generate_id();let effect=nomifun_common::generate_id();
            sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,1,?,'runtime_supervisor',?,'effect/started',1,'old-operation','{}')")
                .bind(old_session).bind(&start).bind(&start).execute(f.db.pool()).await.unwrap();
            sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,accepted_at) VALUES(?,'old-operation','old-operation','old-operation','running',1)")
                .bind(old_session).execute(f.db.pool()).await.unwrap();
            sqlx::query("INSERT INTO agent_effects(effect_id,session_id,turn_id,operation_id,owner_domain,capability_module,action_id,input_digest,strategy,state,started_event_id,created_at) VALUES(?,?,'old-operation','old-effect','test','test','test',?,'external_uncertain_effect','pending',?,1)")
                .bind(effect).bind(old_session).bind("a".repeat(64)).bind(start).execute(f.db.pool()).await.unwrap();
        }
        let current=f.detail().await;let replacement=current.steps.iter().find(|step|step.title=="replacement").unwrap();
        let params=ReplaceAgentExecutionStepWithExistingParams {old_step_id:old.step.step_id.clone(),expected_old_step_version:old.step.version,
            replacement_step_id:replacement.step_id.clone(),expected_replacement_step_version:replacement.version,
            replacement_terminal:AgentExecutionClosedTurn {conversation_id:session,canonical_operation_id:operation,terminal_event_id:terminal}};
        let command=command_event(AgentExecutionEventKind::PlanChanged,"unsafe-replacement",serde_json::json!({"old":old.step.step_id}));
        assert!(f.repository.replace_step_with_existing(&f.owner,&f.execution_id,current.execution.version,&params,&command).await.is_err(),"{obstacle}");
        let after=f.detail().await;
        assert_eq!(after.execution.version,current.execution.version);
        assert_eq!(after.execution.event_sequence,current.execution.event_sequence);
        assert!(after.steps.iter().find(|step|step.step_id==old.step.step_id).unwrap().superseded_in_revision.is_none());
    }
}
