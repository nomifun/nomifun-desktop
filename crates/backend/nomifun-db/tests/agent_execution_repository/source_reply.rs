use super::*;
use nomifun_db::{AgentExecutionSourceAppend,AppendAgentExecutionStepsParams};
use nomifun_agent_contracts::{AgentBindingValue,AgentHandoffBindingRefV1,PresetRevisionRef,ResolvedSnapshotRef};

fn binding()->AgentBindingValue {
    AgentBindingValue {preset_revision_ref:PresetRevisionRef {preset_id:"fixture".into(),revision:1,revision_digest:"a".repeat(64).into()},
        resolved_snapshot_ref:ResolvedSnapshotRef {snapshot_id:"fixture-snapshot".into(),snapshot_digest:"b".repeat(64).into()},
        typed_resource_bindings:vec![],binding_version:1}
}

async fn event(db:&nomifun_db::Database,session:&str,seq:i64,kind:&str,producer:&str,correlation:&str,cause:Option<&str>,value:serde_json::Value)->String {
    let id=nomifun_common::generate_id();
    let correlation=if correlation=="self" {id.as_str()} else {correlation};
    sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json) VALUES(?,?,?,?,?,?,1,?,?,?)")
        .bind(session).bind(seq).bind(&id).bind(producer).bind(&id).bind(kind).bind(correlation).bind(cause).bind(value.to_string())
        .execute(db.pool()).await.unwrap();id
}

fn started(operation:&str,input:&str,binding:&AgentBindingValue)->serde_json::Value {
    serde_json::json!({"operation_id":operation,"source_message_id":input,"admission":{
        "resolved_snapshot_ref":binding.resolved_snapshot_ref,"binding_ref":AgentHandoffBindingRefV1::from(binding)}})
}

async fn reply(fixture:&scoped_stop::Fixture,session:&str)->(String,String) {
    let b=binding();
    sqlx::query("UPDATE agent_sessions SET agent_binding_json=? WHERE agent_session_id=?")
        .bind(serde_json::to_string(&b).unwrap()).bind(session).execute(fixture.db.pool()).await.unwrap();
    let original=event(&fixture.db,session,1,"message/user-accepted","session_api","self",None,serde_json::json!({"content":"original task"})).await;
    let start=event(&fixture.db,session,2,"turn/started","session_api","source-turn",Some(&original),started("source-turn",&original,&b)).await;
    let request=serde_json::json!({"request_id":"confirm","question":"Confirm the next input?","remaining_work":"continue original child"});
    event(&fixture.db,session,3,"runtime/progress-recorded","runtime_supervisor","source-turn",Some(&start),
        serde_json::json!({"producer_seq":1,"event":{"event":"user_input_requested","step":1,"request":request}})).await;
    let terminal=event(&fixture.db,session,4,"turn/completed","runtime_supervisor","source-turn",Some(&start),serde_json::json!({"user_input_request":request})).await;
    sqlx::query("UPDATE agent_turns SET state='completed',source_message_id=?,started_event_id=?,terminal_event_id=?,finished_at=4 WHERE session_id=? AND operation_id='source-turn'")
        .bind(&original).bind(&start).bind(terminal).bind(session).execute(fixture.db.pool()).await.unwrap();
    let accepted=event(&fixture.db,session,5,"message/user-accepted","session_api","self",None,
        serde_json::json!({"content":"yes, continue","reply_to":{"turn_id":original,"question_seq":3}})).await;
    let start=event(&fixture.db,session,6,"turn/started","session_api","reply-turn",Some(&accepted),started("reply-turn",&accepted,&b)).await;
    sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,source_message_id,started_event_id,accepted_at,started_at,execution_owner,execution_fence,execution_generation) VALUES(?,'reply-turn','reply-turn','reply-turn','running',?,?,5,6,'reply-native',9,6)")
        .bind(session).bind(&accepted).bind(start).execute(fixture.db.pool()).await.unwrap();
    sqlx::query("UPDATE agent_session_heads SET status='running',active_turn_id='reply-turn',last_seq=6 WHERE session_id=?")
        .bind(session).execute(fixture.db.pool()).await.unwrap();
    ("reply-turn".into(),accepted)
}

#[tokio::test]
async fn explicit_question_reply_scope_and_native_epoch_govern_new_child_input() {
    for mutation in ["valid","plain","clear","binding","question","fence","generation","stored","stored_corrupt"] {
        let (fixture,session,_)=super::source_continuation::fixture(&["original child"]).await;
        let original=fixture.start("original child").await;fixture.complete("original child").await;
        let (_,accepted)=reply(&fixture,&session).await;
        let content="a new input following the explicit reply";
        let (source,operation)=super::source_continuation::message_source(&fixture,&session,"reply-turn",&original.step.step_id,&format!("reply-{mutation}"),content).await;
        match mutation {
            "plain"=>{sqlx::query("UPDATE agent_events SET inline_json=json_remove(inline_json,'$.reply_to') WHERE event_id=?").bind(&accepted).execute(fixture.db.pool()).await.unwrap();},
            "clear"=>{event(&fixture.db,&session,8,"context/cleared","session_api","clear",None,serde_json::json!({})).await;},
            "binding"=>{let mut b=binding();b.binding_version=2;sqlx::query("UPDATE agent_sessions SET agent_binding_json=? WHERE agent_session_id=?").bind(serde_json::to_string(&b).unwrap()).bind(&session).execute(fixture.db.pool()).await.unwrap();},
            "question"=>{sqlx::query("UPDATE agent_events SET inline_json=json_set(inline_json,'$.event.request.request_id','other') WHERE session_id=? AND seq=3").bind(&session).execute(fixture.db.pool()).await.unwrap();},
            "fence"=>{sqlx::query("UPDATE agent_turns SET execution_fence=10 WHERE session_id=? AND operation_id='reply-turn'").bind(&session).execute(fixture.db.pool()).await.unwrap();},
            "generation"=>{sqlx::query("UPDATE agent_turns SET execution_generation=7 WHERE session_id=? AND operation_id='reply-turn'").bind(&session).execute(fixture.db.pool()).await.unwrap();},
            "stored"|"stored_corrupt"=>{
                let raw:String=sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE event_id=?").bind(&accepted).fetch_one(fixture.db.pool()).await.unwrap();
                let value:serde_json::Value=serde_json::from_str(&raw).unwrap();
                let bytes=nomifun_agent_contracts::canonical_json_bytes(&value).unwrap();
                let payload=nomifun_common::generate_id();
                let digest=if mutation=="stored_corrupt" {"0".repeat(64)} else {nomifun_agent_contracts::digest_bytes(&bytes).0};
                sqlx::query("INSERT INTO agent_payloads(payload_id,session_id,media_type,byte_len,digest,body) VALUES(?,?,'application/json',?,?,?)")
                    .bind(&payload).bind(&session).bind(bytes.len() as i64).bind(digest)
                    .bind(serde_json::to_vec(&nomifun_agent_contracts::SessionPayloadBody::Json(nomifun_agent_contracts::StrictJsonValue(value))).unwrap())
                    .execute(fixture.db.pool()).await.unwrap();
                sqlx::query("UPDATE agent_events SET inline_json=NULL,payload_id=? WHERE event_id=?").bind(payload).bind(&accepted).execute(fixture.db.pool()).await.unwrap();
            },
            _=>{},
        }
        let mut next=step(nomifun_common::generate_id(),original.step.assigned_participant_id.clone(),"continued child");next.spec=content.into();
        let before=fixture.detail().await;
        let result=fixture.repository.append_source_steps(&fixture.owner,&fixture.execution_id,before.execution.version,
            &AgentExecutionSourceAppend::ChildFollowup {message_source:source,original_step_id:original.step.step_id.clone(),operation_id:operation,content_digest:nomifun_agent_contracts::digest_bytes(content.as_bytes()).0},
            &AppendAgentExecutionStepsParams {new_steps:vec![next],new_dependencies:vec![]},
            &NewAgentExecutionEvent {actor:nomifun_common::AgentExecutionActor::agent(&session,None),..super::event(AgentExecutionEventKind::PlanChanged)}).await;
        if matches!(mutation,"valid"|"stored") {
            let result=result.unwrap();assert_eq!(result.added_step_ids.len(),1);
            let events=fixture.repository.list_events(&fixture.owner,&fixture.execution_id,0,100).await.unwrap();
            let value:serde_json::Value=serde_json::from_str(&events.last().unwrap().payload).unwrap();
            assert_eq!(value["sender_turn_operation_id"],"reply-turn");
            assert_eq!(value["delegation_source"]["turn_operation_id"],"source-turn");
        } else {
            assert!(result.is_err(),"{mutation} cannot grant executable reply continuity");
            assert_eq!(fixture.detail().await.execution.version,before.execution.version);
        }
    }
}

#[tokio::test]
async fn explicit_reply_batch_keeps_the_original_execution_and_uses_its_true_new_source() {
    let (fixture,session,_)=super::source_continuation::fixture(&["existing child"]).await;
    let (source_turn,_)=reply(&fixture,&session).await;
    let mut command=super::delegation_creation::delegated_params(&session,&source_turn,"reply-batch");command.goal="a new batch in the original task".into();
    let before=fixture.detail().await;
    let existing=&before.steps[0];
    let result=fixture.repository.append_source_steps(&fixture.owner,&fixture.execution_id,before.execution.version,
        &AgentExecutionSourceAppend::Delegation {command:command.clone()},
        &AppendAgentExecutionStepsParams {new_steps:vec![step(nomifun_common::generate_id(),existing.assigned_participant_id.clone(),"reply batch")],new_dependencies:vec![]},
        &NewAgentExecutionEvent {actor:nomifun_common::AgentExecutionActor::agent(&session,None),..super::event(AgentExecutionEventKind::PlanChanged)}).await.unwrap();
    let actual=fixture.repository.step_source(&fixture.owner,&fixture.execution_id,&result.added_step_ids[0]).await.unwrap().unwrap();
    assert_eq!(actual.turn_operation_id,"reply-turn");assert_eq!(actual.tool_operation_id,"tool-reply-batch");
    assert_eq!(fixture.repository.find_source_command(&fixture.owner,&command).await.unwrap().unwrap().execution_id,fixture.execution_id);
    assert_eq!(result.detail.execution.max_parallel,before.execution.max_parallel);
}

#[tokio::test]
async fn continued_child_binding_comes_from_its_original_admission_and_rechecks_at_link_commit() {
    let (fixture,parent,_)=super::source_continuation::fixture(&["original child"]).await;
    let original=fixture.start("original child").await;fixture.complete("original child").await;
    let attempt=original.current_attempt.as_ref().unwrap();let session=attempt.conversation_id.as_ref().unwrap();
    let mut child=binding();child.resolved_snapshot_ref.snapshot_id="derived-child-snapshot".into();
    let expected=AgentHandoffBindingRefV1::from(&child);
    sqlx::query("UPDATE agent_sessions SET agent_binding_json=? WHERE agent_session_id=?")
        .bind(serde_json::to_string(&child).unwrap()).bind(session).execute(fixture.db.pool()).await.unwrap();
    let operation=format!("turn:user:{}:{session}:{}:initial-turn",fixture.owner,attempt.attempt.attempt_id);
    let root=event(&fixture.db,session,1,"message/user-accepted","session_api","self",None,serde_json::json!({"content":"original child input"})).await;
    let start=event(&fixture.db,session,2,"turn/started","session_api",&operation,Some(&root),started(&operation,&root,&child)).await;
    let terminal=event(&fixture.db,session,3,"turn/completed","runtime_supervisor",&operation,Some(&start),serde_json::json!({})).await;
    sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,source_message_id,started_event_id,terminal_event_id,accepted_at,started_at,finished_at) VALUES(?,?,?,?,'completed',?,?,?,1,1,2)")
        .bind(session).bind(&operation).bind(&operation).bind(&operation).bind(root).bind(start).bind(&terminal).execute(fixture.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_session_heads(session_id,status,active_turn_id,active_set_generation,last_seq,unread_count) VALUES(?,'ready',NULL,0,3,0)")
        .bind(session).execute(fixture.db.pool()).await.unwrap();
    sqlx::query("UPDATE agent_execution_events SET payload=json_set(payload,'$.canonical_operation_id',?,'$.terminal_event_id',?) WHERE execution_id=? AND step_id=? AND attempt_id=? AND sequence=(SELECT MAX(sequence) FROM agent_execution_events WHERE execution_id=? AND step_id=? AND attempt_id=?)")
        .bind(&operation).bind(&terminal).bind(&fixture.execution_id).bind(&original.step.step_id).bind(&attempt.attempt.attempt_id)
        .bind(&fixture.execution_id).bind(&original.step.step_id).bind(&attempt.attempt.attempt_id).execute(fixture.db.pool()).await.unwrap();
    let content="new input to the derived child";
    let (message_source,operation_id)=super::source_continuation::message_source(&fixture,&parent,"source-turn",&original.step.step_id,"original-binding-input",content).await;
    let mut next=step(nomifun_common::generate_id(),original.step.assigned_participant_id.clone(),"followup");next.spec=content.into();
    let before=fixture.detail().await;
    let appended=fixture.repository.append_source_steps(&fixture.owner,&fixture.execution_id,before.execution.version,
        &AgentExecutionSourceAppend::ChildFollowup {message_source,
            original_step_id:original.step.step_id.clone(),operation_id,content_digest:nomifun_agent_contracts::digest_bytes(content.as_bytes()).0},
        &AppendAgentExecutionStepsParams {new_steps:vec![next],new_dependencies:vec![]},
        &NewAgentExecutionEvent {actor:nomifun_common::AgentExecutionActor::agent(&parent,None),..super::event(AgentExecutionEventKind::PlanChanged)}).await.unwrap();
    let target=fixture.repository.continuation_target(&fixture.owner,&fixture.execution_id,&appended.added_step_ids[0]).await.unwrap().unwrap();
    assert_eq!(fixture.repository.original_child_binding(&fixture.owner,&fixture.execution_id,&target).await.unwrap(),expected);
    let next=appended.detail.steps.iter().find(|step|step.step_id==appended.added_step_ids[0]).unwrap();
    let queued=fixture.repository.create_attempt(&fixture.owner,&fixture.execution_id,&next.step_id,next.version,None,
        &CreateAgentExecutionAttemptParams {participant_id:next.assigned_participant_id.clone(),start_immediately:false,
            trigger_reason:"initial".into(),effective_config:"{}".into(),retry_after:None,runtime_state:None},&super::event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    let queued_attempt=queued.current_attempt.as_ref().unwrap();
    child.binding_version=2;
    sqlx::query("UPDATE agent_sessions SET agent_binding_json=? WHERE agent_session_id=?")
        .bind(serde_json::to_string(&child).unwrap()).bind(session).execute(fixture.db.pool()).await.unwrap();
    assert!(fixture.repository.start_attempt(&fixture.owner,&fixture.execution_id,&queued.step.step_id,queued.step.version,
        &queued_attempt.attempt.attempt_id,queued_attempt.attempt.version,session,AgentExecutionAttemptSessionKind::ChildAttempt,None, None,
        &super::event(AgentExecutionEventKind::AttemptChanged)).await.is_err(),"rebind after preflight cannot enter the new Turn");
    child.binding_version=1;
    sqlx::query("UPDATE agent_sessions SET agent_binding_json=? WHERE agent_session_id=?")
        .bind(serde_json::to_string(&child).unwrap()).bind(session).execute(fixture.db.pool()).await.unwrap();
    let started=fixture.repository.start_attempt(&fixture.owner,&fixture.execution_id,&queued.step.step_id,queued.step.version,
        &queued_attempt.attempt.attempt_id,queued_attempt.attempt.version,session,AgentExecutionAttemptSessionKind::ChildAttempt,None, None,
        &super::event(AgentExecutionEventKind::AttemptChanged)).await.unwrap();
    assert_eq!(started.current_attempt.unwrap().conversation_id.as_deref(),Some(session.as_str()));
}

#[tokio::test]
async fn explicit_reply_ancestry_is_transitive_and_a_later_clear_revokes_prior_scope() {
    let (fixture,session,_)=super::source_continuation::fixture(&["original child"]).await;
    let (_,middle)=reply(&fixture,&session).await;
    let request=serde_json::json!({"request_id":"second-question","question":"One more fact?","remaining_work":"finish the same task"});
    let middle_start:String=sqlx::query_scalar("SELECT started_event_id FROM agent_turns WHERE session_id=? AND operation_id='reply-turn'")
        .bind(&session).fetch_one(fixture.db.pool()).await.unwrap();
    event(&fixture.db,&session,7,"runtime/progress-recorded","runtime_supervisor","reply-turn",Some(&middle_start),
        serde_json::json!({"producer_seq":1,"event":{"event":"user_input_requested","step":1,"request":request}})).await;
    let terminal=event(&fixture.db,&session,8,"turn/completed","runtime_supervisor","reply-turn",Some(&middle_start),serde_json::json!({"user_input_request":request})).await;
    sqlx::query("UPDATE agent_turns SET state='completed',terminal_event_id=?,finished_at=8 WHERE session_id=? AND operation_id='reply-turn'")
        .bind(terminal).bind(&session).execute(fixture.db.pool()).await.unwrap();
    let accepted=event(&fixture.db,&session,9,"message/user-accepted","session_api","self",None,
        serde_json::json!({"content":"another explicit answer","reply_to":{"turn_id":middle,"question_seq":7}})).await;
    let start=event(&fixture.db,&session,10,"turn/started","session_api","third-turn",Some(&accepted),started("third-turn",&accepted,&binding())).await;
    sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,source_message_id,started_event_id,accepted_at,started_at) VALUES(?,'third-turn','third-turn','third-turn','running',?,?,9,10)")
        .bind(&session).bind(accepted).bind(start).execute(fixture.db.pool()).await.unwrap();
    assert_eq!(fixture.repository.source_reply_turns(&fixture.owner,&session,"third-turn").await.unwrap(),["reply-turn","source-turn"]);
    // A later clear cannot make an older question executable through a newer
    // association. Its exact sequence remains in the original canonical log.
    event(&fixture.db,&session,11,"context/cleared","session_api","clear",None,serde_json::json!({})).await;
    assert!(fixture.repository.source_reply_turns(&fixture.owner,&session,"third-turn").await.unwrap().is_empty());
}
