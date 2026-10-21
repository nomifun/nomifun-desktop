//! Ordinary input is durable before execution is selected. The existing
//! Session gate schedules work; Runtime safe boundaries claim supplements.
use super::*;

enum SessionInputIntakePolicy {
    Domain,
    Composer(Option<nomifun_agent_contracts::SessionInputDeliveryIntent>),
}
use nomifun_agent_contracts::EventId;

/// Cancellation transport for the original input-drain owner. Durable input
/// state remains solely in the canonical Event chain; this owns no queue or
/// retry decision, and never acquires a Runtime when queried or stopped.
#[derive(Default)]
pub(super) struct InputPreparationControls {
    slots:std::sync::Mutex<std::collections::HashMap<String,InputPreparation>>,
}
struct InputPreparation {nonce:Uuid,input:EventId,cancel:tokio_util::sync::CancellationToken,finished:tokio_util::sync::CancellationToken}
struct InputPreparationGuard<'a> {owner:&'a InputPreparationControls,session:String,nonce:Uuid,cancel:tokio_util::sync::CancellationToken,finished:tokio_util::sync::CancellationToken}
impl InputPreparationControls {
    fn observations(&self,session:&AgentSessionId)->Result<Vec<(EventId,bool)>,AppError> {
        let slots=self.slots.lock().map_err(|_|AppError::Conflict("input preparation ownership is unconfirmed".into()))?;
        Ok(slots.get(session.as_ref()).map(|slot|(slot.input.clone(),slot.cancel.is_cancelled())).into_iter().collect())
    }
    fn sessions(&self)->Result<Vec<String>,AppError> {
        Ok(self.slots.lock().map_err(|_|AppError::Conflict("input preparation ownership is unconfirmed".into()))?.keys().cloned().collect())
    }
    fn register(&self,session:&AgentSessionId,input:&EventId)->Result<InputPreparationGuard<'_>,AppError> {
        let mut slots=self.slots.lock().map_err(|_|AppError::Conflict("input preparation ownership is unconfirmed".into()))?;
        if slots.contains_key(session.as_ref()) {return Err(AppError::Conflict("original input preparation still owns this Session".into()));}
        let nonce=Uuid::now_v7();let cancel=tokio_util::sync::CancellationToken::new();
        let finished=tokio_util::sync::CancellationToken::new();
        slots.insert(session.as_ref().into(),InputPreparation {nonce,input:input.clone(),cancel:cancel.clone(),finished:finished.clone()});
        Ok(InputPreparationGuard {owner:self,session:session.as_ref().into(),nonce,cancel,finished})
    }
    fn cancel(&self,session:&AgentSessionId,input:&EventId)->Result<bool,AppError> {
        Ok(self.cancel_and_observe(session,input)?.is_some())
    }
    fn cancel_and_observe(&self,session:&AgentSessionId,input:&EventId)->Result<Option<tokio_util::sync::CancellationToken>,AppError> {
        let control=self.slots.lock().map_err(|_|AppError::Conflict("input preparation ownership is unconfirmed".into()))?
            .get(session.as_ref()).filter(|slot|slot.input==*input).map(|slot|(slot.cancel.clone(),slot.finished.clone()));
        Ok(control.map(|(cancel,finished)|{cancel.cancel();finished}))
    }
}
impl Drop for InputPreparationGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut slots)=self.owner.slots.lock() {
            if slots.get(&self.session).is_some_and(|slot|slot.nonce==self.nonce) {slots.remove(&self.session);}
        }
        self.finished.cancel();
    }
}

fn accepted_delivery(message_id:String,replayed:bool)->IdempotentMessageDelivery {
    IdempotentMessageDelivery {message_id,replayed,completed:false,result_ok:None,result_text:None,result_error:None,
        result_error_code:None,result_error_retryable:None,user_input_request:None,turn_outcome:None}
}

pub(super) fn unclaimed_input_unavailability(state:&Value)
    ->Result<Option<nomifun_conversation::UnavailableInputCause>,AppError> {
    use nomifun_conversation::UnavailableInputCause;
    let reason=||state["reason"].as_str().ok_or_else(||AppError::Internal("canonical unavailable input lost its reason".into()));
    Ok(match state["state"].as_str() {
        Some("pending"|"claimed")=>None,
        Some("cancelled") if reason()?=="CANCELLED_BY_OWNER"=>Some(UnavailableInputCause::CancelledByOwner),
        Some("discarded") if reason()?=="CONTEXT_CLEARED"=>Some(UnavailableInputCause::ContextCleared),
        Some("blocked")=>{
            let reason=reason()?;
            let typed:nomifun_agent_session::SessionInputBlockedReason=serde_json::from_value(json!(reason))
                .map_err(|error|AppError::Internal(format!("canonical input block reason is invalid: {error}")))?;
            if typed==nomifun_agent_session::SessionInputBlockedReason::CancelledByOwner {
                return Err(AppError::Internal("terminal input cancellation cannot be a repairable block".into()));
            }
            Some(UnavailableInputCause::PreparationBlocked {reason:reason.into()})
        },
        _=>return Err(AppError::Internal("canonical input delivery state is invalid".into())),
    })
}

pub(super) fn unavailable_input_error(message_id:&str,cause:&nomifun_conversation::UnavailableInputCause)->AppError {
    let message=format!("Accepted input {message_id} is unavailable before Turn admission: {}",cause.code());
    match cause {
        nomifun_conversation::UnavailableInputCause::PreparationBlocked {..}=>AppError::UnprocessableEntity(message),
        _=>AppError::Conflict(message),
    }
}

fn unclaimed_channel_observation(state:&Value)->Result<nomifun_channel::ChannelInputObservation,AppError> {
    use nomifun_conversation::UnavailableInputCause;
    Ok(match unclaimed_input_unavailability(state)? {
        None=>nomifun_channel::ChannelInputObservation::Pending,
        Some(UnavailableInputCause::PreparationBlocked {reason})=>nomifun_channel::ChannelInputObservation::Blocked {reason},
        Some(cause)=>nomifun_channel::ChannelInputObservation::Discarded {reason:cause.code().into()},
    })
}

/// Project the public message fields from the canonical accepted envelope.
/// Host provenance remains in the original fact and is read again by admission;
/// it is not a client-authored SendMessageRequest field.
pub(super) fn runtime_message_request(input:&nomifun_agent_contracts::StrictJsonValue)->Result<SendMessageRequest,AppError> {
    let mut content=input.0.clone();
    if let Some(object)=content.as_object_mut() {
        object.remove("reply_to");
        object.remove("channel_source");
        object.remove("delivery_intent");
        if let Some(source)=object.remove("input_source").filter(|source|!source.is_null()) {
            serde_json::from_value::<nomifun_agent_contracts::AgentInputSource>(source)
                .map_err(|error|AppError::Internal(format!("invalid accepted input provenance: {error}")))?;
        }
    }
    serde_json::from_value(content).map_err(|error|AppError::Internal(error.to_string()))
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
    pub(super) async fn stop_composer_source(self:&std::sync::Arc<Self>,owner_id:String,session:AgentSessionId,input:EventId,key:String)
        ->Result<nomifun_api_types::CancelAgentSessionTurnResponseDto,AppError> {
        let owner=self.clone();let (sender,receiver)=tokio::sync::oneshot::channel();
        // The original task owner retains the durable fence, cancellation
        // signal and preparation join even when the HTTP observer disappears.
        if !self.background_tasks.spawn(Box::pin(async move {
            let outcome=async {
                let principal=PrincipalRef {principal_kind:"user".into(),principal_id:owner_id.clone()};
                match owner.canonical.store().stop_session_input(&principal,&session,&input,&key).await.map_err(agent_session_store_error)? {
                    nomifun_agent_session::SessionInputStop::Turn {operation_id}=>{
                        let receipt=owner.cancel_owned_turn(&owner_id,&session,&key,Some(&operation_id),
                            nomifun_common::AgentKillReason::UserCancelled,true).await?;
                        // Initial claim can win while its preparation owner
                        // still holds the handoff. Confirm that exact owner as
                        // well as the native Turn cancellation before replying.
                        if let Some(finished)=owner.input_preparations.cancel_and_observe(&session,&input)? {finished.cancelled().await;}
                        Ok(nomifun_api_types::CancelAgentSessionTurnResponseDto::Turn(nomifun_api_types::AgentSessionTurnMutationResponseDto {
                            agent_session_id:session.as_ref().into(),target_operation_id:receipt.target_operation_id.as_ref().into(),
                            message_id:receipt.event_id.as_ref().into(),cursor:session_cursor(&session,receipt.cursor.seq),status:"cancelled".into(),duplicate:receipt.duplicate,
                        }))
                    },
                    nomifun_agent_session::SessionInputStop::Input {event,through_seq,duplicate}=>{
                        let finished=owner.input_preparations.cancel_and_observe(&session,&input)?;
                        let _=owner.publish_session_input_delivery(&owner_id,&session,&input).await;
                        if let Some(finished)=finished {finished.cancelled().await;}
                        Ok(nomifun_api_types::CancelAgentSessionTurnResponseDto::Input(nomifun_api_types::AgentSessionInputCancellationResponseDto {
                            agent_session_id:session.as_ref().into(),input_message_id:input.as_ref().into(),message_id:event.event_id.as_ref().into(),
                            cursor:session_cursor(&session,through_seq),status:"cancelled".into(),duplicate,
                        }))
                    },
                }
            }.await;
            let _=sender.send(outcome);
        })) {return Err(AppError::Conflict("Session input Stop owner is shutting down".into()));}
        receiver.await.map_err(|_|AppError::Internal("Input Stop acknowledgement is unknown; retry its original key".into()))?
    }

    pub(crate) fn cancel_input_preparation(&self,session:&AgentSessionId,input:&EventId)->Result<bool,AppError> {
        self.input_preparations.cancel(session,input)
    }
    pub(crate) fn input_preparation_observations(&self,session:&AgentSessionId)->Result<Vec<(EventId,bool)>,AppError> {
        self.input_preparations.observations(session)
    }
    pub(crate) fn registered_input_preparation_sessions(&self)->Result<Vec<String>,AppError> {
        self.input_preparations.sessions()
    }
    #[cfg(test)]
    pub(in crate::router) async fn hold_input_preparation_gate_for_test(&self,session:&AgentSessionId)
        ->tokio::sync::OwnedRwLockWriteGuard<()> {
        self.session_operation_lock(session.as_ref()).write_owned().await
    }
    #[cfg(test)]
    pub(in crate::router) fn hold_input_preparation_for_test(&self,session:&AgentSessionId,input:&EventId)
        ->Result<impl Drop+Send+'_,AppError> {
        self.input_preparations.register(session,input)
    }
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
        let _observation=self.session_operation_lock(session_id).read_owned().await;
        self.canonical.session_record(&principal,&session).await?;
        let input=self.canonical.store().session_input(&session,&input_event_id.into()).await.map_err(agent_session_store_error)?;
        let claim=self.canonical.store().session_input_claim(&session,&input.event.event_id).await.map_err(agent_session_store_error)?;
        self.guard_channel_guest_input_read(owner_id,&session,&input,claim.as_ref()).await?;
        let Some(claim)=claim else {
            let state=self.canonical.store().session_input_delivery(&session,&input.event.event_id).await.map_err(agent_session_store_error)?;
            return unclaimed_channel_observation(&state);
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
            PublicTurnDeliveryState::InputUnavailable {message_id,cause,..}=>return Err(unavailable_input_error(&message_id,&cause)),
        };
        Ok(ChannelInputObservation::Claimed {turn_id:claim.source_message_id.as_ref().into(),
            delivery:nomifun_channel::ChannelTurnDelivery {delivery:channel_delivery_from_conversation(delivery),events:None},question,terminal_events})
    }

    pub(super) async fn accept_ordinary_session_input(&self,owner_id:&str,session:&AgentSessionId,key:&str,
        request:SendMessageRequest,reply_to:Option<nomifun_api_types::AgentQuestionReplyTargetDto>)
        ->Result<IdempotentMessageDelivery,AppError> {
        self.accept_session_input_with_source(owner_id,session,key,request,reply_to,None,false,SessionInputIntakePolicy::Domain).await
    }

    pub(super) async fn accept_channel_session_input(&self,owner_id:&str,session:&AgentSessionId,key:&str,
        request:SendMessageRequest,source:nomifun_agent_contracts::ChannelInputSource,
        reply_to:Option<nomifun_api_types::AgentQuestionReplyTargetDto>)->Result<IdempotentMessageDelivery,AppError> {
        source.validate().map_err(AppError::BadRequest)?;
        self.accept_session_input_with_source(owner_id,session,key,request,reply_to,Some(source),false,SessionInputIntakePolicy::Domain).await
    }

    async fn accept_session_input_with_source(&self,owner_id:&str,session:&AgentSessionId,key:&str,
        request:SendMessageRequest,reply_to:Option<nomifun_api_types::AgentQuestionReplyTargetDto>,
        source:Option<nomifun_agent_contracts::ChannelInputSource>,initial_only:bool,
        policy:SessionInputIntakePolicy)->Result<IdempotentMessageDelivery,AppError> {
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
        let composer_default=matches!(&policy,SessionInputIntakePolicy::Composer(None));
        if let SessionInputIntakePolicy::Composer(Some(delivery))=policy {
            input["delivery_intent"]=serde_json::to_value(delivery).map_err(|error|AppError::Internal(error.to_string()))?;
        }
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
                    // Current-generation uncertain submissions retain their
                    // exact original envelope. Only a new omitted preference
                    // defaults to Queue; a replay never rewrites old intent.
                    if composer_default && original.input.0.get("delivery_intent").is_some() {
                        input["delivery_intent"]=json!({"kind":"queue"});
                    }
                    if original.input.0!=input {return Err(AppError::Conflict("Session input key was reused with different content".into()));}
                    return Ok((original.event.event_id,true));
                }
                if composer_default {input["delivery_intent"]=json!({"kind":"queue"});}
                host.require_ordinary_owner_input(&principal,&session).await?;
                let saved=if initial_only {host.canonical.accept_initial_session_input(&principal,&session,&key,input).await?}
                    else {host.canonical.accept_session_input(&principal,&session,&key,input).await?};
                let ack=saved.ack.ok_or_else(||AppError::Internal("accepted input has no canonical acknowledgement".into()))?;
                Ok::<_,AppError>((ack.event_id,saved.duplicate))
            }.await;
            match outcome {
                Ok((input,replayed))=>{
                    // Commit transfers the wake to the existing Host task
                    // owner before any read-only publication or HTTP observer.
                    // A failed/slow claim observation must not strand a newly
                    // accepted idle input. Replays are checked under the drain
                    // gate against this exact source before selecting work.
                    host.schedule_session_input_drain(owner.clone(),session.clone(),replayed.then_some(input.clone()));
                    if !replayed {let _=host.publish_session_input_delivery(&owner,&session,&input).await;}
                    let _=send.send(Ok(accepted_delivery(input.as_ref().to_owned(),replayed)));
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
        if let Some(input)=&retry_input {
            // An original-key retry only confirms or prepares its unclaimed
            // source. In particular a late replay cannot wake another queued
            // task after the source has already joined a Turn.
            if self.canonical.store().session_input_claim(session,input).await.map_err(agent_session_store_error)?.is_some() {
                return Ok(());
            }
            let delivery=self.canonical.store().session_input_delivery(session,input).await.map_err(agent_session_store_error)?;
            if matches!(delivery["state"].as_str(),Some("cancelled"|"discarded")) {return Ok(());}
        }
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
            let preparing=self.input_preparations.register(session,&original.event.event_id)?;
            // Publish the exact token before this reread. Stop on either side
            // of registration is therefore consumed before construction or
            // signalled to the original factory owner, without the Session gate.
            let delivery=self.canonical.store().session_input_delivery(session,&original.event.event_id).await.map_err(agent_session_store_error)?;
            if matches!(delivery.get("state").and_then(Value::as_str),Some("cancelled"|"discarded"|"claimed")) {
                preparing.cancel.cancel();continue;
            }
            // Re-read the selected original through the canonical reference
            // validator before preparing files or Skills. A pending batch is
            // only a candidate list, never authority to skip replacement
            // ancestry validation or fall back to its previously loaded body.
            let original=self.canonical.store().session_input(session,&original.event.event_id).await.map_err(agent_session_store_error)?;
            let request=runtime_message_request(&original.input)?;
            let operation=OperationId::from(format!("turn:session-input:{}",original.event.event_id.as_ref()));
            let preparation=async {
                let admitted=self.canonical_turn_input_with_admission(owner_id,session,&operation,&request).await?;
                let projection=self.canonical_conversation_projection(owner_id,session).await?
                    .ok_or_else(||AppError::NotFound("Session input target disappeared".into()))?;
                let options=self.runtime_options_for_projection(owner_id,session,projection).await?;
                let runtime=self.runtime_sessions.get_or_create_runtime_for_preparation(session.as_ref(),preparing.cancel.clone(),options).await?;
                if preparing.cancel.is_cancelled() {return Err(AppError::Conflict("original Session input preparation was stopped".into()));}
                runtime.validate_session_input(request.files.clone(),request.inject_skills.clone()).await?;
                if preparing.cancel.is_cancelled() {return Err(AppError::Conflict("original Session input preparation was stopped".into()));}
                Ok::<_,AppError>(admitted)
            }.await;
            let admitted=match preparation {
                Ok(admitted)=>admitted,
                Err(_)=>{
                    // A precise queue control can withdraw preparation without
                    // declaring the retained input broken or dropping its owner.
                    if preparing.cancel.is_cancelled() {return Ok(());}
                    self.canonical.store().mark_session_input_blocked(session,&original.event.event_id,
                        nomifun_agent_session::SessionInputBlockedReason::ContextPreparationFailed).await.map_err(agent_session_store_error)?;
                    let _=self.publish_session_input_delivery(owner_id,session,&original.event.event_id).await;
                    continue;
                },
            };
            let admission=admitted.get("admission").cloned().ok_or_else(||AppError::Internal("new Turn lost its exact execution admission".into()))?;
            let Some(claimed)=self.canonical.store().try_claim_session_input_as_turn(&principal,session,&original.event.event_id,&operation,StrictJsonValue(admission))
                .await.map_err(agent_session_store_error)? else {
                // A newer input/queue control owns its own wake. Release this
                // stale preparation instead of dispatching the old order.
                return Ok(());
            };
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

async fn confirmed_input_error(host:&NomiCoreSessionOwner,owner:&AuthenticatedOwner,session:&AgentSessionId,key:&str,
    error:NomiCoreApiError)->NomiCoreApiError {
    if !error.status.is_client_error() {return error;}
    match host.canonical.session_input_by_key(&authenticated_principal(owner),session,key).await {
        Ok(None)=>NomiCoreApiError::with_details(error.status,"INPUT_NOT_ACCEPTED",error.message,
            json!({"accepted":false,"reason":error.code})),
        Ok(Some(_))=>error,
        Err(failure)=>failure.into(),
    }
}

pub(super) async fn accept_message(State(state):State<NomiCoreAgentApiState>,Extension(owner):Extension<AuthenticatedOwner>,
    Path(id):Path<String>,headers:HeaderMap,Json(request):Json<nomifun_api_types::AcceptAgentSessionMessageRequestDto>)
    ->Result<Json<ApiResponse<nomifun_api_types::AcceptAgentSessionMessageResponseDto>>,NomiCoreApiError> {
    let initial_only=initial_delivery_requested(&headers)?;
    let session=parse_agent_session_id(&id)?;let key=canonical_nonempty(&request.idempotency_key,"idempotency_key")?;
    let input=match bounded_turn_input(request.input) {
        Ok(input)=>input,
        Err(error)=>return Err(confirmed_input_error(&state.session_owner,&owner,&session,&key,error).await),
    };
    let delivery=match state.session_owner.accept_session_input_with_source(owner.as_ref(),&session,&key,input,None,None,initial_only,SessionInputIntakePolicy::Composer(request.delivery)).await {
        Ok(delivery)=>delivery,
        Err(error)=>{
            // Only the completed acceptance owner plus an authoritative absent
            // receipt can prove non-admission. Existing-key conflicts remain
            // delivery-unknown and retain the original client identity.
            return Err(confirmed_input_error(&state.session_owner,&owner,&session,&key,error.into()).await);
        },
    };
    let original=state.session_owner.canonical.session_input_by_key(&authenticated_principal(&owner),&session,&key).await?
        .ok_or_else(||AppError::Internal("accepted input acknowledgement could not be re-read".into()))?;
    let input_delivery=serde_json::from_value(state.session_owner.canonical.store().session_input_delivery(&session,&original.event.event_id)
        .await.map_err(agent_session_store_error)?).map_err(|error|AppError::Internal(format!("invalid accepted input view: {error}")))?;
    Ok(Json(ApiResponse::ok(nomifun_api_types::AcceptAgentSessionMessageResponseDto {agent_session_id:id,
        message_id:delivery.message_id,cursor:session_cursor(&session,original.event.seq),replayed:delivery.replayed,status:"accepted".into(),input_delivery})))
}

/// Reading a queue never acquires a Runtime or schedules execution.
pub(super) async fn get_input_queue(State(state):State<NomiCoreAgentApiState>,Extension(owner):Extension<AuthenticatedOwner>,
    Path(id):Path<String>)->Result<Json<ApiResponse<Value>>,NomiCoreApiError> {
    let session=parse_agent_session_id(&id)?;let principal=authenticated_principal(&owner);
    let snapshot=state.session_owner.canonical.store().session_input_queue(&principal,&session).await.map_err(agent_session_store_error)?;
    Ok(Json(ApiResponse::ok(serde_json::to_value(snapshot).map_err(|error|AppError::Internal(error.to_string()))?)))
}

/// The existing tracked task owner retains a queue mutation after its observer
/// disappears. The Store serializes it with initial/steering claim and Stop.
pub(super) async fn update_input_queue(State(state):State<NomiCoreAgentApiState>,Extension(owner):Extension<AuthenticatedOwner>,
    Path(id):Path<String>,Json(request):Json<nomifun_api_types::UpdateAgentSessionInputQueueRequestDto>)
    ->Result<Json<ApiResponse<Value>>,NomiCoreApiError> {
    let session=parse_agent_session_id(&id)?;let principal=authenticated_principal(&owner);
    let key=canonical_nonempty(&request.idempotency_key,"idempotency_key")?;
    let host=state.session_owner.clone();let (sender,receiver)=tokio::sync::oneshot::channel();
    if !state.session_owner.background_tasks.spawn(Box::pin(async move {
        let outcome=async {
            let before=host.canonical.store().session_input_queue(&principal,&session).await.map_err(agent_session_store_error)?;
            let command_key=key.into();
            let (snapshot,replayed)=match host.canonical.store().read_session_input_queue_command_receipt(&principal,&session,&command_key,&request.action,Some(request.expected_version))
                .await.map_err(agent_session_store_error)? {
                Some(snapshot)=>(snapshot,true),
                None=>{
                    host.require_ordinary_owner_input(&principal,&session).await?;
                    (host.canonical.store().update_session_input_queue_command(&principal,&session,&command_key,&request.action,Some(request.expected_version))
                        .await.map_err(agent_session_store_error)?,false)
                },
            };
            // Commit transfers scheduling to the retained Session owner before
            // any fallible observer read or client notification can interrupt it.
            if !replayed {host.schedule_session_input_drain(owner.as_ref().to_owned(),session.clone(),None);}
            for (input,_) in host.input_preparation_observations(&session)? {
                let delivery=host.canonical.store().session_input_delivery(&session,&input).await.map_err(agent_session_store_error)?;
                // Terminal input cancellation cannot acquire a later owner for
                // this identity. Pause/reorder instead use the claim writer's
                // eligibility fence: signalling by message ID after a quick
                // resume could otherwise cancel its legitimate new preparation.
                if matches!(delivery["state"].as_str(),Some("cancelled"|"discarded")) {
                    host.cancel_input_preparation(&session,&input)?;
                }
            }
            // Publish original input identities so all connected clients reread
            // the same projection, including items just removed by this command.
            let ids=before.items.iter().chain(snapshot.items.iter()).map(|item|item.message_id.clone()).collect::<BTreeSet<_>>();
            for input in ids {let _=host.publish_session_input_delivery(owner.as_ref(),&session,&input).await;}
            serde_json::to_value(snapshot).map_err(|error|AppError::Internal(error.to_string()))
        }.await;
        let _=sender.send(outcome);
    })) {return Err(AppError::Conflict("Session queue control owner is shutting down".into()).into());}
    let snapshot=receiver.await.map_err(|_|AppError::Internal("Queue acknowledgement is unknown; retry its original key".into()))??;
    Ok(Json(ApiResponse::ok(snapshot)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_preparation_stop_signals_only_the_captured_source_and_never_a_successor() {
        let controls=InputPreparationControls::default();
        let session=AgentSessionId::from(Uuid::now_v7().to_string());
        let first=EventId::from("first-accepted-input");let next=EventId::from("next-accepted-input");
        let original=controls.register(&session,&first).unwrap();
        assert!(!controls.cancel(&session,&next).unwrap());
        assert!(!original.cancel.is_cancelled());
        assert!(controls.cancel(&session,&first).unwrap());
        assert!(original.cancel.is_cancelled());
        drop(original);
        let successor=controls.register(&session,&next).unwrap();
        assert!(!controls.cancel(&session,&first).unwrap());
        assert!(!successor.cancel.is_cancelled());
        drop(successor);
        assert!(controls.slots.lock().unwrap().is_empty(),"completed preparation retains no control/Runtime references");
    }

    #[test]
    fn unavailable_input_read_causes_preserve_cancel_clear_and_repairable_blocks() {
        use nomifun_conversation::UnavailableInputCause;
        for state in ["pending","claimed"] {
            assert_eq!(unclaimed_input_unavailability(&json!({"state":state})).unwrap(),None);
        }
        for (state,reason,cause) in [
            ("cancelled","CANCELLED_BY_OWNER",UnavailableInputCause::CancelledByOwner),
            ("discarded","CONTEXT_CLEARED",UnavailableInputCause::ContextCleared),
            ("blocked","SKILL_UNAVAILABLE",UnavailableInputCause::PreparationBlocked {reason:"SKILL_UNAVAILABLE".into()}),
        ] {
            let value=json!({"state":state,"reason":reason,"event_seq":7});
            assert_eq!(unclaimed_input_unavailability(&value).unwrap(),Some(cause.clone()));
            assert_eq!(cause.code(),reason);
            match unclaimed_channel_observation(&value).unwrap() {
                nomifun_channel::ChannelInputObservation::Discarded {reason:code}
                |nomifun_channel::ChannelInputObservation::Blocked {reason:code}=>assert_eq!(code,reason),
                _=>panic!("an unavailable input is never pending or a claimed Turn"),
            }
            let remote=super::super::remote_unavailable_initial_input("original-accepted-id",&cause);
            assert_eq!(remote["message_id"],"original-accepted-id");
            assert_eq!(remote["initial_input"]["state"],state);
            assert_eq!(remote["turn_admitted"],false);
            assert!(remote.get("result_ok").is_none(),"opening a Session does not imply work completion");
        }
        for invalid in [json!({"state":"cancelled","reason":"SKILL_UNAVAILABLE"}),
            json!({"state":"blocked","reason":"CANCELLED_BY_OWNER"}),json!({"state":"blocked"}),
            json!({"state":"blocked","reason":"UNKNOWN_PREPARATION_FAILURE"})] {
            assert!(unclaimed_input_unavailability(&invalid).is_err());
        }
    }

    #[test]
    fn physical_provenance_stays_canonical_while_the_runtime_receives_public_message_fields() {
        let input=StrictJsonValue(json!({
            "content":"查询今天的天气", "files":[], "inject_skills":[], "hidden":false,
            "origin":"robot", "channel_platform":"robot",
            "input_source":{"kind":"authenticated_robot","companion_id":"partner",
                "robot_id":"device","connection_id":"original-connection"},
            "reply_to":{"turn_id":"original-question","question_seq":7}
        }));
        let original=input.clone();
        assert!(serde_json::from_value::<SendMessageRequest>(input.0.clone()).is_err());
        let message=runtime_message_request(&input).unwrap();
        assert_eq!(message.content,"查询今天的天气");
        assert_eq!(message.origin.as_deref(),Some("robot"));
        assert_eq!(input,original,"projection must not erase the durable authenticated source or question");

        let mut unknown=input.clone();
        unknown.0["unexpected_client_field"]=json!(true);
        assert!(runtime_message_request(&unknown).is_err(),"unknown client fields remain rejected");
        let mut retired=input.clone();
        retired.0["input_source"]["kind"]=json!("directed_delegation_result");
        assert!(runtime_message_request(&retired).is_err(),"read-only retired history cannot enter live admission");
    }
}
