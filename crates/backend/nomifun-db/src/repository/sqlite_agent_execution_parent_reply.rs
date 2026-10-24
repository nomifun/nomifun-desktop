//! Agent-authored replies use the existing decision outbox and original
//! Execution writer. They never become authenticated human consent.
use super::*;
use nomifun_agent_contracts::{AgentCollaborationMessage,AgentCollaborationMessageIntent,AgentParentQuestionReply,AgentQuestionReplyReference};

pub(super) async fn validate_tx(tx:&mut Transaction<'_,Sqlite>,user:&str,execution:&str,step:&str,attempt:&str,
    session:&str,params:&AttemptConversationEffectParams,event:&NewAgentExecutionEvent)->Result<(),DbError> {
    let payload:serde_json::Value=serde_json::from_str(&event.payload).map_err(|error|DbError::Init(error.to_string()))?;
    let effects=AttemptConversationEffects::decode(params.runtime_state.as_deref()).map_err(|error|DbError::Init(error.to_string()))?;
    let source=payload.get("message_source").filter(|value|!value.is_null());
    let Some(source)=source else {
        if matches!(&event.actor,AgentExecutionActor::Agent {..})||payload.get("question_reply").is_some()
            ||effects.pending_conversation_effects.iter().any(|effect|matches!(effect,crate::PendingConversationEffect::DecisionInput {parent_reply:Some(_),..})) {
            return Err(conflict("parent question reply requires its exact native source"));
        }
        return Ok(());
    };
    let source:crate::AgentExecutionMessageSource=serde_json::from_value(source.clone()).map_err(|error|DbError::Conflict(error.to_string()))?;
    if event.event_type!=AgentExecutionEventKind::DecisionAnswered
        ||!matches!(&event.actor,AgentExecutionActor::Agent {conversation_id:Some(actor),..} if actor==&source.agent_session_id)
        ||payload.get("answered").and_then(serde_json::Value::as_bool)!=Some(true)
        ||payload.get("reply_authority").and_then(serde_json::Value::as_str)!=Some("parent_agent_guidance") {
        return Err(conflict("parent question reply actor and authority"));
    }
    let operation=payload.get("operation_id").and_then(serde_json::Value::as_str).ok_or_else(||conflict("parent reply operation"))?;
    let digest=payload.get("content_digest").and_then(serde_json::Value::as_str).ok_or_else(||conflict("parent reply digest"))?;
    crate::repository::agent_source_reply::verify_message_source_tx(tx,user,&source).await?;
    let (_,arguments)=crate::repository::agent_source_reply::agent_message_command_bound_tx(tx,&source,execution,step,operation,digest).await?;
    let command:AgentCollaborationMessage=serde_json::from_value(arguments.0).map_err(|error|DbError::Init(error.to_string()))?;
    let AgentCollaborationMessageIntent::QuestionReply {question}=&command.intent else {return Err(conflict("Agent command is not a question reply"));};
    let declared:AgentQuestionReplyReference=serde_json::from_value(payload.get("question_reply").cloned().ok_or_else(||conflict("parent reply question reference"))?)
        .map_err(|error|DbError::Conflict(error.to_string()))?;
    if &declared!=question||question.agent_session_id.as_ref()!=session {return Err(conflict("parent reply exact target question"));}
    let row:AgentExecutionRow=sqlx::query_as("SELECT * FROM agent_executions WHERE execution_id=? AND user_id=? AND deleted_at IS NULL")
        .bind(execution).bind(user).fetch_optional(&mut **tx).await?.ok_or_else(||conflict("parent reply Execution"))?;
    source_continuation::owning_source_tx(tx,&row,step,&source.agent_session_id,&source.turn_operation_id).await?;
    let current=load_step_detail_tx(tx,user,execution,step).await?.ok_or_else(||conflict("parent reply Step"))?;
    let waiting=current.current_attempt.as_ref().filter(|current|current.attempt.attempt_id==attempt)
        .ok_or_else(||conflict("parent reply current Attempt"))?;
    if current.step.status!="waiting_input"||waiting.attempt.status!="waiting_input"||waiting.conversation_id.as_deref()!=Some(session) {
        return Err(conflict("parent reply waiting Attempt"));
    }
    let key=crate::execution_attempt_turn_key_tx(tx,user,execution,step,attempt).await?;
    if question.turn_operation_id.as_ref()!=format!("turn:user:{user}:{session}:{key}") {return Err(conflict("parent reply original invocation"));}
    let (request,terminal,_)=crate::agent_parent_question_tx(tx,user,question).await?;
    if waiting.attempt.question.as_deref()!=Some(request.question.as_str()) {return Err(conflict("parent reply current question text"));}
    let notice:Option<String>=sqlx::query_scalar("SELECT payload FROM agent_execution_events WHERE execution_id=? AND step_id=? AND attempt_id=? AND event_type='decision_requested' ORDER BY sequence DESC LIMIT 1")
        .bind(execution).bind(step).bind(attempt).fetch_optional(&mut **tx).await?;
    let notice:serde_json::Value=serde_json::from_str(&notice.ok_or_else(||conflict("parent reply original decision request"))?).map_err(|error|DbError::Init(error.to_string()))?;
    if ["force_click","requires_explicit_click"].iter().any(|field|notice.get(*field).and_then(serde_json::Value::as_bool)==Some(true))
        ||notice.get("canonical_operation_id").and_then(serde_json::Value::as_str)!=Some(question.turn_operation_id.as_ref())
        ||notice.get("terminal_event_id").and_then(serde_json::Value::as_str)!=Some(terminal.as_str())
        ||notice.get("question").and_then(serde_json::Value::as_str)!=Some(request.question.as_str())
        ||notice.get("user_input_request")!=Some(&serde_json::to_value(&request).map_err(|error|DbError::Init(error.to_string()))?) {
        return Err(conflict("parent reply cannot replace a human decision or a successor request"));
    }
    let parent=AgentParentQuestionReply {question:question.clone(),sender_session_id:source.agent_session_id.clone().into(),
        sender_turn_operation_id:source.turn_operation_id.clone().into(),sender_command_event_id:source.command_event_id.clone().into()};
    let mut expected=AttemptConversationEffects::decode(waiting.attempt.runtime_state.as_deref()).map_err(|error|DbError::Init(error.to_string()))?;
    expected.push_parent_reply(operation.into(),command.content,parent).map_err(|error|DbError::Conflict(error.to_string()))?;
    if effects!=expected {return Err(conflict("parent reply original decision outbox and content"));}
    Ok(())
}
