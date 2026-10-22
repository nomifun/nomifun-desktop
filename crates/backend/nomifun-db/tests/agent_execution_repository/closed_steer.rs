use super::*;
use nomifun_db::{AgentExecutionClosedTurn,AttemptConversationEffects,AttemptConversationEffectParams};

async fn native_turn(fixture:&scoped_stop::Fixture,target:&nomifun_db::AgentExecutionStepDetailRow)->(String,String) {
    let attempt=target.current_attempt.as_ref().unwrap();let session=attempt.conversation_id.as_ref().unwrap();
    let operation=format!("turn:user:{}:{session}:{}:initial-turn",fixture.owner,attempt.attempt.attempt_id);
    let start=nomifun_common::generate_id();let terminal=nomifun_common::generate_id();
    sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,1,?,'session_api',?,'turn/started',1,?,'{}')")
        .bind(session).bind(&start).bind(&start).bind(&operation).execute(fixture.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,started_event_id,accepted_at,started_at) VALUES(?,?,?,?,'running',?,1,1)")
        .bind(session).bind(&operation).bind(&operation).bind(&operation).bind(start).execute(fixture.db.pool()).await.unwrap();
    (operation,terminal)
}

async fn close(fixture:&scoped_stop::Fixture,session:&str,operation:&str,terminal:&str) {
    sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,inline_json) VALUES(?,2,?,'runtime_supervisor',?,'turn/completed',1,?,'{}')")
        .bind(session).bind(terminal).bind(terminal).bind(operation).execute(fixture.db.pool()).await.unwrap();
    sqlx::query("UPDATE agent_turns SET state='completed',terminal_event_id=?,finished_at=2 WHERE session_id=? AND operation_id=?")
        .bind(terminal).bind(session).bind(operation).execute(fixture.db.pool()).await.unwrap();
}

#[tokio::test]
async fn closed_user_steer_becomes_one_new_input_and_releases_the_original_outcome_hold() {
    let fixture=scoped_stop::Fixture::memory(&["target"],&[]).await;
    let target=fixture.start("target").await;
    let attempt=target.current_attempt.as_ref().unwrap();let session=attempt.conversation_id.as_ref().unwrap();
    let (operation,terminal)=native_turn(&fixture,&target).await;
    let mut effects=AttemptConversationEffects::default();effects.push_steer("accepted-guidance".into(),operation.clone(),"continue independently".into()).unwrap();
    let before=fixture.detail().await;
    fixture.repository.enqueue_attempt_conversation_effect(&fixture.owner,&fixture.execution_id,before.execution.version,
        &target.step.step_id,target.step.version,&attempt.attempt.attempt_id,attempt.attempt.version,
        &AttemptConversationEffectParams {runtime_state:Some(effects.encode().unwrap())},
        &NewAgentExecutionEvent {actor:nomifun_common::AgentExecutionActor::user(&fixture.owner),payload:serde_json::json!({"operation_id":"accepted-guidance","action":"steer"}).to_string(),..event(AgentExecutionEventKind::StepChanged)}).await.unwrap();
    close(&fixture,session,&operation,&terminal).await;
    let closed=AgentExecutionClosedTurn {conversation_id:session.clone(),canonical_operation_id:operation,terminal_event_id:terminal};
    let mut wrong=closed.clone();wrong.terminal_event_id=nomifun_common::generate_id();
    assert!(fixture.repository.redirect_closed_steer(&fixture.owner,&fixture.execution_id,&target.step.step_id,&attempt.attempt.attempt_id,"accepted-guidance",&wrong).await.is_err());
    fixture.repository.redirect_closed_steer(&fixture.owner,&fixture.execution_id,&target.step.step_id,&attempt.attempt.attempt_id,"accepted-guidance",&closed).await.unwrap();
    let redirected=fixture.detail().await;
    let next=redirected.steps.iter().find(|step|step.step_id!=target.step.step_id).unwrap();
    assert_eq!(next.spec,"continue independently");assert!(redirected.dependencies.is_empty());
    let current=fixture.task("target").await;
    let pending=AttemptConversationEffects::decode(current.current_attempt.as_ref().unwrap().attempt.runtime_state.as_deref()).unwrap();
    assert!(pending.pending_conversation_effects.is_empty(),"redirect commits new work and removes only the old accepted intent atomically");
    assert!(!fixture.repository.step_session_available(&fixture.owner,&fixture.execution_id,&next.step_id).await.unwrap());
    fixture.repository.redirect_closed_steer(&fixture.owner,&fixture.execution_id,&target.step.step_id,&attempt.attempt.attempt_id,"accepted-guidance",&closed).await.unwrap();
    assert_eq!(fixture.detail().await.execution.version,redirected.execution.version);
    fixture.complete("target").await;
    assert!(fixture.repository.step_session_available(&fixture.owner,&fixture.execution_id,&next.step_id).await.unwrap());
    assert_eq!(fixture.repository.continuation_target(&fixture.owner,&fixture.execution_id,&next.step_id).await.unwrap().unwrap().conversation_id,*session);
}

#[tokio::test]
async fn active_caller_attempt_is_independent_of_steer_target_and_forged_actor_is_rejected() {
    let fixture=scoped_stop::Fixture::memory(&["sender","target"],&[]).await;
    let sender=fixture.start("sender").await;let target=fixture.start("target").await;
    let caller=sender.current_attempt.as_ref().unwrap();let target_attempt=target.current_attempt.as_ref().unwrap();
    let sender_session=caller.conversation_id.as_ref().unwrap();
    let sender_turn=format!("turn:user:{}:{sender_session}:{}:initial-turn",fixture.owner,caller.attempt.attempt_id);
    sqlx::query("INSERT INTO agent_session_heads(session_id,status,active_turn_id,active_set_generation,last_seq,unread_count) VALUES(?,'running',?,0,0,0)")
        .bind(sender_session).bind(&sender_turn).execute(fixture.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,accepted_at,started_at,execution_owner,execution_fence,execution_generation) VALUES(?,?,?,?,'running',1,1,'sender-owner',4,1)")
        .bind(sender_session).bind(&sender_turn).bind(&sender_turn).bind(&sender_turn).execute(fixture.db.pool()).await.unwrap();
    let (source,operation)=super::source_continuation::message_source(&fixture,sender_session,&sender_turn,&target.step.step_id,"sender-guidance","new guidance").await;
    let mut effects=AttemptConversationEffects::default();effects.push_steer(operation.clone(),"target-native".into(),"new guidance".into()).unwrap();
    let before=fixture.detail().await;
    let event=NewAgentExecutionEvent {actor:nomifun_common::AgentExecutionActor::agent(sender_session,Some(caller.attempt.attempt_id.clone())),
        payload:serde_json::json!({"operation_id":operation,"content_digest":nomifun_agent_contracts::digest_bytes(b"new guidance"),"message_source":source}).to_string(),..event(AgentExecutionEventKind::StepChanged)};
    let mut forged=event.clone();forged.actor=nomifun_common::AgentExecutionActor::agent(sender_session,Some(target_attempt.attempt.attempt_id.clone()));
    assert!(fixture.repository.enqueue_attempt_conversation_effect(&fixture.owner,&fixture.execution_id,before.execution.version,
        &target.step.step_id,target.step.version,&target_attempt.attempt.attempt_id,target_attempt.attempt.version,
        &AttemptConversationEffectParams {runtime_state:Some(effects.encode().unwrap())},&forged).await.is_err());
    fixture.repository.enqueue_attempt_conversation_effect(&fixture.owner,&fixture.execution_id,before.execution.version,
        &target.step.step_id,target.step.version,&target_attempt.attempt.attempt_id,target_attempt.attempt.version,
        &AttemptConversationEffectParams {runtime_state:Some(effects.encode().unwrap())},&event).await.unwrap();
    let facts=fixture.repository.list_events(&fixture.owner,&fixture.execution_id,0,100).await.unwrap();let accepted=facts.last().unwrap();
    assert_eq!(accepted.actor_attempt_id.as_deref(),Some(caller.attempt.attempt_id.as_str()));
    assert_eq!(accepted.attempt_id.as_deref(),Some(target_attempt.attempt.attempt_id.as_str()));
    sqlx::query("UPDATE agent_turns SET execution_fence=5 WHERE session_id=?").bind(sender_session).execute(fixture.db.pool()).await.unwrap();
    let current=fixture.task("target").await;let now=fixture.detail().await;
    assert!(fixture.repository.enqueue_attempt_conversation_effect(&fixture.owner,&fixture.execution_id,now.execution.version,
        &current.step.step_id,current.step.version,&target_attempt.attempt.attempt_id,current.current_attempt.as_ref().unwrap().attempt.version,
        &AttemptConversationEffectParams {runtime_state:Some(effects.encode().unwrap())},&event).await.is_err(),
        "a stale native future cannot commit after its original epoch was fenced");
}

#[tokio::test]
async fn already_delegated_children_outlive_a_parent_task_stop_but_not_its_explicit_owned_scope_stop() {
    for scope in [nomifun_common::AgentExecutionCancellationScope::Task,nomifun_common::AgentExecutionCancellationScope::DelegatedWork] {
        let fixture=scoped_stop::Fixture::memory(&["parent"],&[]).await;
        fixture.start("parent").await;
        let child=fixture.append("parent","accepted child").await;
        let before=fixture.detail().await;let parent=fixture.task("parent").await;
        fixture.repository.cancel_step(&fixture.owner,&fixture.execution_id,before.execution.version,&parent.step.step_id,parent.step.version,scope,
            &NewAgentExecutionEvent {actor:nomifun_common::AgentExecutionActor::user(&fixture.owner),..event(AgentExecutionEventKind::StepChanged)}).await.unwrap();
        let current=fixture.repository.get_step(&fixture.owner,&fixture.execution_id,&child).await.unwrap().unwrap();
        let dispatch=fixture.repository.create_attempt(&fixture.owner,&fixture.execution_id,&child,current.version,None,
            &CreateAgentExecutionAttemptParams {participant_id:current.assigned_participant_id,start_immediately:false,trigger_reason:"initial".into(),effective_config:"{}".into(),retry_after:None,runtime_state:None},
            &event(AgentExecutionEventKind::AttemptChanged)).await;
        assert_eq!(dispatch.is_ok(),scope==nomifun_common::AgentExecutionCancellationScope::Task,
            "accepted ownership follows the user's explicit stop range, not the parent's status");
    }
}
