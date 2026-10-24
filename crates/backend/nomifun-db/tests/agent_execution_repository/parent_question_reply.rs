use super::*;
use nomifun_agent_contracts::{AgentBindingValue,AgentCollaborationMessage,AgentCollaborationMessageIntent,AgentHandoffBindingRefV1,
    AgentParentQuestionReply,AgentQuestionReplyReference,AgentUserInputRequest,PresetRevisionRef,ResolvedSnapshotRef};
use nomifun_db::{AgentExecutionMessageSource,AgentExecutionTerminalTurnGuard,AttemptConversationEffectParams,AttemptConversationEffects,
    PendingConversationEffect,SettleAgentExecutionAttemptParams};

struct Fixture {inner:scoped_stop::Fixture,parent:String,child:String,step:String,attempt:String,question:AgentQuestionReplyReference}

async fn canonical_event(db:&nomifun_db::Database,session:&str,seq:i64,kind:&str,producer:&str,correlation:Option<&str>,cause:Option<&str>,body:serde_json::Value)->String {
    let id=nomifun_common::generate_id();
    sqlx::query("INSERT INTO agent_events(session_id,seq,event_id,producer_id,idempotency_key,kind,kind_version,correlation_id,causation_event_id,inline_json) VALUES(?,?,?,?,?,?,1,?,?,?)")
        .bind(session).bind(seq).bind(&id).bind(producer).bind(&id).bind(kind).bind(correlation.unwrap_or(&id)).bind(cause).bind(body.to_string())
        .execute(db.pool()).await.unwrap();id
}

async fn fixture()->Fixture {
    let (inner,parent,_)=source_continuation::fixture(&["question child"]).await;
    let binding=AgentBindingValue {preset_revision_ref:PresetRevisionRef {preset_id:"question-parent".into(),revision:1,revision_digest:"a".repeat(64).into()},
        resolved_snapshot_ref:ResolvedSnapshotRef {snapshot_id:"question-snapshot".into(),snapshot_digest:"b".repeat(64).into()},typed_resource_bindings:vec![],binding_version:1};
    sqlx::query("UPDATE agent_sessions SET agent_binding_json=? WHERE agent_session_id=?")
        .bind(serde_json::to_string(&binding).unwrap()).bind(&parent).execute(inner.db.pool()).await.unwrap();
    let accepted=canonical_event(&inner.db,&parent,1,"message/user-accepted","session_api",None,None,serde_json::json!({"content":"original parent work"})).await;
    let started=canonical_event(&inner.db,&parent,2,"turn/started","session_api",Some("source-turn"),Some(&accepted),serde_json::json!({
        "operation_id":"source-turn","source_message_id":accepted,"admission":{"resolved_snapshot_ref":binding.resolved_snapshot_ref,"binding_ref":AgentHandoffBindingRefV1::from(&binding)}})).await;
    sqlx::query("UPDATE agent_turns SET source_message_id=?,started_event_id=?,accepted_at=1,started_at=2,execution_generation=2 WHERE session_id=? AND operation_id='source-turn'")
        .bind(&accepted).bind(started).bind(&parent).execute(inner.db.pool()).await.unwrap();
    sqlx::query("UPDATE agent_session_heads SET last_seq=2 WHERE session_id=?").bind(&parent).execute(inner.db.pool()).await.unwrap();
    let running=inner.start("question child").await;let current=running.current_attempt.as_ref().unwrap();
    let child=current.conversation_id.clone().unwrap();let attempt=current.attempt.attempt_id.clone();
    sqlx::query("UPDATE agent_sessions SET agent_binding_json=? WHERE agent_session_id=?")
        .bind(serde_json::to_string(&binding).unwrap()).bind(&child).execute(inner.db.pool()).await.unwrap();
    let operation=format!("turn:user:{}:{child}:{attempt}:initial-turn",inner.owner);
    let request=AgentUserInputRequest {request_id:"original-child-question".into(),question:"Which title should the original child use?".into(),remaining_work:"Finish the original child using that title.".into()};
    let accepted=canonical_event(&inner.db,&child,1,"message/user-accepted","session_api",None,None,serde_json::json!({"content":"original child input"})).await;
    let started=canonical_event(&inner.db,&child,2,"turn/started","session_api",Some(&operation),Some(&accepted),serde_json::json!({"operation_id":operation,"source_message_id":accepted,
        "admission":{"resolved_snapshot_ref":binding.resolved_snapshot_ref,"binding_ref":AgentHandoffBindingRefV1::from(&binding)}})).await;
    let question=canonical_event(&inner.db,&child,3,"runtime/progress-recorded","runtime_supervisor",Some(&operation),Some(&accepted),serde_json::json!({"producer_seq":1,"event":{"event":"user_input_requested","step":1,"request":request}})).await;
    let terminal=canonical_event(&inner.db,&child,4,"turn/completed","runtime_supervisor",Some(&operation),Some(&started),serde_json::json!({"model_steps":1,"user_input_request":request,"finished_at_ms":4})).await;
    sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,source_message_id,started_event_id,terminal_event_id,accepted_at,started_at,finished_at) VALUES(?,?,?,?,'completed',?,?,?,1,2,4)")
        .bind(&child).bind(&operation).bind(&operation).bind(&operation).bind(accepted).bind(started).bind(&terminal).execute(inner.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO agent_session_heads(session_id,status,last_seq,active_set_generation,unread_count) VALUES(?,'ready',4,0,0)")
        .bind(&child).execute(inner.db.pool()).await.unwrap();
    inner.repository.settle_attempt(&inner.owner,&inner.execution_id,&running.step.step_id,running.step.version,&attempt,current.attempt.version,None,
        &SettleAgentExecutionAttemptParams {expected_active_session_turn:None,expected_terminal_turn:Some(AgentExecutionTerminalTurnGuard::InputRequired {
            conversation_id:child.clone(),canonical_operation_id:operation.clone(),terminal_event_id:terminal.clone(),user_input_request:request.clone()}),
            attempt_status:nomifun_common::ExecutionAttemptStatus::WaitingInput,step_status:ExecutionStepStatus::WaitingInput,execution_status:Some(AgentExecutionStatus::WaitingInput),
            question:Some(Some(request.question.clone())),runtime_state:Some(None),error:None,output_summary:None,output_files:None,tokens:None,retry_after:None,started_at:None,finished_at:None,loop_repeat_reset:None},
        &NewAgentExecutionEvent {payload:serde_json::json!({"question":request.question,"user_input_request":request,"canonical_operation_id":operation,
            "terminal_event_id":terminal,"turn_operation_key":format!("{attempt}:initial-turn")}).to_string(),..event(AgentExecutionEventKind::DecisionRequested)}).await.unwrap();
    Fixture {inner,parent,child,step:running.step.step_id,attempt,question:AgentQuestionReplyReference {agent_session_id:current.conversation_id.clone().unwrap().into(),
        turn_operation_id:operation.into(),question_event_id:question.into(),request_id:request.request_id}}
}

async fn command(f:&Fixture,key:&str,question:AgentQuestionReplyReference)->(AgentExecutionMessageSource,String,AttemptConversationEffectParams,NewAgentExecutionEvent) {
    let content="  Exact parent-provided title 🦀\n";
    let (source,operation)=source_continuation::message_source(&f.inner,&f.parent,"source-turn",&f.step,key,content).await;
    let command=AgentCollaborationMessage {execution_id:f.inner.execution_id.clone(),step_id:f.step.clone(),content:content.into(),idempotency_key:key.into(),
        intent:AgentCollaborationMessageIntent::QuestionReply {question:question.clone()}};
    sqlx::query("UPDATE agent_events SET inline_json=json_set(inline_json,'$.event.call.arguments',json(?)) WHERE session_id=? AND event_id=?")
        .bind(serde_json::to_string(&command).unwrap()).bind(&f.parent).bind(&source.command_event_id).execute(f.inner.db.pool()).await.unwrap();
    let mut effects=AttemptConversationEffects::default();effects.push_parent_reply(operation.clone(),content.into(),AgentParentQuestionReply {question:question.clone(),
        sender_session_id:f.parent.clone().into(),sender_turn_operation_id:"source-turn".into(),sender_command_event_id:source.command_event_id.clone().into()}).unwrap();
    let event=NewAgentExecutionEvent {actor:nomifun_common::AgentExecutionActor::agent(&f.parent,None),payload:serde_json::json!({"answered":true,"operation_id":operation,
        "message_source":source,"content_digest":command.content_digest().unwrap(),"question_reply":question,"reply_authority":"parent_agent_guidance"}).to_string(),
        ..event(AgentExecutionEventKind::DecisionAnswered)};
    (source,operation,AttemptConversationEffectParams {runtime_state:Some(effects.encode().unwrap())},event)
}

async fn submit(f:&Fixture,params:&AttemptConversationEffectParams,event:&NewAgentExecutionEvent)->Result<(),nomifun_db::DbError> {
    let current=f.inner.task("question child").await;let execution=f.inner.detail().await.execution;let attempt=current.current_attempt.as_ref().unwrap();
    f.inner.repository.resume_waiting_attempt(&f.inner.owner,&f.inner.execution_id,execution.version,&f.step,current.step.version,&f.attempt,attempt.attempt.version,params,event).await.map(|_|())
}

#[tokio::test]
async fn parent_question_reply_preserves_agent_provenance_and_only_one_original_decision_outbox() {
    let f=fixture().await;let (_,operation,params,event)=command(&f,"same-original-reply",f.question.clone()).await;
    let current=f.inner.task("question child").await;let execution=f.inner.detail().await.execution;let attempt=current.current_attempt.as_ref().unwrap();
    let (one,two)=tokio::join!(
        f.inner.repository.resume_waiting_attempt(&f.inner.owner,&f.inner.execution_id,execution.version,&f.step,current.step.version,&f.attempt,attempt.attempt.version,&params,&event),
        f.inner.repository.resume_waiting_attempt(&f.inner.owner,&f.inner.execution_id,execution.version,&f.step,current.step.version,&f.attempt,attempt.attempt.version,&params,&event));
    assert_eq!(usize::from(one.is_ok())+usize::from(two.is_ok()),1);
    let accepted=one.or(two).unwrap();
    let effects=AttemptConversationEffects::decode(accepted.detail.current_attempt.as_ref().unwrap().attempt.runtime_state.as_deref()).unwrap();
    assert!(matches!(effects.pending_conversation_effects.as_slice(),[PendingConversationEffect::DecisionInput {operation_id,content,parent_reply:Some(parent)}]
        if operation_id==&operation&&content=="  Exact parent-provided title 🦀\n"&&parent.question==f.question&&parent.sender_session_id.as_ref()==f.parent));
    let replies=f.inner.repository.list_events(&f.inner.owner,&f.inner.execution_id,0,100).await.unwrap().into_iter()
        .filter(|event|event.event_type=="decision_answered").collect::<Vec<_>>();
    assert_eq!(replies.len(),1);assert_eq!(replies[0].actor_type,"agent");assert_eq!(replies[0].actor_conversation_id.as_deref(),Some(f.parent.as_str()));
}

#[tokio::test]
async fn parent_question_reply_rejects_stale_identity_fenced_source_human_decision_and_modified_outbox_atomically() {
    for mutation in ["question_event","request_id","target_session","source_fence","source_generation","source_scope","content","provenance","explicit_human","notice_terminal","review_blocked","binding","clear_floor","successor","stop"] {
        let f=fixture().await;let mut question=f.question.clone();
        match mutation {"question_event"=>question.question_event_id=nomifun_common::generate_id().into(),"request_id"=>question.request_id="another-request".into(),
            "target_session"=>question.agent_session_id=f.parent.clone().into(),_=>{}}
        let (_,_,mut params,event)=command(&f,&format!("reject-{mutation}"),question).await;
        match mutation {
            "source_fence"|"source_generation"=>{
                let field=if mutation=="source_fence" {"execution_fence"} else {"execution_generation"};
                let sql=if field=="execution_fence" {"UPDATE agent_turns SET execution_fence=execution_fence+1 WHERE session_id=? AND operation_id='source-turn'"}
                    else {"UPDATE agent_turns SET execution_generation=execution_generation+1 WHERE session_id=? AND operation_id='source-turn'"};
                sqlx::query(sql).bind(&f.parent).execute(f.inner.db.pool()).await.unwrap();
            },
            "source_scope"=>{sqlx::query("UPDATE agent_executions SET initial_plan_input=json_set(initial_plan_input,'$.source.turn_operation_id','different-source') WHERE execution_id=?")
                .bind(&f.inner.execution_id).execute(f.inner.db.pool()).await.unwrap();},
            "content"|"provenance"=>{
                let mut effects=AttemptConversationEffects::decode(params.runtime_state.as_deref()).unwrap();
                let PendingConversationEffect::DecisionInput {content,parent_reply,..}=&mut effects.pending_conversation_effects[0] else {unreachable!()};
                if mutation=="content" {*content="rewritten after native command".into();} else {parent_reply.as_mut().unwrap().sender_command_event_id="forged-command".into();}
                params.runtime_state=Some(effects.encode().unwrap());
            },
            "explicit_human"|"notice_terminal"=>{
                let sql=if mutation=="explicit_human" {"UPDATE agent_execution_events SET payload=json_set(payload,'$.requires_explicit_click',json('true')) WHERE execution_id=? AND event_type='decision_requested'"}
                    else {"UPDATE agent_execution_events SET payload=json_set(payload,'$.terminal_event_id','different-terminal') WHERE execution_id=? AND event_type='decision_requested'"};
                sqlx::query(sql).bind(&f.inner.execution_id).execute(f.inner.db.pool()).await.unwrap();
            },
            "review_blocked"=>{sqlx::query("UPDATE agent_execution_attempts SET runtime_state=? WHERE execution_id=? AND attempt_id=?")
                .bind(serde_json::json!({"pending_conversation_effects":[],"review_blocked":{"kind":"unknown","operation_id":"original-effect","receipt_state":"unknown","reason":"requires original owner review"}}).to_string())
                .bind(&f.inner.execution_id).bind(&f.attempt).execute(f.inner.db.pool()).await.unwrap();},
            "binding"=>{sqlx::query("UPDATE agent_sessions SET agent_binding_json=json_set(agent_binding_json,'$.binding_version',2) WHERE agent_session_id=?")
                .bind(&f.child).execute(f.inner.db.pool()).await.unwrap();},
            "clear_floor"=>{canonical_event(&f.inner.db,&f.child,5,"context/cleared","session_api",Some("clear"),None,serde_json::json!({})).await;},
            "successor"=>{sqlx::query("INSERT INTO agent_turns(session_id,turn_id,operation_id,idempotency_key,state,accepted_at) VALUES(?,'successor','successor','successor','accepted',5)")
                .bind(&f.child).execute(f.inner.db.pool()).await.unwrap();},
            "stop"=>{let before=f.inner.detail().await;let step=f.inner.task("question child").await;
                f.inner.repository.cancel_step(&f.inner.owner,&f.inner.execution_id,before.execution.version,&f.step,step.step.version,nomifun_common::AgentExecutionCancellationScope::Task,
                    &NewAgentExecutionEvent {actor:nomifun_common::AgentExecutionActor::user(&f.inner.owner),..super::event(AgentExecutionEventKind::StepChanged)}).await.unwrap();},
            _=>{},
        }
        let before=f.inner.detail().await;let sequence=before.execution.event_sequence;
        assert!(submit(&f,&params,&event).await.is_err(),"{mutation} must not authorize a parent reply");
        let after=f.inner.detail().await;
        assert_eq!(after.execution.version,before.execution.version,"{mutation}");assert_eq!(after.execution.event_sequence,sequence,"{mutation}");
        assert_eq!(f.inner.task("question child").await.current_attempt.unwrap().attempt.runtime_state,before.attempts.iter().find(|attempt|attempt.attempt.attempt_id==f.attempt).unwrap().attempt.runtime_state,"{mutation}");
    }
}
