//! Physical speech consumes irrevocable canonical prefixes, never live tokens.
use super::*;

pub(crate) struct RobotPublicOutput {
    pub(crate) segments: Vec<(nomifun_voice_contracts::VoiceSpeechSourceRef, String)>,
    pub(crate) through_seq: u64,
}

impl NomiCoreSessionOwner {
    pub(crate) async fn capture_robot_input_context(&self,owner:&str,request:&nomifun_robot::services::RobotTurnRequest)
        ->Result<nomifun_robot::services::RobotInputContext,AppError> {
        let session=AgentSessionId::from(request.conversation_id.clone());
        let _fence=self.session_operation_lock(session.as_ref()).write_owned().await;
        // A recording freezes the contract that will admit its eventual
        // transcript. Complete the normal idle evolution at this same fence
        // before capturing it, including retired physical-entry adoption.
        self.prepare_retired_companion_device_binding_locked(owner,&session).await?;
        self.prepare_session_contract_evolution(owner,&session).await?;
        let current=self.canonical.session_record(&PrincipalRef {principal_kind:"user".into(),principal_id:owner.into()},&session).await?;
        let floor:i64=sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
            .bind(session.as_ref()).fetch_one(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        let mut question=request.presented_question_id.clone();
        if question.as_deref().is_some_and(|id|id.starts_with("robot-question:")) {
            match self.direct_robot_question_reply(owner,&session,request).await {
                Ok(Some(_))=>{},Ok(None)|Err(AppError::Conflict(_)|AppError::Forbidden(_))=>question=None,
                Err(error)=>return Err(error),
            }
        }else if question.is_some() {
            let recording_key=format!("robot-recording-question:{}",Uuid::now_v7());
            question=self.capture_execution_question(owner,&session,&request.connection_id,&recording_key,question.as_deref()).await?;
        }
        let operation=self.canonical.store().head(&session).await.map_err(agent_session_store_error)?.active_turn_id;
        let generation=match operation.as_ref() {Some(operation)=>Some(self.canonical.store().native_execution_generation(&session,&operation.clone().into()).await.map_err(agent_session_store_error)?),None=>None};
        let floor=u64::try_from(floor).map_err(|error|AppError::Conflict(error.to_string()))?;
        let (background_work,background_work_ambiguous)=self.capture_robot_background_work(owner,request,current.agent_binding.binding_version,floor).await?;
        Ok(nomifun_robot::services::RobotInputContext {binding_version:current.agent_binding.binding_version,
            context_floor:floor,presented_question_id:question,
            work_operation_id:operation,execution_generation:generation,background_work,background_work_ambiguous})
    }
    pub(crate) async fn robot_input_context_matches(&self,owner:&str,request:&nomifun_robot::services::RobotTurnRequest)->Result<bool,AppError> {
        let Some(captured)=request.captured_context.as_ref() else{return Ok(true);};
        let session=AgentSessionId::from(request.conversation_id.clone());
        let current=self.canonical.session_record(&PrincipalRef {principal_kind:"user".into(),principal_id:owner.into()},&session).await?;
        let opening = if current.metadata.companion.is_none() {
            self.canonical.store().read_events(&session,None,1).await.map_err(agent_session_store_error)?.events
        } else {Vec::new()};
        let scope=super::companion_sessions::canonical_companion_origin_from_opening(owner,&current,&opening)?;
        if !scope.is_some_and(|scope|super::robot_saved_response::robot_playback_scope_matches(
            &scope,&request.companion_id,&request.robot_id)) {return Ok(false);}
        let floor:i64=sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
            .bind(session.as_ref()).fetch_one(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        Ok(current.agent_binding.binding_version==captured.binding_version&&u64::try_from(floor).ok()==Some(captured.context_floor))
    }
    /// A short read lease protects the exact writer-acceptance boundary.
    /// Callers reserve capacity first and send synchronously before dropping
    /// this guard; provider I/O and backpressure never hold the Session fence.
    pub(crate) async fn hold_robot_output_context(&self,owner:&str,request:&nomifun_robot::services::RobotTurnRequest)
        ->Result<Option<tokio::sync::OwnedRwLockReadGuard<()>>,AppError> {
        let guard=self.session_operation_lock(&request.conversation_id).read_owned().await;
        if !self.robot_input_context_matches(owner,request).await? {return Ok(None);}
        Ok(Some(guard))
    }

    /// Recording admission checks scope before an input exists. Playback also
    /// observes the immutable original key once it has been accepted, so an
    /// external cancellation can release a held TTS provider without waiting
    /// for another chunk or selecting a later Turn.
    pub(crate) async fn robot_output_context_matches(&self,owner:&str,request:&nomifun_robot::services::RobotTurnRequest)->Result<bool,AppError> {
        if !self.robot_input_context_matches(owner,request).await? {return Ok(false);}
        let session=AgentSessionId::from(request.conversation_id.clone());
        let principal=PrincipalRef {principal_kind:"user".into(),principal_id:owner.into()};
        let key=crate::robot_wiring::robot_turn_idempotency_key(request);
        let Some(input)=self.canonical.session_input_by_key(&principal,&session,&key).await? else{return Ok(true);};
        if let Some(claim)=self.canonical.store().session_input_claim(&session,&input.event.event_id).await.map_err(agent_session_store_error)? {
            let receipt=self.canonical.store().read_turn_receipt(&session,&claim.operation_id).await.map_err(agent_session_store_error)?;
            return Ok(receipt.status!=nomifun_agent_session::TurnReceiptStatus::Cancelled);
        }
        Ok(!matches!(self.session_turn_delivery_state(owner,session.as_ref(),&key).await?,
            PublicTurnDeliveryState::InputUnavailable {cause:nomifun_conversation::UnavailableInputCause::CancelledByOwner
                |nomifun_conversation::UnavailableInputCause::ContextCleared,..}))
    }

    /// A UI cancel commits directly through the canonical Store, without the
    /// Session operation lock. Reserve SQLite writes as well as the existing
    /// scope lease until synchronous writer acceptance has completed.
    pub(crate) async fn hold_robot_request_playback(&self,owner:&str,request:&nomifun_robot::services::RobotTurnRequest)
        ->Result<Option<sqlx::Transaction<'static,sqlx::Sqlite>>,AppError> {
        let session=AgentSessionId::from(request.conversation_id.clone());
        let principal=PrincipalRef {principal_kind:"user".into(),principal_id:owner.into()};
        let key=crate::robot_wiring::robot_turn_idempotency_key(request);
        let input=self.canonical.session_input_by_key(&principal,&session,&key).await?;
        let claim=if let Some(input)=input.as_ref() {
            let source:nomifun_agent_contracts::AgentInputSource=serde_json::from_value(input.input.0.get("input_source").cloned()
                .ok_or_else(||AppError::Conflict("Robot output lost its original authenticated input".into()))?)
                .map_err(|error|AppError::Conflict(error.to_string()))?;
            if input.input.0.get("content").and_then(Value::as_str)!=Some(request.text.as_str())
                ||!matches!(source,nomifun_agent_contracts::AgentInputSource::AuthenticatedRobot {companion_id,robot_id,connection_id,..}
                    if companion_id==request.companion_id&&robot_id==request.robot_id&&connection_id==request.connection_id) {
                return Err(AppError::Conflict("Robot output differs from its original accepted request".into()));
            }
            let Some(claim)=self.canonical.store().session_input_claim(&session,&input.event.event_id).await.map_err(agent_session_store_error)?
                else{return Ok(None);};
            Some(claim)
        } else {
            // Source-context playback carries no human input. Ordinary
            // utterances must already have their exact admission before any
            // public text or PCM is allowed into the writer.
            if !request.text.is_empty() {return Ok(None);}
            None
        };
        let mut tx=self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(|error|AppError::Internal(error.to_string()))?;
        if let Some(claim)=claim {
            let expected_start=(!claim.steering).then_some(claim.claim_event_id.as_ref());
            let state:Option<String>=sqlx::query_scalar("SELECT state FROM agent_turns WHERE session_id=? AND operation_id=? AND source_message_id=? AND (? IS NULL OR started_event_id=?)")
                .bind(session.as_ref()).bind(claim.operation_id.as_ref()).bind(claim.source_message_id.as_ref())
                .bind(expected_start).bind(expected_start)
                .fetch_optional(&mut *tx).await.map_err(|error|AppError::Internal(error.to_string()))?;
            let Some(state)=state else{return Err(AppError::Conflict("Robot output lost its exact canonical Turn claim".into()));};
            match state.as_str() {
                "cancelled"|"interrupted"=>return Ok(None),
                "accepted"|"running"|"completed"|"failed"=>{},
                other=>return Err(AppError::Conflict(format!("Robot output has unknown canonical Turn state {other}"))),
            }
        }
        Ok(Some(tx))
    }
    pub(crate) async fn record_robot_question_presented(&self,owner:&str,request:&nomifun_robot::services::RobotTurnRequest,
        source:&nomifun_voice_contracts::VoiceSpeechSourceRef,presentation:&str)->Result<(),AppError> {
        let session=AgentSessionId::from(request.conversation_id.clone());
        let _fence=self.session_operation_lock(session.as_ref()).write_owned().await;
        let owned=self.canonical.get(&PrincipalRef {principal_kind:"user".into(),principal_id:owner.into()},&session).await?;
        let cursor=nomifun_agent_contracts::SessionEventCursor {agent_session_id:session.clone(),seq:source.through_seq.saturating_sub(1)};
        let original=self.canonical.store().read_events(&session,Some(&cursor),1).await.map_err(agent_session_store_error)?.events.into_iter().next();
        let Some(original)=original else{return Err(AppError::Conflict("Robot question event disappeared".into()));};
        if original.kind.0!="runtime/progress-recorded"||original.event_id.as_ref()!=source.message_id||original.seq!=source.through_seq||source.revision!=source.through_seq
            ||original.producer_id.as_ref()!="runtime_supervisor"||original.kind_version!=1 {
            return Err(AppError::Conflict("Robot question source changed before presentation".into()));
        }
        let payloads=self.canonical.store().resolved_event_payloads(&session,std::slice::from_ref(&original.event_id)).await.map_err(agent_session_store_error)?;
        let native=payloads.get(original.event_id.as_ref()).and_then(|payload|payload.get("event"))
            .ok_or_else(||AppError::Conflict("Robot question has no typed canonical payload".into()))?;
        if !matches!(serde_json::from_value::<AgentEngineEvent>(native.clone()),Ok(AgentEngineEvent::UserInputRequested {..})) {
            return Err(AppError::Conflict("Robot presentation source is not a formal question".into()));
        }
        let root=original.causation_event_id.as_ref().ok_or_else(||AppError::Conflict("Robot question lost its original input".into()))?;
        if presentation!=format!("robot-question:{}:{}",root.as_ref(),source.through_seq) {return Err(AppError::Conflict("Robot presentation differs from its canonical question".into()));}
        let operation=OperationId::from(original.correlation_id.as_ref().to_owned());
        let receipt=self.canonical.store().read_turn_receipt(&session,&operation).await.map_err(agent_session_store_error)?;
        let started=receipt.started_event.ok_or_else(||AppError::Conflict("Robot question has no original admission".into()))?;
        let starts=self.canonical.store().resolved_event_payloads(&session,std::slice::from_ref(&started.event_id)).await.map_err(agent_session_store_error)?;
        let admitted=starts.get(started.event_id.as_ref()).ok_or_else(||AppError::Conflict("Robot question has no original admission payload".into()))?;
        if canonical_admission_binding_ref(admitted)?.as_ref()!=Some(&AgentHandoffBindingRefV1::from(&owned.session.agent_binding)) {
            return Err(AppError::Conflict("Robot question binding authority was replaced".into()));
        }
        let (_,input)=self.canonical.store().turn_accepted_input(&session,&operation).await.map_err(agent_session_store_error)?;
        let input_source:nomifun_agent_contracts::AgentInputSource=serde_json::from_value(input.0.get("input_source").cloned().ok_or_else(||AppError::Conflict("Robot question lost its authenticated input".into()))?).map_err(|error|AppError::Conflict(error.to_string()))?;
        if !matches!(input_source,nomifun_agent_contracts::AgentInputSource::AuthenticatedRobot {companion_id,robot_id,connection_id,..}
            if companion_id==request.companion_id&&robot_id==request.robot_id&&connection_id==request.connection_id) {return Err(AppError::Forbidden("Robot question belongs to another connection".into()));}
        let revoked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND seq>? AND kind IN ('context/cleared','session/agent-binding-changed'))")
            .bind(session.as_ref()).bind(i64::try_from(original.seq).map_err(|error|AppError::Conflict(error.to_string()))?).fetch_one(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if revoked||owned.session.owner_ref.principal_id!=owner {return Err(AppError::Conflict("Robot question presentation authority was revoked".into()));}
        let payload=json!({"notice_message_id":source.message_id,"notice_seq":source.through_seq,
            "connection_id":request.connection_id,"presentation_id":presentation});
        self.canonical.store().append_event(&nomifun_agent_contracts::SessionEventAppend {agent_session_id:session,
            event_id:format!("robot-question-presented:{}:{}",source.message_id,request.connection_id).into(),producer_id:"session_api".into(),
            idempotency_key:format!("robot-question-playback:{}:{}",source.message_id,request.connection_id).into(),
            semantic_event:nomifun_agent_contracts::SemanticSessionEventDraft {kind:nomifun_agent_contracts::SessionEventKind("robot/question-playback-accepted".into()),
                kind_version:1,correlation_id:source.message_id.clone().into(),causation_event_id:Some(original.event_id),
                payload:nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(StrictJsonValue(payload))}}).await.map_err(agent_session_store_error)?;
        Ok(())
    }

    pub(crate) async fn supplement_robot_session_request(&self,owner:&str,request:&nomifun_robot::services::RobotTurnRequest,
        key:&str,text:&str,correction:bool,input_id:&str)->Result<(),AppError> {
        let session=AgentSessionId::from(request.conversation_id.clone());let principal=PrincipalRef {principal_kind:"user".into(),principal_id:owner.into()};
        let _fence=self.session_operation_lock(session.as_ref()).write_owned().await;
        let input=self.canonical.session_input_by_key(&principal,&session,key).await?
            .ok_or_else(||AppError::Conflict("The original Robot input has no canonical source".into()))?;
        let claim=self.canonical.store().session_input_claim(&session,&input.event.event_id).await.map_err(agent_session_store_error)?
            .ok_or_else(||AppError::Conflict("The original Robot request has not started".into()))?;
        let facts=self.canonical.store().turn_output_facts(&session,&claim.operation_id).await.map_err(agent_session_store_error)?;
        if facts.session.owner_ref!=principal || facts.head.active_turn_id.as_deref()!=Some(claim.operation_id.as_ref()) {
            return Err(AppError::Conflict("The original Robot work has ended; the supplement was not retargeted".into()));
        }
        if let Some(context)=request.captured_context.as_ref()
            && (context.work_operation_id.as_deref().is_some_and(|operation|operation!=claim.operation_id.as_ref())
                ||context.execution_generation.is_some_and(|generation|generation!=facts.execution_generation)) {
            return Err(AppError::Conflict("The recorded work generation was replaced; the supplement was not retargeted".into()));
        }
        let mut value=super::canonical_turn_input(&SendMessageRequest {content:text.into(),files:vec![],inject_skills:vec![],hidden:false,origin:Some("robot".into()),channel_platform:Some("robot".into())});
        value["input_source"]=serde_json::to_value(nomifun_agent_contracts::AgentInputSource::AuthenticatedRobot {
            companion_id:request.companion_id.clone(),robot_id:request.robot_id.clone(),connection_id:request.connection_id.clone(),presented_question_id:None}).map_err(|error|AppError::Internal(error.to_string()))?;
        let control_key=format!("robot-{}:{}:{}",if correction {"correction"} else {"supplement"},key,input_id);
        let receipt=self.canonical.steer_exact_native_turn(&principal,&session,&control_key,&claim.operation_id,value,
            &nomifun_agent_session::NativeTurnMutationFence {binding_version:facts.session.agent_binding.binding_version,execution_generation:facts.execution_generation}).await?;
        self.queue_receipted_steering(&session,&receipt).await?;Ok(())
    }
    pub(crate) async fn robot_question_output(&self, owner:&str, session:&str,key:&str,
        question:&AgentUserInputRequest)->Result<Option<(nomifun_voice_contracts::VoiceSpeechSourceRef,String,String)>,AppError> {
        let session=AgentSessionId::from(session.to_owned());let principal=PrincipalRef {principal_kind:"user".into(),principal_id:owner.into()};
        let mut operation=Self::turn_operation_id(owner,session.as_ref(),key);
        if let Some(input)=self.canonical.session_input_by_key(&principal,&session,key).await? {
            let Some(claim)=self.canonical.store().session_input_claim(&session,&input.event.event_id).await.map_err(agent_session_store_error)? else{return Ok(None);};
            operation=claim.operation_id;
        }
        let facts=self.canonical.store().turn_output_facts(&session,&operation).await.map_err(agent_session_store_error)?;
        if facts.session.owner_ref!=principal{return Err(AppError::Forbidden("Robot question belongs to another owner".into()));}
        let (_,input)=self.canonical.store().turn_accepted_input(&session,&operation).await.map_err(agent_session_store_error)?;
        let receipt=self.canonical.store().read_turn_receipt(&session,&operation).await.map_err(agent_session_store_error)?;
        let started=receipt.started_event.as_ref().ok_or_else(||AppError::Conflict("Robot question has no original admission".into()))?;
        let admitted=facts.event_payloads.get(started.event_id.as_ref()).ok_or_else(||AppError::Conflict("Robot question admission payload is missing".into()))?;
        if canonical_admission_binding_ref(admitted)?.as_ref()!=Some(&AgentHandoffBindingRefV1::from(&facts.session.agent_binding)) {
            return Err(AppError::Conflict("Robot question binding authority was replaced".into()));
        }
        let root=receipt.started_event.as_ref().and_then(|start|facts.event_payloads.get(start.event_id.as_ref()))
            .and_then(|payload|payload.get("source_message_id")).and_then(Value::as_str).ok_or_else(||AppError::Conflict("question has no original root".into()))?;
        let _=input;
        for event in facts.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded"&&event.kind_version==1
            &&event.producer_id.as_ref()=="runtime_supervisor"&&event.correlation_id.as_ref()==operation.as_ref()) {
            let Some(value)=facts.event_payloads.get(event.event_id.as_ref()).and_then(|payload|payload.get("event")) else{continue;};
            if value.get("event").and_then(Value::as_str)!=Some("user_input_requested"){continue;}
            if let AgentEngineEvent::UserInputRequested {request,..}=serde_json::from_value(value.clone()).map_err(|error|AppError::Conflict(error.to_string()))? {
                if &request!=question{continue;}
                return Ok(Some((nomifun_voice_contracts::VoiceSpeechSourceRef {message_id:event.event_id.as_ref().into(),revision:event.seq,through_seq:event.seq},
                    format!("robot-question:{root}:{}",event.seq),request.question)));
            }
        }
        Err(AppError::Conflict("Robot question has no exact canonical interaction event".into()))
    }

    pub(super) async fn direct_robot_question_reply(&self,owner:&str,session:&AgentSessionId,
        request:&nomifun_robot::services::RobotTurnRequest)->Result<Option<nomifun_api_types::AgentQuestionReplyTargetDto>,AppError> {
        let Some(id)=request.presented_question_id.as_deref().and_then(|id|id.strip_prefix("robot-question:")) else{return Ok(None);};
        let (root,sequence)=id.rsplit_once(':').ok_or_else(||AppError::Conflict("Invalid Robot question identity".into()))?;
        let sequence=sequence.parse::<u64>().map_err(|_|AppError::Conflict("Invalid Robot question sequence".into()))?;
        let answered:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND kind='message/user-accepted' AND seq>? AND COALESCE(json_extract(inline_json,'$.hidden'),0)=0)")
            .bind(session.as_ref()).bind(i64::try_from(sequence).map_err(|error|AppError::Conflict(error.to_string()))?).fetch_one(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if answered {return Err(AppError::Conflict("Robot question already has an accepted answer".into()));}
        let operation:Option<String>=sqlx::query_scalar("SELECT operation_id FROM agent_turns WHERE session_id=? AND source_message_id=?")
            .bind(session.as_ref()).bind(root).fetch_optional(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        let operation=operation.ok_or_else(||AppError::Conflict("Robot question source disappeared".into()))?;
        let facts=self.canonical.store().turn_output_facts(session,&operation.clone().into()).await.map_err(agent_session_store_error)?;
        let start=facts.events.iter().find(|event|event.kind.0=="turn/started"&&event.correlation_id.as_ref()==operation)
            .and_then(|event|facts.event_payloads.get(event.event_id.as_ref())).ok_or_else(||AppError::Conflict("Robot reply has no exact original admission".into()))?;
        if canonical_admission_binding_ref(start)?.as_ref()!=Some(&AgentHandoffBindingRefV1::from(&facts.session.agent_binding)) {
            return Err(AppError::Conflict("Robot question binding authority was replaced".into()));
        }
        let revoked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND seq>=? AND kind IN ('context/cleared','session/agent-binding-changed'))")
            .bind(session.as_ref()).bind(i64::try_from(sequence).map_err(|error|AppError::Conflict(error.to_string()))?).fetch_one(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if facts.session.owner_ref.principal_id!=owner ||revoked {
            return Err(AppError::Conflict("Robot question source was revoked".into()));
        }
        let (_,input)=self.canonical.store().turn_accepted_input(session,&operation.into()).await.map_err(agent_session_store_error)?;
        let source:nomifun_agent_contracts::AgentInputSource=serde_json::from_value(input.0.get("input_source").cloned()
            .ok_or_else(||AppError::Conflict("Robot question has no original authenticated input".into()))?).map_err(|error|AppError::Conflict(error.to_string()))?;
        if !matches!(source,nomifun_agent_contracts::AgentInputSource::AuthenticatedRobot {companion_id,robot_id,connection_id,..}
            if companion_id==request.companion_id&&robot_id==request.robot_id&&connection_id==request.connection_id) {
            return Err(AppError::Conflict("Robot question belongs to another connection".into()));
        }
        let accepted:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND kind='robot/question-playback-accepted' AND kind_version=1 AND producer_id='session_api' AND json_extract(inline_json,'$.notice_seq')=? AND json_extract(inline_json,'$.connection_id')=? AND json_extract(inline_json,'$.presentation_id')=?)")
            .bind(session.as_ref()).bind(i64::try_from(sequence).map_err(|error|AppError::Conflict(error.to_string()))?).bind(&request.connection_id).bind(request.presented_question_id.as_deref()).fetch_one(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if !accepted {return Err(AppError::Conflict("Robot question was not accepted by its original connection".into()));}
        Ok(Some(nomifun_api_types::AgentQuestionReplyTargetDto {turn_id:root.into(),question_seq:sequence}))
    }
    /// Subscribe before the durable read. Events are wake hints only; a lagged
    /// receiver is repaired by rereading the original typed fact chain.
    pub(crate) async fn robot_output_wake(&self, owner: &str, session: &str, key: &str)
        -> Result<Option<broadcast::Receiver<AgentStreamEvent>>,AppError> {
        let deadline=tokio::time::Instant::now()+Duration::from_secs(30);
        loop {
            if let Some(handle)=self.runtime_sessions.get_runtime(session) {
                return Ok(Some(handle.subscribe()));
            }
            // An unclaimed input can finish without ever opening a Runtime.
            // Its existing canonical lifecycle must end registration waiting
            // too; the caller reads the exact cause before emitting anything.
            if !matches!(self.session_turn_delivery_state(owner,session,key).await?,
                PublicTurnDeliveryState::Accepted {..}) {
                return Ok(None);
            }
            if tokio::time::Instant::now()>=deadline {return Ok(None);}
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    pub(crate) async fn robot_public_output(&self, owner: &str, session: &str, key: &str,
        after_seq: u64, delivered_bytes: &mut BTreeMap<String, usize>) -> Result<RobotPublicOutput, AppError> {
        let session=AgentSessionId::from(session.to_owned());
        let principal=PrincipalRef {principal_kind:"user".into(),principal_id:owner.into()};
        let mut operation=Self::turn_operation_id(owner, session.as_ref(),key);
        if let Some(input)=self.canonical.session_input_by_key(&principal,&session,key).await? {
            let Some(claim)=self.canonical.store().session_input_claim(&session,&input.event.event_id).await.map_err(agent_session_store_error)?
                else {return Ok(RobotPublicOutput {segments:Vec::new(),through_seq:after_seq});};
            operation=claim.operation_id;
        }
        let facts=self.canonical.store().turn_output_facts(&session,&operation).await.map_err(agent_session_store_error)?;
        if facts.session.owner_ref!=principal {return Err(AppError::Forbidden("Robot output belongs to another owner".into()));}
        let receipt=self.canonical.store().read_turn_receipt(&session,&operation).await.map_err(agent_session_store_error)?;
        let Some(start)=receipt.started_event.as_ref() else {return Ok(RobotPublicOutput {segments:Vec::new(),through_seq:after_seq});};
        let root=facts.event_payloads.get(start.event_id.as_ref()).and_then(|payload|payload.get("source_message_id"))
            .and_then(Value::as_str).ok_or_else(||AppError::Conflict("Robot output has no accepted source".into()))?;
        let floor:i64=sqlx::query_scalar("SELECT COALESCE(MAX(seq),0) FROM agent_events WHERE session_id=? AND kind='context/cleared'")
            .bind(session.as_ref()).fetch_one(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        let floor=u64::try_from(floor).map_err(|error|AppError::Conflict(error.to_string()))?;
        if start.seq<=floor {return Err(AppError::Conflict("Robot output source was cleared".into()));}
        let rebound:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_events WHERE session_id=? AND kind='session/agent-binding-changed' AND seq>?)")
            .bind(session.as_ref()).bind(i64::try_from(start.seq).map_err(|error|AppError::Conflict(error.to_string()))?).fetch_one(&self.pool).await.map_err(|error|AppError::Internal(error.to_string()))?;
        if rebound {return Err(AppError::Conflict("Robot output source binding was replaced".into()));}
        let admitted=facts.event_payloads.get(start.event_id.as_ref()).ok_or_else(||AppError::Conflict("Robot output admission payload is missing".into()))?;
        if canonical_admission_binding_ref(admitted)?.as_ref()!=Some(&AgentHandoffBindingRefV1::from(&facts.session.agent_binding)) {
            return Err(AppError::Conflict("Robot output binding was replaced".into()));
        }
        committed_prefixes(&facts,&operation,root,after_seq,delivered_bytes)
    }
}

fn committed_prefixes(facts:&nomifun_agent_session::ChatCausalityFacts, operation:&OperationId, root:&str,
    after_seq:u64, delivered_bytes:&mut BTreeMap<String,usize>) ->Result<RobotPublicOutput,AppError> {
    let invalid=|detail:&str|AppError::Conflict(format!("Robot public output: {detail}"));
    // Final answers remain proposals until the Host commits the actual
    // successful terminal. In particular CompletionDelivered can still be
    // narrowed to TurnPaused during cleanup. A final source is selected only
    // by the original typed canonical outcome, never by a runtime stop token.
    let final_sources=if let Some(terminal)=facts.events.iter().find(|event|event.kind.0=="turn/completed"&&event.kind_version==1
        &&event.correlation_id.as_ref()==operation.as_ref()) {
        let started=facts.events.iter().find(|event|event.kind.0=="turn/started"&&event.kind_version==1&&event.correlation_id.as_ref()==operation.as_ref())
            .ok_or_else(||invalid("completed public source has no original Turn"))?;
        let receipt=nomifun_agent_session::TurnReceipt {agent_session_id:facts.session.agent_session_id.clone(),operation_id:operation.clone(),
            status:nomifun_agent_session::TurnReceiptStatus::Completed,started_event:Some(started.clone()),terminal_event:Some(terminal.clone())};
        let delivery=nomifun_agent_execution::canonical_turn_delivery(facts,&receipt,true).map_err(|error|AppError::Conflict(error.to_string()))?;
        match delivery.turn_outcome {
            Some(nomifun_agent_contracts::AgentTurnOutcome::Answered {message_ids}) if delivery.result_ok==Some(true)
                =>Some((message_ids.into_iter().collect::<BTreeSet<_>>(),terminal.seq)),
            _=>None,
        }
    }else{None};
    let mut texts=BTreeMap::<String,(String,u64,String)>::new();
    let mut steps=BTreeSet::new();let mut withdrawn=BTreeSet::new();let mut committed=BTreeSet::new();
    let mut segments=Vec::new();let mut through_seq=after_seq;
    let mut ordered=facts.events.iter().collect::<Vec<_>>();ordered.sort_by_key(|event|event.seq);
    for event in ordered {
        through_seq=through_seq.max(event.seq);
        if event.producer_id.as_ref()!="runtime_supervisor" {continue;}
        if event.kind.0=="message/content-part" {
            if event.kind_version!=1 {return Err(invalid("unsupported public part version"));}
            let payload=facts.event_payloads.get(event.event_id.as_ref()).ok_or_else(||invalid("missing public part"))?;
            if payload.get("turn_id").and_then(Value::as_str)!=Some(root) {continue;}
            let text=payload.get("content").and_then(Value::as_str).ok_or_else(||invalid("public part has no text"))?;
            let current=texts.entry(event.correlation_id.as_ref().into()).or_default();
            if current.0.len().saturating_add(text.len())>8*1024*1024 {return Err(invalid("public source exceeds its budget"));}
            current.0.push_str(text);current.1+=1;current.2=event.event_id.as_ref().into();continue;
        }
        if event.kind.0=="message/completed" {
            let Some((_,terminal_seq))=final_sources.as_ref().filter(|(sources,_)|sources.contains(event.correlation_id.as_ref())) else{continue;};
            if *terminal_seq<=after_seq {continue;}
            let complete=facts.event_payloads.get(event.event_id.as_ref()).ok_or_else(||invalid("final source has no completion proof"))?;
            let Some((text,count,last_event))=texts.get(event.correlation_id.as_ref()) else{continue;};
            if complete.get("part_count").and_then(Value::as_u64)!=Some(*count)
                ||complete.get("content_digest").and_then(Value::as_str)!=Some(digest_bytes(text.as_bytes()).as_ref())
                ||event.causation_event_id.as_ref().map(|id|id.as_ref())!=Some(last_event.as_str()) {
                return Err(invalid("final publication differs from its exact immutable parts"));
            }
            let id=event.correlation_id.as_ref().to_owned();let prior=delivered_bytes.get(&id).copied().unwrap_or(0);
            if prior>text.len()||!text.is_char_boundary(prior){return Err(invalid("final public prefix moved backwards"));}
            if prior<text.len() {
                segments.push((nomifun_voice_contracts::VoiceSpeechSourceRef {message_id:event.event_id.as_ref().into(),revision:event.seq,through_seq:*terminal_seq},text[prior..].to_owned()));
                delivered_bytes.insert(id,text.len());
            }
            continue;
        }
        if event.kind.0!="runtime/progress-recorded" || event.correlation_id.as_ref()!=operation.as_ref() {continue;}
        if event.kind_version!=1 || event.causation_event_id.as_ref().map(|id|id.as_ref())!=Some(root) {return Err(invalid("runtime source identity differs"));}
        let payload=facts.event_payloads.get(event.event_id.as_ref()).and_then(|payload|payload.get("event"))
            .ok_or_else(||invalid("runtime publication has no payload"))?;
        if !matches!(payload.get("event").and_then(Value::as_str),Some("model_step_started"|"public_output_committed"|"voice_model_step_superseded"|"model_response_rejected"|"model_output_truncated"|"delivery_review_superseded")) {continue;}
        let native:AgentEngineEvent=serde_json::from_value(payload.clone()).map_err(|_|invalid("invalid typed publication event"))?;
        match native {
            AgentEngineEvent::ModelStepStarted {step,operation_id,..}=>{
                if operation_id.as_ref()!=format!("{}:model:{step}",operation.as_ref()) || !steps.insert(step) {return Err(invalid("model publication identity differs"));}
            },
            AgentEngineEvent::VoiceModelStepSuperseded {step,..}|AgentEngineEvent::ModelResponseRejected {step,..}
            |AgentEngineEvent::ModelOutputTruncated {step,..}|AgentEngineEvent::DeliveryReviewSuperseded {step,..}=>{
                if committed.contains(&step) {return Err(invalid("an irrevocable source was subsequently withdrawn"));}withdrawn.insert(step);
            },
            AgentEngineEvent::PublicOutputCommitted {step,source}=>{
                if !steps.contains(&step)||withdrawn.contains(&step) {return Err(invalid("publication has no live exact model source"));}
                committed.insert(step);
                if event.seq<=after_seq {continue;}
                let id=nomifun_agent_contracts::canonical_assistant_step_message_id(root,step).map_err(|_|invalid("invalid accepted root"))?;
                let Some((text,count,last_event))=texts.get(&id) else {if source.is_some(){return Err(invalid("public proof has no original parts"));}continue;};
                let source=source.ok_or_else(||invalid("public prefix has no bound integrity proof"))?;
                if source.message_id!=id||source.part_count!=*count||source.through_event_id.as_ref()!=last_event
                    ||source.content_digest!=digest_bytes(text.as_bytes())||source.binding_version!=facts.session.agent_binding.binding_version {
                    return Err(invalid("public prefix differs from its exact canonical proof"));
                }
                let prior=delivered_bytes.get(&id).copied().unwrap_or(0);
                if prior>text.len() || !text.is_char_boundary(prior) {return Err(invalid("published prefix moved backwards"));}
                if prior<text.len() {
                    // Each committed prefix is its own immutable source.
                    // The proof above binds it to the original model message;
                    // subsequent same-step report text cannot change its ID.
                    segments.push((nomifun_voice_contracts::VoiceSpeechSourceRef {message_id:event.event_id.as_ref().into(),revision:event.seq,through_seq:event.seq},text[prior..].to_owned()));
                    delivered_bytes.insert(id,text.len());
                }
            },_=>{}
        }
    }
    Ok(RobotPublicOutput {segments,through_seq})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::engine_journal::{EngineJournalWrite,EngineTurnJournal};
    async fn record(journal:&EngineTurnJournal,event:AgentEngineEvent) {
        journal.append(serde_json::to_string(&event).unwrap(),None,EngineJournalWrite::Progress).await.unwrap();
    }
    async fn facts(pool:&nomifun_db::SqlitePool)->(nomifun_agent_session::ChatCausalityFacts,String) {
        let store=nomifun_agent_session::AgentSessionStore::from_pool(pool.clone()).await.unwrap();
        let session=AgentSessionId::from("0190f5fe-7c00-7a00-8000-000000000002");
        let facts=store.turn_output_facts(&session,&"turn".into()).await.unwrap();
        let (input,_)=store.turn_accepted_input(&session,&"turn".into()).await.unwrap();
        (facts,input.event_id.as_ref().into())
    }
    #[tokio::test]
    async fn public_prefix_is_delivered_before_turn_completion_and_is_not_repeated() {
        let (journal,pool)=crate::router::engine_journal::test_fixture().await;
        record(&journal,AgentEngineEvent::ModelStepStarted {step:1,operation_id:"turn:model:1".into(),correction_source:None}).await;
        record(&journal,AgentEngineEvent::OutputTextDelta {step:1,text:"已受理，正在查询".into(),source:None}).await;
        record(&journal,AgentEngineEvent::ReasoningDelta {step:1,text:"PRIVATE_REASONING".into()}).await;
        let (raw,root)=facts(&pool).await;let mut delivered=BTreeMap::new();
        assert!(committed_prefixes(&raw,&"turn".into(),&root,0,&mut delivered).unwrap().segments.is_empty());
        record(&journal,AgentEngineEvent::PublicOutputCommitted {step:1,source:None}).await;
        let (committed,_)=facts(&pool).await;
        assert_eq!(committed.head.status,"running");
        let first=committed_prefixes(&committed,&"turn".into(),&root,0,&mut delivered).unwrap();
        assert_eq!(first.segments.len(),1);assert_eq!(first.segments[0].1,"已受理，正在查询");
        assert!(first.segments[0].0.revision>0);
        record(&journal,AgentEngineEvent::PublicOutputCommitted {step:1,source:None}).await;
        record(&journal,AgentEngineEvent::OutputTextDelta {step:1,text:"。已获取真实结果。".into(),source:None}).await;
        record(&journal,AgentEngineEvent::PublicOutputCommitted {step:1,source:None}).await;
        let (later,_)=facts(&pool).await;
        let suffix=committed_prefixes(&later,&"turn".into(),&root,first.through_seq,&mut delivered).unwrap();
        assert_eq!(suffix.segments.len(),1);assert_eq!(suffix.segments[0].1,"。已获取真实结果。");
        assert_ne!(first.segments[0].0.message_id,suffix.segments[0].0.message_id,"each formal prefix owns an independent immutable source");
    }
    #[tokio::test]
    async fn missing_or_corrupt_integrity_proof_never_authorizes_speech() {
        let (journal,pool)=crate::router::engine_journal::test_fixture().await;
        record(&journal,AgentEngineEvent::ModelStepStarted {step:1,operation_id:"turn:model:1".into(),correction_source:None}).await;
        record(&journal,AgentEngineEvent::OutputTextDelta {step:1,text:"actual public result".into(),source:None}).await;
        record(&journal,AgentEngineEvent::PublicOutputCommitted {step:1,source:None}).await;
        let (original,root)=facts(&pool).await;
        for field in ["content_digest","part_count","through_event_id","message_id","binding_version"] {
            let mut changed=original.clone();
            let value=changed.event_payloads.values_mut().find(|payload|payload.pointer("/event/event").and_then(Value::as_str)==Some("public_output_committed")).unwrap();
            value["event"]["source"][field]=if matches!(field,"part_count"|"binding_version") {json!(999)}else{json!("changed")};
            assert!(committed_prefixes(&changed,&"turn".into(),&root,0,&mut BTreeMap::new()).is_err(),"{field}");
        }
        let mut missing=original;
        missing.event_payloads.values_mut().find(|payload|payload.pointer("/event/event").and_then(Value::as_str)==Some("public_output_committed")).unwrap()["event"].as_object_mut().unwrap().remove("source");
        assert!(committed_prefixes(&missing,&"turn".into(),&root,0,&mut BTreeMap::new()).is_err());
    }
    #[tokio::test]
    async fn withdrawn_and_future_steps_are_rejected_before_persistence() {
        let (journal,pool)=crate::router::engine_journal::test_fixture().await;
        record(&journal,AgentEngineEvent::ModelStepStarted {step:1,operation_id:"turn:model:1".into(),correction_source:None}).await;
        record(&journal,AgentEngineEvent::OutputTextDelta {step:1,text:"retractable draft".into(),source:None}).await;
        record(&journal,AgentEngineEvent::ModelResponseRejected {step:1,discarded_tool_call_ids:vec![],continuation:true,tool_hint:None,rejection:None,feedback:None}).await;
        let (before,_)=facts(&pool).await;
        for step in [1,2] {
            assert!(journal.append(serde_json::to_string(&AgentEngineEvent::PublicOutputCommitted {step,source:None}).unwrap(),None,EngineJournalWrite::Progress).await.is_err());
        }
        let (after,root)=facts(&pool).await;assert_eq!(before.events.len(),after.events.len());
        assert!(committed_prefixes(&after,&"turn".into(),&root,0,&mut BTreeMap::new()).unwrap().segments.is_empty());
    }
    #[tokio::test]
    async fn a_final_answer_requires_actual_host_completion_and_a_pause_cannot_publish_it() {
        for paused in [true,false] {
            let (journal,pool)=crate::router::engine_journal::test_fixture().await;
            record(&journal,AgentEngineEvent::ModelStepStarted {step:1,operation_id:"turn:model:1".into(),correction_source:None}).await;
            record(&journal,AgentEngineEvent::OutputTextDelta {step:1,text:"最终答案".into(),source:None}).await;
            let (proposal,root)=facts(&pool).await;
            assert!(committed_prefixes(&proposal,&"turn".into(),&root,0,&mut BTreeMap::new()).unwrap().segments.is_empty());
            journal.append(json!({"event":"host_cleanup_proven"}).to_string(),None,EngineJournalWrite::Cleanup).await.unwrap();
            let terminal=if paused {AgentEngineEvent::TurnPaused {model_steps:1,reason:"HOST_CLEANUP_UNPROVEN".into()}}
                else {AgentEngineEvent::TurnCompleted {model_steps:1,finish_reason:nomifun_chat_model_broker::ChatFinishReason::Completed}};
            journal.append(serde_json::to_string(&terminal).unwrap(),None,EngineJournalWrite::Terminal).await.unwrap();
            let (settled,_)=facts(&pool).await;
            let output=committed_prefixes(&settled,&"turn".into(),&root,0,&mut BTreeMap::new()).unwrap();
            if paused {assert!(output.segments.is_empty(),"a Host-narrowed proposal is never a final spoken result");}
            else {
                assert_eq!(output.segments.len(),1);assert_eq!(output.segments[0].1,"最终答案");
                assert!(output.segments[0].0.through_seq>output.segments[0].0.revision,"final source includes its actual Host terminal permission");
            }
        }
    }
}
