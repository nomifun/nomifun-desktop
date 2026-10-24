//! Shared live/cold terminal authoring. Cold settlement reads canonical facts
//! only and requires the old native owner's already-persisted cleanup proof.
use super::*;
use crate::router::nomi_core_session::TurnErrorContext;
use nomifun_agent_session::NativeTurnSettlementReceipt;

pub(super) fn terminal_payload(event:&AgentEngineEvent,context:&TurnErrorContext,finished_at_ms:i64)
    ->Result<(&'static str,Value),AppError> {
    match event {
        AgentEngineEvent::TurnInputRequired {model_steps,request}=>{
            request.validate().map_err(failure)?;
            Ok(("turn/completed",json!({"model_steps":model_steps,"finish_reason":nomifun_chat_model_broker::ChatFinishReason::Completed,
                "user_input_request":request,"finished_at_ms":finished_at_ms})))
        }
        AgentEngineEvent::TurnCompleted {model_steps,finish_reason}=>Ok(("turn/completed",json!({
            "model_steps":model_steps,"finish_reason":finish_reason,"finished_at_ms":finished_at_ms}))),
        AgentEngineEvent::TurnFailed {model_steps,message,failure:typed}=>{
            let mut error=nomifun_ai_agent::AgentSendError::from_runtime_turn_failure(message.clone(),typed.as_ref()).into_stream_error();
            context.apply(&mut error);
            Ok(("turn/failed",json!({"model_steps":model_steps,"message":message,"error":error,"finished_at_ms":finished_at_ms})))
        }
        _=>Err(failure("this exit has no canonical chat terminal")),
    }
}

pub(super) fn completion_event(session:&AgentSessionId,operation:&OperationId,root:&EventId,step:&AssistantStepCursor)->SessionEventAppend {
    let identity=format!("assistant-complete:{}:{}:{}",session.as_ref(),operation.as_ref(),step.step);
    SessionEventAppend {agent_session_id:session.clone(),event_id:identity.clone().into(),producer_id:"runtime_supervisor".into(),idempotency_key:identity.into(),
        semantic_event:SemanticSessionEventDraft {kind:SessionEventKind("message/completed".into()),kind_version:1,
            correlation_id:step.message_id.as_str().into(),causation_event_id:Some(step.last_event_id.clone().unwrap_or_else(||root.clone())),
            payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"part_count":step.parts,"content_digest":digest_bytes(&step.text)})))}}
}

fn saved_step(facts:&nomifun_agent_session::ChatCausalityFacts,root:&EventId,step:u16)->Result<AssistantStepCursor,AppError> {
    let id=canonical_assistant_step_message_id(root.as_ref(),step)?;
    let mut saved=AssistantStepCursor {step,message_id:id.clone(),parts:0,text:Vec::new(),last_event_id:None};
    for event in facts.events.iter().filter(|event|event.kind.0=="message/content-part"&&event.correlation_id.as_ref()==id) {
        let body=facts.event_payloads.get(event.event_id.as_ref()).ok_or_else(||failure("cold result part lost its original Payload"))?;
        if body["turn_id"].as_str()!=Some(root.as_ref()) {return Err(failure("cold result part belongs to another accepted root"));}
        let text=body["content"].as_str().ok_or_else(||failure("cold result part is not text"))?;
        saved.parts+=1;saved.text.extend_from_slice(text.as_bytes());saved.last_event_id=Some(event.event_id.clone());
    }
    Ok(saved)
}

impl EngineTurnJournal {
    /// Finish only a saved exit whose exact old native cleanup is proven.
    /// No Runtime, model, tool, input or checkpoint execution is constructed.
    pub(in crate::router) async fn settle_cold_native_turn(store:&AgentSessionStore,receipt:&NativeTurnSettlementReceipt,context:&TurnErrorContext)
        ->Result<bool,AppError> {
        if !store.native_turn_settlement_current(receipt).await.map_err(failure)?
            ||!store.native_turn_settlement_cleanup_proven(receipt).await.map_err(failure)? {return Ok(false);}
        let turn=store.read_turn_receipt(receipt.session_id(),receipt.operation_id()).await.map_err(failure)?;
        if matches!(turn.status,TurnReceiptStatus::Completed|TurnReceiptStatus::Failed|TurnReceiptStatus::Cancelled) {
            store.complete_native_turn_settlement(receipt).await.map_err(failure)?;return Ok(true);
        }
        let mut outcome:nomifun_ai_agent::engine_sdk::EngineTurnOutcome=serde_json::from_value(receipt.outcome.0.clone()).map_err(failure)?;
        let original_event=nomifun_ai_agent::unified_runtime::runtime_terminal(&outcome);
        let user_pause=store.native_turn_settlement_requires_user_pause(receipt).await.map_err(failure)?;
        if user_pause {outcome.terminal=nomifun_ai_agent::engine_sdk::EngineTurnTerminal::Paused {reason:"EXECUTION_USER_REQUESTED".into()};}
        let event=nomifun_ai_agent::unified_runtime::runtime_terminal(&outcome);
        let facts=store.chat_causality_facts(receipt.session_id(),receipt.operation_id()).await.map_err(failure)?;
        let started=turn.started_event.ok_or_else(||failure("cold settlement lost its original Turn start"))?;
        let root=started.causation_event_id.ok_or_else(||failure("cold settlement lost its original accepted input"))?;
        let mut native_sequence=0u64;
        let mut public_step=None;
        let mut discarded=std::collections::BTreeSet::new();
        let mut terminal_sequence=None;
        for recorded in facts.events.iter().filter(|recorded|recorded.kind.0=="runtime/progress-recorded"&&recorded.correlation_id.as_ref()==receipt.operation_id().as_ref()) {
            let body=facts.event_payloads.get(recorded.event_id.as_ref()).ok_or_else(||failure("cold settlement lost native journal Payload"))?;
            let sequence=body["producer_seq"].as_u64().ok_or_else(||failure("cold settlement has no native producer sequence"))?;
            if sequence!=native_sequence+1 {return Err(failure("cold settlement native journal is incomplete"));}
            native_sequence=sequence;
            // A resumed Turn keeps its prior pauses and producer sequence.
            // Only this receipt's admitted generation can provide its exit;
            // older exits remain history, never a substitute or a conflict.
            let original_generation=recorded.seq>=receipt.generation();
            if let Ok(recorded)=serde_json::from_value::<AgentEngineEvent>(body["event"].clone()) {
                match &recorded {
                    AgentEngineEvent::OutputTextDelta {step,..}|AgentEngineEvent::CompletionDelivered {step,..}=>public_step=Some(*step),
                    AgentEngineEvent::VoiceModelStepSuperseded {step,..}=>{discarded.insert(*step);},
                    AgentEngineEvent::TurnCompleted {..}|AgentEngineEvent::TurnInputRequired {..}|AgentEngineEvent::TurnCancelled {..}|AgentEngineEvent::TurnFailed {..}|AgentEngineEvent::TurnPaused {..}
                        if original_generation=>{
                        if recorded==event {terminal_sequence=Some(sequence);}
                        else if !user_pause||recorded!=original_event {return Err(failure("cold terminal differs from the immutable saved exit"));}
                    }
                    _=>{},
                }
            }
        }
        let terminal_sequence=terminal_sequence.unwrap_or(native_sequence+1);
        let identity=format!("runtime-progress:{}:{}:{}",receipt.session_id().as_ref(),receipt.operation_id().as_ref(),terminal_sequence);
        let (runtime_reference,runtime_payload)=canonical_event_payload(receipt.session_id(),json!({"producer_seq":terminal_sequence,"event":event}))?;
        let runtime=SessionEventAppend {agent_session_id:receipt.session_id().clone(),event_id:identity.clone().into(),producer_id:"runtime_supervisor".into(),idempotency_key:identity.into(),
                semantic_event:SemanticSessionEventDraft {kind:SessionEventKind("runtime/progress-recorded".into()),kind_version:1,
                    correlation_id:receipt.operation_id().as_ref().into(),causation_event_id:Some(root.clone()),
                    payload:runtime_reference}};
        if matches!(event,AgentEngineEvent::TurnPaused {..}) {
            let message=public_step.filter(|step|!discarded.contains(step)).map(|step|saved_step(&facts,&root,step)).transpose()?
                .map(|step|completion_event(receipt.session_id(),receipt.operation_id(),&root,&step));
            store.append_native_settlement_terminal_bundle(receipt,&runtime,runtime_payload.as_ref(),message.as_ref(),None).await.map_err(failure)?;
            return Ok(true);
        }
        if matches!(event,AgentEngineEvent::TurnCancelled {..}) {
            let id=format!("runtime-cancel:{}:{}",receipt.session_id().as_ref(),receipt.operation_id().as_ref());
            let append=SessionEventAppend {agent_session_id:receipt.session_id().clone(),event_id:id.clone().into(),producer_id:"runtime_supervisor".into(),idempotency_key:id.into(),
                semantic_event:SemanticSessionEventDraft {kind:SessionEventKind("turn/cancelled".into()),kind_version:1,
                    correlation_id:receipt.operation_id().as_ref().into(),causation_event_id:Some(started.event_id),
                    payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(json!({"operation_id":receipt.operation_id()})))}};
            store.append_native_settlement_terminal_bundle(receipt,&runtime,runtime_payload.as_ref(),None,Some(&append)).await.map_err(failure)?;return Ok(true);
        }
        let final_step=match &event {AgentEngineEvent::TurnCompleted {model_steps,..}|AgentEngineEvent::TurnInputRequired {model_steps,..} if *model_steps>0=>Some(*model_steps),_=>None};
        public_step=public_step.filter(|step|!discarded.contains(step));
        if let (Some(previous),Some(final_step))=(public_step,final_step) {
            if previous>final_step {return Err(failure("cold result precedes its original public step"));}
            if previous<final_step {
                let prior=saved_step(&facts,&root,previous)?;
                store.append_native_settlement_observation(receipt,&completion_event(receipt.session_id(),receipt.operation_id(),&root,&prior),None).await.map_err(failure)?;
            }
        }
        let step=saved_step(&facts,&root,final_step.or(public_step).unwrap_or(1))?;
        let message=completion_event(receipt.session_id(),receipt.operation_id(),&root,&step);
        let (kind,payload)=terminal_payload(&event,context,receipt.pause.as_ref().map_or_else(now_ms,|pause|pause.paused_at_ms))?;
        let identity=format!("turn-terminal:{}:{}",receipt.session_id().as_ref(),receipt.operation_id().as_ref());
        let terminal=SessionEventAppend {agent_session_id:receipt.session_id().clone(),event_id:identity.clone().into(),producer_id:"runtime_supervisor".into(),idempotency_key:identity.into(),
            semantic_event:SemanticSessionEventDraft {kind:SessionEventKind(kind.into()),kind_version:1,correlation_id:receipt.operation_id().as_ref().into(),
                causation_event_id:Some(message.event_id.clone()),payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(payload))}};
        store.append_native_settlement_terminal_bundle(receipt,&runtime,runtime_payload.as_ref(),Some(&message),Some(&terminal)).await.map_err(failure)?;
        Ok(true)
    }
}
