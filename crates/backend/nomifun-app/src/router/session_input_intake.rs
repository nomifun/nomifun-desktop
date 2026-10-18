//! Ordinary input is durable before execution is selected. The existing
//! Session gate schedules work; Runtime safe boundaries claim supplements.
use super::*;
use nomifun_agent_contracts::EventId;

fn accepted_delivery(message_id:String,replayed:bool)->IdempotentMessageDelivery {
    IdempotentMessageDelivery {message_id,replayed,completed:false,result_ok:None,result_text:None,result_error:None,
        result_error_code:None,result_error_retryable:None,user_input_request:None,turn_outcome:None}
}

fn original_native_events(facts:&nomifun_agent_session::ChatCausalityFacts,operation:&OperationId)->Result<Vec<(u64,AgentEngineEvent)>,AppError> {
    let mut events=Vec::new();
    for event in facts.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded"&&event.correlation_id.as_ref()==operation.as_ref()) {
        let value=facts.event_payloads.get(event.event_id.as_ref()).and_then(|payload|payload.get("event"))
            .ok_or_else(||AppError::Internal("claimed input history lost a native payload".into()))?;
        if value.get("event").and_then(Value::as_str).is_some_and(|kind|kind.starts_with("host_")) {continue;}
        events.push((event.seq,serde_json::from_value(value.clone()).map_err(|error|AppError::Internal(error.to_string()))?));
    }
    Ok(events)
}

fn original_pending_question(events:&[(u64,AgentEngineEvent)],source:&EventId)
    ->Result<Option<nomifun_ai_agent::protocol::events::InputQuestionEventData>,AppError> {
    let typed=events.iter().map(|(_,event)|event.clone()).collect::<Vec<_>>();
    let Some(request)=nomifun_agent_runtime::recorded_pending_user_input(&typed).map_err(|error|AppError::Internal(error.to_string()))? else {return Ok(None)};
    let seq=events.iter().rev().find_map(|(seq,event)|matches!(event,AgentEngineEvent::UserInputRequested {request:original,..} if original==&request).then_some(*seq))
        .ok_or_else(||AppError::Internal("pending question lost its original event sequence".into()))?;
    Ok(Some(nomifun_ai_agent::protocol::events::InputQuestionEventData {request,target:nomifun_api_types::AgentQuestionReplyTargetDto {
        turn_id:source.as_ref().into(),question_seq:seq}}))
}

impl NomiCoreSessionOwner {
    pub(in crate::router) async fn publish_session_input_delivery(&self,owner_id:&str,session:&AgentSessionId,input:&EventId)->Result<(),AppError> {
        let original=self.canonical.store().session_input(session,input).await.map_err(agent_session_store_error)?;
        let mut content=original.input.0;
        content["input_delivery"]=self.canonical.store().session_input_delivery(session,input).await.map_err(agent_session_store_error)?;
        let created=self.canonical.store().session_created_at(session).await.map_err(agent_session_store_error)?;
        self.user_events.send_to_user(owner_id,Self::canonical_accepted_input_wire_event(session,created,&original.event,&content)?);
        Ok(())
    }

    pub(super) async fn observe_session_input(&self,owner_id:&str,session_id:&str,input_event_id:&str)
        ->Result<nomifun_channel::ChannelInputObservation,AppError> {
        use nomifun_channel::ChannelInputObservation;
        let session=AgentSessionId::from(session_id.to_owned());let principal=PrincipalRef {principal_kind:"user".into(),principal_id:owner_id.into()};
        self.canonical.session_record(&principal,&session).await?;
        let input=self.canonical.store().session_input(&session,&input_event_id.into()).await.map_err(agent_session_store_error)?;
        let Some(claim)=self.canonical.store().session_input_claim(&session,&input.event.event_id).await.map_err(agent_session_store_error)? else {
            let state=self.canonical.store().session_input_delivery(&session,&input.event.event_id).await.map_err(agent_session_store_error)?;
            return Ok(match state["state"].as_str() {
                Some("blocked")=>ChannelInputObservation::Blocked {reason:state["reason"].as_str().unwrap_or("INPUT_UNAVAILABLE").into()},
                Some("discarded")=>ChannelInputObservation::Discarded {reason:"CONTEXT_CLEARED".into()},
                _=>ChannelInputObservation::Pending,
            });
        };
        if claim.steering {
            let state=self.canonical.store().session_input_delivery(&session,&input.event.event_id).await.map_err(agent_session_store_error)?;
            match state["state"].as_str() {
                Some("pending")=>return Ok(ChannelInputObservation::Pending),
                Some("blocked")=>return Ok(ChannelInputObservation::Blocked {reason:state["reason"].as_str().unwrap_or("INPUT_UNAVAILABLE").into()}),
                _=>{},
            }
        }
        // Input observation may begin after the Turn has emitted a prefix.
        // Channels therefore deliver its exact committed question/terminal
        // through one canonical path, never a suffix from late subscription.
        let state=self.canonical_delivery_state_for_operation(owner_id,&session,&claim.operation_id,true).await?;
        let mut question=None;let mut terminal_events=Vec::new();
        let delivery=match state {
            PublicTurnDeliveryState::Completed(delivery)=>{
                let facts=self.canonical.store().turn_output_facts(&session,&claim.operation_id).await.map_err(agent_session_store_error)?;
                let native=original_native_events(&facts,&claim.operation_id)?;
                if delivery.user_input_request.is_some() {question=original_pending_question(&native,&claim.source_message_id)?;}
                if delivery.user_input_request.as_ref().is_some_and(|request|question.as_ref().is_none_or(|question|&question.request!=request)) {
                    return Err(AppError::Internal("closed Turn question differs from its exact public handoff".into()));
                }
                terminal_events=nomifun_ai_agent::project_canonical_tool_delivery(&native).map_err(|error|AppError::Internal(error.to_string()))?;
                delivery
            },
            PublicTurnDeliveryState::Accepted {message_id}=>{
                let facts=self.canonical.store().turn_output_facts(&session,&claim.operation_id).await.map_err(agent_session_store_error)?;
                question=original_pending_question(&original_native_events(&facts,&claim.operation_id)?,&claim.source_message_id)?;
                accepted_delivery(message_id,true)
            },
            PublicTurnDeliveryState::Missing=>return Err(AppError::Internal("input claim lost its exact original Turn".into())),
        };
        Ok(ChannelInputObservation::Claimed {turn_id:claim.source_message_id.as_ref().into(),
            delivery:nomifun_channel::ChannelTurnDelivery {delivery:channel_delivery_from_conversation(delivery),events:None},question,terminal_events})
    }

    pub(super) async fn accept_ordinary_session_input(&self,owner_id:&str,session:&AgentSessionId,key:&str,
        request:SendMessageRequest,reply_to:Option<nomifun_api_types::AgentQuestionReplyTargetDto>)
        ->Result<IdempotentMessageDelivery,AppError> {
        self.accept_session_input_with_source(owner_id,session,key,request,reply_to,None,false).await
    }

    pub(super) async fn accept_channel_session_input(&self,owner_id:&str,session:&AgentSessionId,key:&str,
        request:SendMessageRequest,source:nomifun_agent_contracts::ChannelInputSource,
        reply_to:Option<nomifun_api_types::AgentQuestionReplyTargetDto>)->Result<IdempotentMessageDelivery,AppError> {
        source.validate().map_err(AppError::BadRequest)?;
        self.accept_session_input_with_source(owner_id,session,key,request,reply_to,Some(source),false).await
    }

    async fn accept_session_input_with_source(&self,owner_id:&str,session:&AgentSessionId,key:&str,
        request:SendMessageRequest,reply_to:Option<nomifun_api_types::AgentQuestionReplyTargetDto>,
        source:Option<nomifun_agent_contracts::ChannelInputSource>,initial_only:bool)->Result<IdempotentMessageDelivery,AppError> {
        Self::validate_ordinary_message_origin(&request)?;
        let mut input=canonical_turn_input(&request);
        // These are permanent envelope checks, not a current Turn's budget or
        // a claim that the selected Skill/file is presently available.
        if request.content.trim().is_empty() || request.content.len()>16*1024 {
            return Err(AppError::BadRequest("Session input requires text within 16 KiB".into()));
        }
        super::super::runtime_attachments::references(&input)?;
        super::super::runtime_attachments::selected_skills(&input)?;
        if let Some(target)=reply_to {input["reply_to"]=serde_json::to_value(target).map_err(|error|AppError::Internal(error.to_string()))?;}
        if let Some(source)=source {input["channel_source"]=serde_json::to_value(source).map_err(|error|AppError::Internal(error.to_string()))?;}
        let host=self.native_engines.get().and_then(std::sync::Weak::upgrade)
            .ok_or_else(||AppError::Internal("Session input owner is not installed".into()))?.session_owner()?;
        let principal=PrincipalRef {principal_kind:"user".into(),principal_id:owner_id.into()};
        let owner=owner_id.to_owned();let session=session.clone();let key=key.to_owned();
        let (send,receive)=tokio::sync::oneshot::channel();
        // The existing application task owner retains acceptance and its wake
        // even when the HTTP caller disappears after commit but before ACK.
        let tasks=self.background_tasks.clone();
        if !tasks.spawn(Box::pin(async move {
            let outcome=async {
                let _gate=host.session_operation_lock(session.as_ref()).read_owned().await;
                if let Some(original)=host.canonical.session_input_by_key(&principal,&session,&key).await? {
                    if original.input.0!=input {return Err(AppError::Conflict("Session input key was reused with different content".into()));}
                    return Ok((original.event.event_id,true));
                }
                host.require_ordinary_owner_input(&principal,&session).await?;
                let saved=if initial_only {host.canonical.accept_initial_session_input(&principal,&session,&key,input).await?}
                    else {host.canonical.accept_session_input(&principal,&session,&key,input).await?};
                let ack=saved.ack.ok_or_else(||AppError::Internal("accepted input has no canonical acknowledgement".into()))?;
                Ok::<_,AppError>((ack.event_id,saved.duplicate))
            }.await;
            match outcome {
                Ok((input,replayed))=>{
                    if !replayed {let _=host.publish_session_input_delivery(&owner,&session,&input).await;}
                    let _=send.send(Ok(accepted_delivery(input.as_ref().to_owned(),replayed)));
                    // Replaying an already consumed input only confirms its
                    // original receipt; it cannot select a successor Turn.
                    match host.canonical.store().session_input_claim(&session,&input).await {
                        Ok(None)=>host.schedule_session_input_drain(owner,session,replayed.then_some(input)),
                        Ok(Some(_))=>{},Err(error)=>tracing::warn!(%error,"accepted input claim observation unavailable"),
                    }
                },
                Err(error)=>{let _=send.send(Err(error));},
            }
        })) {return Err(AppError::Conflict("Session input intake is shutting down".into()));}
        receive.await.map_err(|_|AppError::Internal("Session input acknowledgement is unknown; retry its original key".into()))?
    }

    pub(super) fn schedule_session_input_drain(&self,owner_id:String,session:AgentSessionId,retry_input:Option<EventId>) {
        let Some(owner)=self.native_engines.get().and_then(std::sync::Weak::upgrade).and_then(|host|host.session_owner().ok()) else {return;};
        let task=async move {
            if let Err(error)=owner.drain_session_inputs(&owner_id,&session,retry_input).await {
                tracing::warn!(code=error.error_code(),"accepted Session input remains pending; scheduling did not acquire execution");
            }
        };
        if !self.background_tasks.spawn(Box::pin(task)) {tracing::warn!("accepted Session input wake retained for the next Session owner startup");}
    }

    async fn drain_session_inputs(&self,owner_id:&str,session:&AgentSessionId,retry_input:Option<EventId>)->Result<(),AppError> {
        let _gate=self.session_operation_lock(session.as_ref()).write_owned().await;
        let principal=PrincipalRef {principal_kind:"user".into(),principal_id:owner_id.into()};
        if self.require_ordinary_owner_input(&principal,session).await.is_err() {return Ok(());}
        let head=self.canonical.store().head(session).await.map_err(agent_session_store_error)?;
        if head.status=="running" {
            if let Some(runtime)=self.runtime_sessions.get_runtime(session.as_ref()) {
                runtime.session_input_changed(retry_input.map(|id|id.as_ref().to_owned())).await?;
            }
            return Ok(());
        }
        // A technical pause and unknown Effects retain all information, but
        // never become an implicit native resume or new execution grant.
        if head.status!="ready" || head.active_turn_id.is_some()
            || self.canonical.store().has_unsettled_effects(session).await.map_err(agent_session_store_error)?
            || self.runtime_sessions.active_turn_generation(session.as_ref()).is_some() {return Ok(());}
        let inputs=self.canonical.store().pending_session_inputs(session,retry_input.as_ref()).await.map_err(agent_session_store_error)?;
        for original in inputs {
            let mut content=original.input.0.clone();if let Some(content)=content.as_object_mut() {content.remove("reply_to");content.remove("channel_source");}
            let request:SendMessageRequest=serde_json::from_value(content).map_err(|error|AppError::Internal(error.to_string()))?;
            let operation=OperationId::from(format!("turn:session-input:{}",original.event.event_id.as_ref()));
            let preparation=async {
                let admitted=self.canonical_turn_input_with_admission(owner_id,session,&operation,&request).await?;
                let projection=self.canonical_conversation_projection(owner_id,session).await?
                    .ok_or_else(||AppError::NotFound("Session input target disappeared".into()))?;
                let options=self.runtime_options_for_projection(owner_id,session,projection).await?;
                let runtime=self.runtime_sessions.get_or_create_runtime_for_preparation(session.as_ref(),tokio_util::sync::CancellationToken::new(),options).await?;
                runtime.validate_session_input(request.files.clone(),request.inject_skills.clone()).await?;
                Ok::<_,AppError>(admitted)
            }.await;
            let admitted=match preparation {
                Ok(admitted)=>admitted,
                Err(_)=>{
                    self.canonical.store().mark_session_input_blocked(session,&original.event.event_id,
                        nomifun_agent_session::SessionInputBlockedReason::ContextPreparationFailed).await.map_err(agent_session_store_error)?;
                    let _=self.publish_session_input_delivery(owner_id,session,&original.event.event_id).await;
                    continue;
                },
            };
            let admission=admitted.get("admission").cloned().ok_or_else(||AppError::Internal("new Turn lost its exact execution admission".into()))?;
            let claimed=self.canonical.store().claim_session_input_as_turn(&principal,session,&original.event.event_id,&operation,StrictJsonValue(admission))
                .await.map_err(agent_session_store_error)?;
            match self.deliver_claimed_turn(owner_id,session,request,nomifun_conversation::AgentTurnReceipt {
                operation_id:operation.clone(),cursor:claimed.cursor,duplicate:false}).await {
                Ok(_)=>break,
                Err(error)=>{
                    let receipt=self.canonical.store().read_turn_receipt(session,&operation).await.map_err(agent_session_store_error)?;
                    let head=self.canonical.store().head(session).await.map_err(agent_session_store_error)?;
                    if receipt.status==nomifun_agent_session::TurnReceiptStatus::Running || head.status!="ready" || head.active_turn_id.is_some()
                        || self.runtime_sessions.active_turn_generation(session.as_ref()).is_some()
                        || self.canonical.store().has_unsettled_effects(session).await.map_err(agent_session_store_error)? {return Err(error);}
                    // This exact undispatched claim is now terminal. Continue
                    // with other saved intents; never retry/retarget its body.
                },
            }
        }
        Ok(())
    }
}

pub(super) async fn accept_message(State(state):State<NomiCoreAgentApiState>,Extension(owner):Extension<AuthenticatedOwner>,
    Path(id):Path<String>,headers:HeaderMap,Json(request):Json<nomifun_api_types::AcceptAgentSessionMessageRequestDto>)
    ->Result<Json<ApiResponse<nomifun_api_types::AcceptAgentSessionMessageResponseDto>>,NomiCoreApiError> {
    let initial_only=initial_delivery_requested(&headers)?;
    let session=parse_agent_session_id(&id)?;let key=canonical_nonempty(&request.idempotency_key,"idempotency_key")?;
    let input=bounded_turn_input(request.input)?;
    let delivery=state.session_owner.accept_session_input_with_source(owner.as_ref(),&session,&key,input,None,None,initial_only).await?;
    let original=state.session_owner.canonical.session_input_by_key(&authenticated_principal(&owner),&session,&key).await?
        .ok_or_else(||AppError::Internal("accepted input acknowledgement could not be re-read".into()))?;
    Ok(Json(ApiResponse::ok(nomifun_api_types::AcceptAgentSessionMessageResponseDto {agent_session_id:id,
        message_id:delivery.message_id,cursor:session_cursor(&session,original.event.seq),replayed:delivery.replayed,status:"accepted".into()})))
}
