//! Background execution questions reuse the existing canonical decision owner and
//! voice presentation contract. A Robot answer is taken from its accepted
//! authenticated input, never from model-authored tool arguments.

use super::{NomiCoreSessionOwner,canonical_companion_origin,agent_session_store_error};
use nomifun_agent_contracts::{AgentInputSource,AgentSessionId,CompanionSessionRole,OperationId,
    PrincipalRef,SessionEventAppend,SemanticSessionEventDraft,SessionEventKind,SessionEventPayloadRef,
    StrictJsonValue,digest_bytes};
use nomifun_agent_execution::{AgentExecutionInput,AgentExecutionSource};
use nomifun_agent_session::TurnReceiptStatus;
use nomifun_common::AppError;
use nomifun_db::sqlx;
use nomifun_voice_contracts::{ApprovalInteractionMode,ApprovalTarget,VoiceApprovalAnswer,
    VoiceApprovalPresentation,VoiceSpeechSourceRef,VoiceWorkReceipt};
use serde::{Deserialize,Serialize};
use serde_json::{Value,json};
use std::sync::{Arc,Weak};
use crate::router::voice_work_host::AppVoiceWorkHost;

#[path = "execution_question_playback.rs"]
mod playback;

fn invalid(detail:impl std::fmt::Display)->AppError {AppError::Conflict(format!("Robot question handoff: {detail}"))}

#[derive(Clone)]
pub(crate) struct RobotExecutionQuestionSource {
    pub(crate) companion_id:String,pub(crate) robot_id:String,pub(crate) connection_id:String,
    session_id:AgentSessionId,delegation_event_id:String,binding_version:u64,context_floor:u64,
}
pub(crate) struct PreparedExecutionQuestion {
    pub(crate) source:RobotExecutionQuestionSource,pub(crate) text:String,pub(crate) speech_source:VoiceSpeechSourceRef,
    pub(crate) presentation_id:String,
}
impl PreparedExecutionQuestion {
    pub(crate) fn playback_context(&self)->nomifun_robot::registry::RobotPlaybackContext {
        nomifun_robot::registry::RobotPlaybackContext {session_id:self.source.session_id.as_ref().into(),
            input_context:nomifun_robot::services::RobotInputContext {binding_version:self.source.binding_version,
                context_floor:self.source.context_floor,presented_question_id:None,work_operation_id:None,execution_generation:None,
                background_work:None,background_work_ambiguous:false}}
    }
}

#[derive(Serialize,Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
struct QuestionPlaybackAccepted {notice_message_id:String,notice_seq:u64,connection_id:String,presentation_id:String}

#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct QuestionNotice {
    content:String,origin:String,operation_id:String,source:AgentExecutionSource,
    connection_id:String,decision:VoiceApprovalPresentation,requires_desktop:bool,
}

/// Unrelated outbox bookkeeping may advance aggregate versions after a
/// presentation. The question, native work target and interaction binding must
/// remain exact; the owner supplies fresh CAS versions immediately before use.
fn same_question(left:&ApprovalTarget,right:&ApprovalTarget)->bool {
    left.work_target==right.work_target&&left.interaction_agent_session_id==right.interaction_agent_session_id
        &&left.interaction_binding_version==right.interaction_binding_version&&left.execution_id==right.execution_id
        &&left.step_id==right.step_id&&left.attempt_id==right.attempt_id&&left.request_event_sequence==right.request_event_sequence
        &&left.action_id==right.action_id&&left.question_digest==right.question_digest
        &&left.presentation_id==right.presentation_id&&left.interaction_mode==right.interaction_mode
}

fn substantive_answer(text:&str)->bool {
    let normalized=text.trim().to_lowercase().replace([' ','。','！','!','，',',','.'],"");
    !normalized.is_empty()&&!matches!(normalized.as_str(),"嗯"|"嗯嗯"|"呃"|"哦"|"好"|"好的"|"行"|"知道了"|"谢谢"|"ok"|"okay"|"uhhuh"|"hmm"|"thanks"
        |"stop"|"cancel"|"nevermind"|"算了"|"停一下"|"停止"|"取消任务"|"停止这个任务"|"别做了"|"不要做了")
}

impl NomiCoreSessionOwner {
    pub(crate) fn install_execution_voice_work(&self,host:&Arc<AppVoiceWorkHost>)->Result<(),String> {
        self.execution_voice_work.set(Arc::downgrade(host)).map_err(|_|"Execution voice decision owner was already installed".into())
    }
    fn execution_decisions(&self)->Result<Arc<AppVoiceWorkHost>,AppError> {
        self.execution_voice_work.get().and_then(Weak::upgrade).ok_or_else(||invalid("canonical voice decision owner is unavailable"))
    }
    /// Robot input must not scan every historical task or another Session's
    /// executions just to discover whether there is a currently pending reply.
    pub(crate) async fn execution_decision_execution_ids(&self,owner_id:&str,session_id:&AgentSessionId)->Result<Vec<String>,AppError>{
        let rows:Vec<String>=sqlx::query_scalar("SELECT execution.execution_id FROM conversation_execution_links link JOIN agent_executions execution ON execution.execution_id=link.execution_id JOIN agent_sessions session ON session.agent_session_id=link.conversation_id WHERE link.conversation_id=? AND link.relation='lead' AND link.active=1 AND execution.user_id=? AND execution.deleted_at IS NULL AND execution.status='waiting_input' AND session.state='live' AND json_extract(session.owner_ref_json,'$.principal_kind')='user' AND json_extract(session.owner_ref_json,'$.principal_id')=? ORDER BY execution.created_at LIMIT 65")
            .bind(session_id.as_ref()).bind(owner_id).bind(owner_id).fetch_all(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if rows.len()>64{return Err(invalid("active decision presentation budget exceeded"));}Ok(rows)
    }
    pub(super) async fn notify_execution_input_required(&self,owner_id:&str,input:&AgentExecutionInput)->Result<(),AppError> {
        self.execution_report_delivery.get().and_then(Weak::upgrade)
            .ok_or_else(||invalid("Robot delivery owner is unavailable"))?.deliver_input_required(owner_id,input).await
    }
    async fn execution_robot_source(&self,owner_id:&str,source:&AgentExecutionSource)->Result<Option<RobotExecutionQuestionSource>,AppError> {
        let session=AgentSessionId::from(source.agent_session_id.clone());
        let observation=self.canonical.get(&PrincipalRef{principal_kind:"user".into(),principal_id:owner_id.into()},&session).await?;
        let Some(scope)=canonical_companion_origin(owner_id,&observation)?.filter(|scope|scope.role==CompanionSessionRole::Robot) else {return Ok(None);};
        let mut tx=self.pool.begin().await.map_err(|error|AppError::Internal(error.to_string()))?;
        let authority=playback::source_authority_tx(self,&mut tx,owner_id,&session,source,&observation.session.agent_binding).await?;
        tx.commit().await.map_err(|error|AppError::Internal(error.to_string()))?;
        let Some(authority)=authority else {return Ok(None);};
        let Some(input)=authority.input.get("input_source").filter(|source|!source.is_null()).cloned()
            else{return Ok(None);}; // its existing desktop task card owns this handoff
        let AgentInputSource::AuthenticatedRobot{companion_id,robot_id,connection_id,..}=serde_json::from_value(input).map_err(invalid)?;
        if companion_id!=scope.companion_id||scope.robot_id.as_deref()!=Some(robot_id.as_str()) {return Err(invalid("question belongs to another device"));}
        Ok(Some(RobotExecutionQuestionSource{companion_id,robot_id,connection_id,session_id:session,
            delegation_event_id:authority.delegation_event_id,
            binding_version:observation.session.agent_binding.binding_version,context_floor:authority.context_floor}))
    }

    pub(crate) async fn prepare_execution_question_notice(&self,owner_id:&str,input:&AgentExecutionInput)->Result<Option<PreparedExecutionQuestion>,AppError> {
        let Some(source)=self.execution_robot_source(owner_id,&input.source).await? else{return Ok(None);};
        let initial:Option<String>=sqlx::query_scalar("SELECT initial_plan_input FROM agent_executions WHERE execution_id=? AND user_id=? AND deleted_at IS NULL")
            .bind(&input.execution_id).bind(owner_id).fetch_optional(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        let initial=nomifun_agent_execution::execution_source_from_initial_plan(&initial.ok_or_else(||invalid("owned Execution is missing"))?)?;
        if initial.as_ref()!=Some(&input.source) {return Err(invalid("input notice source differs from this Execution"));}
        let decisions=self.execution_decisions()?.current_decision_presentations(owner_id,source.session_id.as_ref(),source.binding_version).await.map_err(invalid)?;
        let Some(decision)=decisions.into_iter().find(|decision|decision.target.execution_id==input.execution_id
            &&decision.target.step_id==input.step_id&&decision.target.attempt_id==input.attempt_id
            &&decision.target.request_event_sequence==input.request_event_sequence) else{return Ok(None);};
        let requires_desktop=decision.target.interaction_mode==ApprovalInteractionMode::ExplicitClick
            ||digest_bytes(decision.question.as_bytes())!=decision.target.question_digest;
        let text=if requires_desktop {format!("任务需要你在电脑端查看并确认：{}",decision.question)} else {decision.question.clone()};
        let operation_id=format!("exec-lead-input:{}:event:{}",input.execution_id,input.request_event_sequence);
        let payload=QuestionNotice{content:text,origin:"agent_execution_input_required".into(),operation_id:operation_id.clone(),
            source:input.source.clone(),connection_id:source.connection_id.clone(),decision,requires_desktop};
        let key=format!("agent-execution-input:{operation_id}");
        let _fence=self.session_operation_lock(source.session_id.as_ref()).write_owned().await;
        let Some(current)=self.execution_robot_source(owner_id,&input.source).await? else{return Ok(None);};
        if current.binding_version!=source.binding_version||current.connection_id!=source.connection_id {return Err(invalid("question interaction binding changed"));}
        let current_questions=self.execution_decisions()?.current_decision_presentations(owner_id,source.session_id.as_ref(),source.binding_version).await.map_err(invalid)?;
        if !current_questions.iter().any(|current|same_question(&current.target,&payload.decision.target)){return Ok(None);}
        let existing:Option<(String,i64,String)>=sqlx::query_as("SELECT event_id,seq,inline_json FROM agent_events WHERE session_id=? AND producer_id='session_api' AND idempotency_key=? AND kind='message/assistant-projected' AND kind_version=1")
            .bind(source.session_id.as_ref()).bind(&key).fetch_optional(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        let (message_id,seq,text)=if let Some((id,seq,json))=existing {
            let previous:QuestionNotice=serde_json::from_str(&json).map_err(invalid)?;
            if previous.source!=payload.source||previous.connection_id!=payload.connection_id||previous.requires_desktop!=payload.requires_desktop
                ||previous.content!=payload.content||!same_question(&previous.decision.target,&payload.decision.target) {return Err(invalid("committed question identity changed"));}
            (id,u64::try_from(seq).map_err(invalid)?,previous.content)
        } else {
            let id=uuid::Uuid::now_v7().to_string();
            let ack=self.canonical.store().append_event(&SessionEventAppend{agent_session_id:source.session_id.clone(),event_id:id.clone().into(),
                producer_id:"session_api".into(),idempotency_key:key.into(),semantic_event:SemanticSessionEventDraft{kind:SessionEventKind("message/assistant-projected".into()),kind_version:1,
                    correlation_id:id.clone().into(),causation_event_id:Some(source.delegation_event_id.clone().into()),
                    payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::to_value(&payload).map_err(invalid)?))}}).await.map_err(agent_session_store_error)?;
            self.user_events.send_to_user(owner_id,Self::canonical_projected_assistant_wire_event(&source.session_id,&id,&payload.content));
            (id,ack.cursor.seq,payload.content)
        };
        let presentation_id=payload.decision.target.presentation_id;
        Ok(Some(PreparedExecutionQuestion{source,text,presentation_id,speech_source:VoiceSpeechSourceRef{message_id,revision:seq,through_seq:seq}}))
    }

    pub(crate) async fn execution_question_status(&self,owner_id:&str,session_id:&AgentSessionId,execution_id:&str)->Result<Value,AppError> {
        let observation=self.canonical.get(&PrincipalRef{principal_kind:"user".into(),principal_id:owner_id.into()},session_id).await?;
        let current=self.execution_decisions()?.current_decision_presentations(owner_id,session_id.as_ref(),observation.session.agent_binding.binding_version).await.map_err(invalid)?;
        Ok(json!(current.into_iter().filter(|question|question.target.execution_id==execution_id).map(|question|{
            let mode=if digest_bytes(question.question.as_bytes())!=question.target.question_digest{ApprovalInteractionMode::ExplicitClick}else{question.target.interaction_mode};
            json!({"question":question.question,"interaction_mode":mode})}).collect::<Vec<_>>()))
    }

    /// Called inside the existing Robot input fence, before the accepted root
    /// is committed. A later model/tool call cannot attach a newer question to
    /// an older utterance. Redelivery retains the first root's exact context.
    pub(crate) async fn capture_execution_question(&self,owner_id:&str,session_id:&AgentSessionId,connection_id:&str,key:&str,utterance_presentation_id:Option<&str>)->Result<Option<String>,AppError>{
        let operation=Self::turn_operation_id(owner_id,session_id.as_ref(),key);
        let existing=self.canonical.store().read_turn_receipt(session_id,&operation).await.map_err(agent_session_store_error)?;
        if existing.status!=TurnReceiptStatus::NotFound{
            let (_,input)=self.canonical.store().turn_accepted_input(session_id,&operation).await.map_err(agent_session_store_error)?;
            let source:AgentInputSource=serde_json::from_value(input.0.get("input_source").cloned().ok_or_else(||invalid("replayed Robot input has no original source"))?).map_err(invalid)?;
            return match source{AgentInputSource::AuthenticatedRobot{connection_id:original,presented_question_id,..} if original==connection_id=>Ok(presented_question_id),
                _=>Err(invalid("Robot redelivery differs from its original authenticated source"))};
        }
        let Some(utterance_presentation_id)=utterance_presentation_id else{return Ok(None);};
        let Some(host)=self.execution_voice_work.get().and_then(Weak::upgrade) else{return Ok(None);};
        let observation=self.canonical.get(&PrincipalRef{principal_kind:"user".into(),principal_id:owner_id.into()},session_id).await?;
        let current=host.current_decision_presentations(owner_id,session_id.as_ref(),observation.session.agent_binding.binding_version).await.map_err(invalid)?;
        if current.len()!=1||current[0].target.interaction_mode!=ApprovalInteractionMode::VoiceAllowed{return Ok(None);}
        let target=&current[0].target;
        if target.presentation_id!=utterance_presentation_id{return Ok(None);}
        let notice_key=format!("agent-execution-input:exec-lead-input:{}:event:{}",target.execution_id,target.request_event_sequence);
        let notice:Option<String>=sqlx::query_scalar("SELECT notice.inline_json FROM agent_events notice JOIN agent_events played ON played.session_id=notice.session_id AND played.kind='robot/question-playback-accepted' AND played.kind_version=1 AND played.producer_id='session_api' AND json_extract(played.inline_json,'$.notice_message_id')=notice.event_id AND json_extract(played.inline_json,'$.notice_seq')=notice.seq AND json_extract(played.inline_json,'$.connection_id')=? WHERE notice.session_id=? AND notice.producer_id='session_api' AND notice.idempotency_key=? AND notice.kind='message/assistant-projected' AND notice.kind_version=1 AND notice.seq<played.seq LIMIT 1")
            .bind(connection_id).bind(session_id.as_ref()).bind(notice_key).fetch_optional(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        let Some(notice)=notice else{return Ok(None);};let notice:QuestionNotice=serde_json::from_str(&notice).map_err(invalid)?;
        if notice.requires_desktop||notice.connection_id!=connection_id||!same_question(&notice.decision.target,target){return Ok(None);}
        if self.execution_robot_source(owner_id,&notice.source).await?.is_none(){return Ok(None);}
        Ok(Some(target.presentation_id.clone()))
    }

    pub(crate) async fn answer_execution_question(&self,owner_id:&str,session_id:&AgentSessionId,turn_id:&OperationId,
        tool_operation:&OperationId,key:&str)->Result<VoiceWorkReceipt,AppError> {
        let principal=PrincipalRef{principal_kind:"user".into(),principal_id:owner_id.into()};
        let observation=self.canonical.get(&principal,session_id).await?;
        let scope=canonical_companion_origin(owner_id,&observation)?.filter(|scope|scope.role==CompanionSessionRole::Robot).ok_or_else(||invalid("answer requires canonical Robot ingress"))?;
        let receipt=self.canonical.store().read_turn_receipt(session_id,turn_id).await.map_err(agent_session_store_error)?;
        if receipt.status!=TurnReceiptStatus::Running {return Err(invalid("answer no longer belongs to the active Robot Turn"));}
        let (root_event,accepted)=super::execution_source_context::canonical_input_at_tool(self.canonical.store(),session_id,turn_id,
            tool_operation,&"session.management".into(),&"session.management/execution_answer_decision".into()).await?;
        let input=&accepted.0;
        let answer=input.get("content").and_then(Value::as_str).filter(|text|text.len()<=16384&&substantive_answer(text)).ok_or_else(||invalid("backchannels do not answer a task question"))?;
        let AgentInputSource::AuthenticatedRobot{companion_id,robot_id,connection_id,presented_question_id}=serde_json::from_value(input.get("input_source").cloned().ok_or_else(||invalid("answer has no authenticated device source"))?).map_err(invalid)?;
        if companion_id!=scope.companion_id||scope.robot_id.as_deref()!=Some(robot_id.as_str()) {return Err(invalid("answer belongs to another device"));}
        let host=self.execution_decisions()?;
        let current=host.current_decision_presentations(owner_id,session_id.as_ref(),observation.session.agent_binding.binding_version).await.map_err(invalid)?;
        if current.len()!=1 {return Err(invalid("answer needs one exact currently pending question"));}
        let current=current.into_iter().next().unwrap();
        if presented_question_id.as_deref()!=Some(current.target.presentation_id.as_str()){return Err(invalid("this input did not observe the same unique question at admission"));}
        if current.target.interaction_mode!=ApprovalInteractionMode::VoiceAllowed {return Err(invalid("this decision requires its desktop control"));}
        let notice_key=format!("agent-execution-input:exec-lead-input:{}:event:{}",current.target.execution_id,current.target.request_event_sequence);
        let row:Option<String>=sqlx::query_scalar("SELECT notice.inline_json FROM agent_events notice JOIN agent_events played ON played.session_id=notice.session_id AND played.kind='robot/question-playback-accepted' AND played.kind_version=1 AND played.producer_id='session_api' AND json_extract(played.inline_json,'$.notice_message_id')=notice.event_id AND json_extract(played.inline_json,'$.notice_seq')=notice.seq AND json_extract(played.inline_json,'$.connection_id')=? WHERE notice.session_id=? AND notice.producer_id='session_api' AND notice.idempotency_key=? AND notice.kind='message/assistant-projected' AND notice.kind_version=1 AND played.seq<? AND notice.seq<played.seq AND json_extract(notice.inline_json,'$.origin')='agent_execution_input_required' LIMIT 1")
            .bind(&connection_id).bind(session_id.as_ref()).bind(notice_key).bind(i64::try_from(root_event.seq).map_err(invalid)?).fetch_optional(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        let notice:QuestionNotice=serde_json::from_str(&row.ok_or_else(||invalid("the pending question was not presented before this input"))?).map_err(invalid)?;
        if notice.requires_desktop||notice.connection_id!=connection_id||!same_question(&notice.decision.target,&current.target) {return Err(invalid("presented question, binding or socket changed"));}
        let Some(original)=self.execution_robot_source(owner_id,&notice.source).await? else{return Err(invalid("original question source was revoked"));};
        if original.connection_id!=connection_id {return Err(invalid("answer does not belong to the original socket"));}
        let _connection=self.execution_report_delivery.get().and_then(Weak::upgrade).ok_or_else(||invalid("Robot connection owner is unavailable"))?
            .hold_input_connection(owner_id,&companion_id,&robot_id,&connection_id).await?.ok_or_else(||invalid("original Robot connection is no longer active"))?;
        // Match the gateway's connection-lease -> Session-fence order. Taking
        // these in reverse can deadlock behind a queued connection revocation.
        let _fence=self.session_operation_lock(session_id.as_ref()).write_owned().await;
        let fresh=self.canonical.get(&principal,session_id).await?;
        if fresh.session.agent_binding.binding_version!=observation.session.agent_binding.binding_version
            ||fresh.head.active_turn_id.as_deref()!=Some(turn_id.as_ref()) {return Err(invalid("answer Turn or interaction binding changed"));}
        if self.execution_robot_source(owner_id,&notice.source).await?.is_none(){return Err(invalid("question source was revoked before answering"));}
        let fresh_questions=host.current_decision_presentations(owner_id,session_id.as_ref(),fresh.session.agent_binding.binding_version).await.map_err(invalid)?;
        if fresh_questions.len()!=1||!same_question(&fresh_questions[0].target,&current.target){return Err(invalid("pending question changed before answering"));}
        let current=fresh_questions.into_iter().next().unwrap();
        host.answer_decision_presentation(owner_id,key,VoiceApprovalAnswer{presented_context_id:current.target.presentation_id.clone(),target:current.target,answer:answer.into()}).await.map_err(invalid)
    }
}

#[cfg(test)]mod tests{use super::*;
    #[test]fn ordinary_answers_need_no_magic_prefix_but_backchannels_are_not_answers(){
        for answer in ["小明","一年级","A","选择第二项","yes","9 点"]{assert!(substantive_answer(answer));}
        for answer in ["嗯","嗯嗯","好的。","okay","谢谢","stop","取消任务"," "]{assert!(!substantive_answer(answer));}
    }
    #[test]fn bookkeeping_versions_do_not_replace_the_exact_presented_question_or_work_target(){
        let target:ApprovalTarget=serde_json::from_value(json!({
            "work_target":{"agent_session_id":"desktop","binding_version":1,"turn_operation_id":"source-turn","execution_generation":2},
            "interaction_agent_session_id":"robot","interaction_binding_version":3,"execution_id":"execution","step_id":"step","attempt_id":"attempt",
            "expected_execution_version":4,"expected_step_version":5,"expected_attempt_version":6,"request_event_sequence":7,
            "action_id":"agent/request_user_decision","question_digest":"a".repeat(64),"presentation_id":"question-seven",
            "presentation_digest":"b".repeat(64),"interaction_mode":"voice_allowed"})).unwrap();
        let mut refreshed=target.clone();refreshed.expected_execution_version+=1;refreshed.presentation_digest="c".repeat(64).into();
        assert!(same_question(&target,&refreshed));
        refreshed.request_event_sequence+=1;assert!(!same_question(&target,&refreshed));
        refreshed=target.clone();refreshed.work_target.turn_operation_id="successor".into();assert!(!same_question(&target,&refreshed));
        refreshed=target.clone();refreshed.interaction_binding_version+=1;assert!(!same_question(&target,&refreshed));
        refreshed=target.clone();refreshed.interaction_mode=ApprovalInteractionMode::ExplicitClick;assert!(!same_question(&target,&refreshed));
    }
}
