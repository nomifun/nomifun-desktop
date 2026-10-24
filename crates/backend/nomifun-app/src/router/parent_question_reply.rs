//! Deliver an already admitted parent reply through the original child Session.
//! Its DecisionInput is the outbox; Agent provenance is never human consent.
use super::*;
use nomifun_agent_contracts::{AgentCollaborationMessage,AgentCollaborationMessageIntent,AgentParentQuestionReply};

pub(super) const ORIGIN:&str="agent_execution_parent_reply";
fn sql(error:sqlx::Error)->AppError {nomifun_db::DbError::Query(error).into()}

/// This private metadata is retained in the canonical input, while Runtime's
/// public SendMessageRequest projection receives only its validated fields.
pub(super) fn validate_metadata(input:&Value)->Result<(),AppError> {
    let Some(value)=input.get("parent_question_reply") else {
        return if input.get("origin").and_then(Value::as_str)==Some(ORIGIN) {
            Err(AppError::Conflict("parent reply input lost its canonical provenance".into()))
        } else {Ok(())};
    };
    let source:AgentParentQuestionReply=serde_json::from_value(value.clone()).map_err(|error|AppError::Conflict(error.to_string()))?;
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Content {kind:String,human_permission:bool,message:String,source:AgentParentQuestionReply}
    let raw=input.get("content").and_then(Value::as_str).ok_or_else(||AppError::Conflict("parent reply content is missing".into()))?;
    let content:Content=serde_json::from_str(raw).map_err(|error|AppError::Conflict(error.to_string()))?;
    if input.get("origin").and_then(Value::as_str)!=Some(ORIGIN)||content.kind!="parent_agent_question_reply"||content.human_permission
        ||content.source!=source||nomifun_agent_contracts::parent_question_reply_content(&content.message,&source).map_err(AppError::Internal)?!=raw
        ||input.get("reply_to").and_then(Value::as_object).is_none_or(|value|value.len()!=2)
        ||input.pointer("/reply_to/question_seq").and_then(Value::as_u64).is_none_or(|seq|seq==0)
        ||input.pointer("/reply_to/turn_id").and_then(Value::as_str).is_none_or(|id|Uuid::parse_str(id).map_or(true,|id|id.get_version_num()!=7)) {
        return Err(AppError::Conflict("parent reply envelope differs from its exact Agent source".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input()->Value {
        let source=AgentParentQuestionReply {question:nomifun_agent_contracts::AgentQuestionReplyReference {
            agent_session_id:Uuid::now_v7().to_string().into(),turn_operation_id:"original-child-turn".into(),question_event_id:"original-question-event".into(),request_id:"original-request".into()},
            sender_session_id:Uuid::now_v7().to_string().into(),sender_turn_operation_id:"original-parent-turn".into(),sender_command_event_id:"original-parent-command".into()};
        json!({"content":nomifun_agent_contracts::parent_question_reply_content("  original parent data\n",&source).unwrap(),
            "origin":ORIGIN,"reply_to":{"turn_id":Uuid::now_v7().to_string(),"question_seq":3},"parent_question_reply":source})
    }
    #[test]
    fn canonical_parent_provenance_is_validated_before_the_runtime_public_projection() {
        let original=input();
        let request=super::super::session_input_intake::runtime_message_request(&StrictJsonValue(original.clone())).unwrap();
        assert_eq!(request.content,original["content"].as_str().unwrap());assert_eq!(request.origin.as_deref(),Some(ORIGIN));
        assert!(NomiCoreSessionOwner::validate_ordinary_message_origin(&request).is_err(),"ordinary clients cannot borrow the formal parent origin");
        for mutation in ["missing","human_permission","provenance","reply","unknown"] {
            let mut changed=original.clone();
            match mutation {
                "missing"=>{changed.as_object_mut().unwrap().remove("parent_question_reply");},
                "human_permission"=>{let mut content:Value=serde_json::from_str(changed["content"].as_str().unwrap()).unwrap();content["human_permission"]=json!(true);changed["content"]=json!(content.to_string());},
                "provenance"=>changed["parent_question_reply"]["sender_command_event_id"]=json!("different-parent-command"),
                "reply"=>changed["reply_to"]["question_seq"]=json!(0),
                _=>changed["unexpected_client_field"]=json!(true),
            }
            assert!(super::super::session_input_intake::runtime_message_request(&StrictJsonValue(changed)).is_err(),"{mutation}");
        }
    }
}

impl NomiCoreSessionOwner {
    pub(super) fn deliver_parent_question_reply_owned(self:Arc<Self>,owner_id:String,session_id:String,operation:String,
        authority:AgentExecutionTurnAuthority,request:SendMessageRequest,source:AgentParentQuestionReply)
        ->std::pin::Pin<Box<dyn Future<Output=Result<nomifun_agent_execution::AgentExecutionDelivery,AppError>>+Send>> {
        self.deliver_attempt_input_owned(owner_id,session_id.into(),operation,authority,request,Some(source))
    }

    pub(super) async fn deliver_parent_question_reply_once(&self,owner_id:&str,session:&AgentSessionId,operation:&str,
        authority:&AgentExecutionTurnAuthority,request:SendMessageRequest,source:&AgentParentQuestionReply)
        ->Result<nomifun_agent_execution::AgentExecutionDelivery,AppError> {
        let session_id=session.as_ref().to_owned();
        if request.origin.as_deref()!=Some(ORIGIN)||!request.files.is_empty()||!request.inject_skills.is_empty()
            ||request.hidden||request.channel_platform.is_some()||source.question.agent_session_id.as_ref()!=session_id {
            return Err(AppError::BadRequest("parent reply requires its exact child and Agent-only input envelope".into()));
        }
        let canonical_operation=Self::turn_operation_id(&owner_id,&session_id,&operation);
        let _gate=self.session_operation_lock(&session_id).write_owned().await;
        let original=self.canonical.store().read_turn_receipt(&session,&canonical_operation).await.map_err(agent_session_store_error)?;
        let mut tx=self.pool.begin().await.map_err(sql)?;
        let state:Option<String>=sqlx::query_scalar("SELECT runtime_state FROM agent_execution_attempts WHERE execution_id=? AND step_id=? AND attempt_id=? AND status='running'")
            .bind(&authority.execution_id).bind(&authority.step_id).bind(&authority.attempt_id).fetch_one(&mut *tx).await.map_err(sql)?;
        let effects=nomifun_db::AttemptConversationEffects::decode(state.as_deref())?;
        if effects.review_blocked.is_some() {return Err(AppError::Conflict("the original parent reply outbox requires owner review".into()));}
        let Some(nomifun_db::PendingConversationEffect::DecisionInput {operation_id,content,parent_reply:Some(parent)})=effects.pending_conversation_effects.first() else {
            return Err(AppError::Conflict("the original Attempt does not own this first parent reply".into()));
        };
        if operation_id!=operation||parent!=source||nomifun_agent_contracts::parent_question_reply_content(content,source).map_err(AppError::Internal)?!=request.content {
            return Err(AppError::Conflict("parent reply differs from its original pending bytes and source".into()));
        }
        let rows:Vec<(String,Option<String>,String)>=sqlx::query_as("SELECT actor_type,actor_conversation_id,payload FROM agent_execution_events WHERE execution_id=? AND step_id=? AND attempt_id=? AND event_type='decision_answered' AND json_extract(payload,'$.operation_id')=? ORDER BY sequence LIMIT 2")
            .bind(&authority.execution_id).bind(&authority.step_id).bind(&authority.attempt_id).bind(&operation).fetch_all(&mut *tx).await.map_err(sql)?;
        let [(actor,actor_session,payload)]=rows.as_slice() else {return Err(AppError::Conflict("parent reply has no unique original DecisionAnswered receipt".into()));};
        let payload:Value=serde_json::from_str(payload).map_err(|error|AppError::Internal(error.to_string()))?;
        let message_source:nomifun_db::AgentExecutionMessageSource=serde_json::from_value(payload.get("message_source").cloned()
            .ok_or_else(||AppError::Conflict("original parent command source is missing".into()))?).map_err(|error|AppError::Conflict(error.to_string()))?;
        if actor!="agent"||actor_session.as_deref()!=Some(source.sender_session_id.as_ref())
            ||message_source.agent_session_id!=source.sender_session_id.as_ref()||message_source.turn_operation_id!=source.sender_turn_operation_id.as_ref()
            ||message_source.command_event_id!=source.sender_command_event_id.as_ref()
            ||payload.get("question_reply")!=Some(&serde_json::to_value(&source.question).map_err(|error|AppError::Internal(error.to_string()))?)
            ||payload.get("reply_authority").and_then(Value::as_str)!=Some("parent_agent_guidance")||payload.get("answered").and_then(Value::as_bool)!=Some(true) {
            return Err(AppError::Conflict("parent reply provenance differs from its admitted Agent receipt".into()));
        }
        let digest=payload.get("content_digest").and_then(Value::as_str).ok_or_else(||AppError::Conflict("parent reply digest is missing".into()))?;
        // Admission already checked the sender's live epoch. Delivery verifies
        // its immutable proposal so a later parent pause/end cannot revoke an
        // independent child's already accepted input responsibility.
        let (_,arguments)=nomifun_db::agent_message_command_bound_tx(&mut tx,&message_source,&authority.execution_id,&authority.step_id,&operation,digest)
            .await.map_err(AppError::from)?;
        let command:AgentCollaborationMessage=serde_json::from_value(arguments.0).map_err(|error|AppError::Internal(error.to_string()))?;
        if command.content!=*content||!matches!(&command.intent,AgentCollaborationMessageIntent::QuestionReply {question} if question==&source.question) {
            return Err(AppError::Conflict("parent reply no longer matches its exact native proposal".into()));
        }
        let original_binding=if original.status==nomifun_agent_session::TurnReceiptStatus::NotFound {
            Some(nomifun_db::agent_parent_question_tx(&mut tx,&owner_id,&source.question).await.map_err(AppError::from)?.2)
        } else {None};
        let reference:Option<(i64,String)>=sqlx::query_as("SELECT question.seq,turn.source_message_id FROM agent_events question JOIN agent_turns turn ON turn.session_id=question.session_id AND turn.operation_id=question.correlation_id WHERE question.session_id=? AND question.event_id=? AND turn.operation_id=? AND question.kind='runtime/progress-recorded' AND question.kind_version=1 AND question.producer_id='runtime_supervisor'")
            .bind(&session_id).bind(source.question.question_event_id.as_ref()).bind(source.question.turn_operation_id.as_ref())
            .fetch_optional(&mut *tx).await.map_err(sql)?;
        let (sequence,original_input)=reference.ok_or_else(||AppError::Conflict("parent reply lost its original canonical question".into()))?;
        let sequence=u64::try_from(sequence).map_err(|_|AppError::Conflict("parent reply question cursor is invalid".into()))?;
        tx.commit().await.map_err(sql)?;
        let question=self.canonical.store().question_source(&session,&original_input,sequence).await.map_err(agent_session_store_error)?;
        if question.request_id!=source.question.request_id {return Err(AppError::Conflict("parent reply request identity changed".into()));}
        let exact=self.canonical.get(&PrincipalRef {principal_kind:"user".into(),principal_id:owner_id.to_owned()},session).await?;
        let _workspace=self.guard_binding_workspace_read(&owner_id,&exact.session.agent_binding)?;
        let mut input=self.canonical_turn_input_with_admission(&owner_id,&session,&canonical_operation,&request).await?;
        if let Some(binding)=original_binding {
            if input.pointer("/admission/binding_ref")!=Some(&serde_json::to_value(binding).map_err(|error|AppError::Internal(error.to_string()))?) {
                return Err(AppError::Conflict("parent reply cannot borrow a replacement question binding".into()));
            }
        }
        input["reply_to"]=json!({"turn_id":original_input,"question_seq":sequence});
        input["parent_question_reply"]=serde_json::to_value(source).map_err(|error|AppError::Internal(error.to_string()))?;
        validate_metadata(&input)?;
        let principal=PrincipalRef {principal_kind:"user".into(),principal_id:owner_id.to_owned()};
        let receipt=self.canonical.start_execution_turn(&principal,session,operation,input,authority).await?;
        #[cfg(test)]
        self.attempt_input_owner_cutpoint(session,&request,"admitted").await?;
        self.deliver_claimed_turn_inner(&owner_id,session,request,receipt,true).await.map(agent_execution_delivery_from_conversation)
    }
}
