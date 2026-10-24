//! Receipt-backed, turn-local inbox. A queued acknowledgement is not proof
//! that a model consumed or followed the text. No automatic cross-turn replay.
use std::collections::{BTreeMap,BTreeSet};
use std::sync::Weak;

use async_trait::async_trait;
use nomifun_ai_agent::RuntimeSteerDelivery;
use nomifun_chat_model_broker::ChatCausality;
use nomifun_agent_runtime::{
    AgentEngineError, AgentEngineEvent, AgentInputPort, AgentSteeringInput,
};
use nomifun_common::AppError;

use super::{ActiveTurn, ConversationRuntimeHost, error};

struct SteeringPreparationFailure {error:AppError,reason:Option<nomifun_agent_session::SessionInputBlockedReason>}
impl From<AppError> for SteeringPreparationFailure {fn from(error:AppError)->Self {Self {error,
    reason:Some(nomifun_agent_session::SessionInputBlockedReason::AttachmentPreparationFailed)}}}
fn preparation_budget(error:AppError)->SteeringPreparationFailure {SteeringPreparationFailure {error,
    reason:Some(nomifun_agent_session::SessionInputBlockedReason::InputBudgetExceeded)}}
fn preparation_skill(error:AppError)->SteeringPreparationFailure {SteeringPreparationFailure {error,
    reason:Some(nomifun_agent_session::SessionInputBlockedReason::SkillUnavailable)}}

struct PreparedSteeringContent {
    text:String,
    files:Vec<String>,
    inject_skills:Vec<String>,
    prepared_images:Vec<nomifun_chat_model_broker::ChatContentPart>,
    prepared_skill_instructions:Vec<String>,
    next_image_bytes:usize,
    next_image_count:usize,
    next_skill_bytes:usize,
}

pub(super) struct Inbox {
    changed: std::sync::Arc<tokio::sync::Notify>,
    open: bool,
    generation: Option<u64>,
    seen: BTreeMap<String, AgentSteeringInput>,
    pending: Vec<AgentSteeringInput>,
    retry_inputs:BTreeSet<String>,
    deferred_inputs:BTreeSet<String>,
    immediate_receipts:BTreeSet<String>,
    admitted_model_steps:BTreeSet<u16>,
    prepared_image_bytes: usize,
    prepared_image_count: usize,
    prepared_skill_bytes: usize,
}

/// Read-only hints from the original Turn inbox. Durable candidate selection
/// still belongs to the canonical Store; this value cannot claim an input.
pub(super) struct PendingInputObservation {
    pub(super) prepared: bool,
    pub(super) open: bool,
    pub(super) retry_inputs: Vec<String>,
    pub(super) deferred_inputs: BTreeSet<String>,
}
impl Default for Inbox {
    fn default() -> Self {
        Self {
            changed: std::sync::Arc::new(tokio::sync::Notify::new()),
            open: false,
            generation: None,
            seen: BTreeMap::new(),
            pending: Vec::new(),
            retry_inputs:BTreeSet::new(),
            deferred_inputs:BTreeSet::new(),
            immediate_receipts:BTreeSet::new(),
            admitted_model_steps:BTreeSet::new(),
            prepared_image_bytes: 0,
            prepared_image_count: 0,
            prepared_skill_bytes: 0,
        }
    }
}

impl Inbox {
    pub(super) fn changed(&self) -> std::sync::Arc<tokio::sync::Notify> { self.changed.clone() }
    pub(super) fn pending_observation(&self) -> PendingInputObservation {
        PendingInputObservation {prepared:!self.pending.is_empty(),open:self.open,
            retry_inputs:self.retry_inputs.iter().cloned().collect(),deferred_inputs:self.deferred_inputs.clone()}
    }
    pub(super) fn immediate_receipt_ids(&self)->Vec<String>{self.immediate_receipts.iter().cloned().collect()}
    pub(super) fn has_admitted_model_tools(&self,step:u16)->bool{self.admitted_model_steps.contains(&step)}
    pub(super) fn permits_resource_dispatch(&self) -> bool {
        self.open && self.pending.is_empty()
    }

    #[cfg(test)]
    pub(super) fn pending_receipt_ids(&self) -> Vec<String> {
        self.pending.iter().map(|input| input.receipt_operation_id.clone()).collect()
    }
}

pub(super) struct HostPort(pub(super) Weak<ConversationRuntimeHost>);
impl std::fmt::Debug for HostPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConversationInputPort")
    }
}
fn engine_error(value: impl std::fmt::Display) -> AgentEngineError {
    AgentEngineError::InvalidContract(format!("Agent Runtime steering: {value}"))
}
pub(super) fn admitted(
    turn: &ActiveTurn,
    host: &ConversationRuntimeHost,
    causality: &ChatCausality,
) -> Result<(), AgentEngineError> {
    if turn.cleanup_started
        || turn.cancellation.is_cancelled()
        || turn.journal.sequence() == 0
        || causality.agent_session_id.as_ref() != host.options.conversation_id
        || causality.turn_operation_id.as_ref() != turn.operation
        || causality.causation_event_id.as_ref() != turn.root
        || causality.resolved_snapshot_ref != host.snapshot_ref
        || causality.route_identity != host.route
    {
        return Err(engine_error("input request differs from the admitted turn"));
    }
    Ok(())
}

impl ConversationRuntimeHost {
    pub(super) async fn wake_session_input(&self,retry_input:Option<String>)->Result<(),AppError> {
        let mut active=self.active.lock().await;
        if let Some(turn)=active.as_mut() {
            if let Some(input)=retry_input {
                if turn.steering.retry_inputs.len()<64 {turn.steering.retry_inputs.insert(input);}
            }
            turn.steering.changed.notify_waiters();
        }
        Ok(())
    }

    async fn claim_pending_session_inputs_locked(&self,turn:&mut ActiveTurn)->Result<(),AppError> {
        if !turn.steering.open || turn.cleanup_started || turn.cancellation.is_cancelled() {return Ok(());}
        let owner=self.session_host.session_owner()?;
        let session=self.options.conversation_id.clone().into();
        if owner.require_ordinary_owner_input(&self.principal,&session).await.is_err() {return Ok(());}
        let store=self.session_host.canonical_store()?;
        if store.has_unsettled_effects(&session).await.map_err(error)? {return Ok(());}
        let operation=turn.operation.clone().into();
        let mut inputs=store.pending_session_inputs_for_steering(&session,&operation,None).await.map_err(error)?;
        for id in std::mem::take(&mut turn.steering.retry_inputs) {
            inputs.extend(store.pending_session_inputs_for_steering(&session,&operation,Some(&id.into())).await.map_err(error)?);
        }
        inputs.sort_by_key(|input|input.event.seq);let mut seen=BTreeSet::new();
        for original in inputs {
            if !seen.insert(original.event.event_id.clone())
                || turn.steering.deferred_inputs.contains(original.event.event_id.as_ref()) {continue;}
            let text=original.input.0.get("content").and_then(serde_json::Value::as_str).ok_or_else(||error("accepted input lost its original text"))?.to_owned();
            let files=super::super::runtime_attachments::references(&original.input.0)?;
            let skills=super::super::runtime_attachments::selected_skills(&original.input.0)?;
            let prepared=match self.prepare_steer_content_locked(turn,text.clone(),files.clone(),skills.clone()).await {
                Ok(prepared)=>prepared,
                Err(failure)=>{
                    if failure.reason==Some(nomifun_agent_session::SessionInputBlockedReason::InputBudgetExceeded) {
                        turn.steering.deferred_inputs.insert(original.event.event_id.as_ref().into());continue;
                    }
                    if let Some(reason)=failure.reason {
                        store.mark_session_input_blocked(&session,&original.event.event_id,reason).await.map_err(error)?;
                        let _=owner.publish_session_input_delivery(&self.principal.principal_id,&session,&original.event.event_id).await;
                    }
                    continue;
                },
            };
            turn.journal.verify_execution_lease().await.map_err(error)?;
            #[cfg(test)]
            super::reliability_tests::session_input_preclaim_cutpoint(&self.options.conversation_id).await;
            // A queue control may win while attachment preparation awaits.
            // The original writer reports that ordinary race without turning
            // a user's pause/remove into a failure of the running model Turn.
            let Some(claim)=store.try_claim_session_input_as_steering(&self.principal,&session,&original.event.event_id,&operation).await.map_err(error)? else {continue;};
            let ack=claim.ack.ok_or_else(||error("input claim has no original acknowledgement"))?;
            // The Store has just checked the exact original input and target
            // under its transaction; the active mutex still owns this scope.
            // No second DB read or cancellation point may split that claim
            // from its prepared inbox entry. Exact domain steering continues
            // to use its independent receipt validation below.
            let input=AgentSteeringInput {receipt_operation_id:ack.event_id.as_ref().to_owned(),
                message_id:original.event.event_id.as_ref().to_owned(),text:prepared.text,
                files:prepared.files,inject_skills:prepared.inject_skills,image_count:prepared.prepared_images.len(),
                prepared_images:prepared.prepared_images,prepared_skill_instructions:prepared.prepared_skill_instructions};
            turn.steering.generation=Some(turn.epoch as u64);
            turn.steering.prepared_image_bytes=prepared.next_image_bytes;
            turn.steering.prepared_image_count=prepared.next_image_count;
            turn.steering.prepared_skill_bytes=prepared.next_skill_bytes;
            turn.steering.seen.insert(input.receipt_operation_id.clone(),input.journal_record());
            turn.steering.pending.push(input);
            turn.event_buffer.content_boundary();
            #[cfg(test)]
            super::reliability_tests::session_input_claim_cutpoint(&self.options.conversation_id).await;
        }
        Ok(())
    }

    async fn take_session_inputs_owned(&self,causality:ChatCausality,close_if_empty:bool)
        ->Result<Vec<AgentSteeringInput>,AgentEngineError> {
        let mut active=self.active.lock().await;
        let turn=active.as_mut().ok_or_else(||engine_error("no active turn"))?;
        admitted(turn,self,&causality)?;
        let result=async {
            self.claim_pending_session_inputs_locked(turn).await.map_err(engine_error)?;
            if turn.steering.pending.is_empty() {
                if close_if_empty {turn.steering.open=false;}
                return Ok(Vec::new());
            }
            let capabilities=self.capability_state.snapshot().map_err(engine_error)?;
            for input in &turn.steering.pending {
                self.skills.validate_active(&input.inject_skills,&capabilities.active).map_err(engine_error)?;
                if input.prepared_skill_instructions!=self.skills.explicit_instructions(&input.inject_skills).map_err(engine_error)? {
                    return Err(engine_error("queued Skill body differs from frozen active selection"));
                }
            }
            turn.steering.open=false;
            self.flush_steering_buffer(turn).await.map_err(engine_error)?;
            let payload=serde_json::to_string(&AgentEngineEvent::SteeringInputs {
                inputs:turn.steering.pending.iter().map(AgentSteeringInput::journal_record).collect(),
            }).map_err(engine_error)?;
            self.append_locked_record(turn,payload,None,false).await.map_err(engine_error)?;
            let inputs=std::mem::take(&mut turn.steering.pending);
            turn.steering.immediate_receipts.clear();turn.steering.open=true;
            Ok(inputs)
        }.await;
        if result.is_err() {
            // An uncertain native write cannot authorize subsequent effects.
            // Pending content is retained for exact cleanup/recovery; a claim
            // alone is never projected as delivery to the Runtime boundary.
            turn.steering.open=false;turn.cancellation.cancel();
        }
        let delivered=result.as_ref().ok().map(|inputs|inputs.iter().map(|input|input.message_id.clone()).collect::<Vec<_>>()).unwrap_or_default();
        drop(active);
        if let Ok(owner)=self.session_host.session_owner() {
            let session=self.options.conversation_id.clone().into();
            for input in delivered {let _=owner.publish_session_input_delivery(&self.principal.principal_id,&session,&input.into()).await;}
        }
        result
    }

    pub(super) async fn restore_recovery_steering(&self) -> Result<(), AppError> {
        let mut active = self.active.lock().await;
        let turn = active.as_mut().ok_or_else(|| error("recovery has no active Turn"))?;
        if !turn.journal.recovered() { return Ok(()); }
        let recovery = turn.journal.recovery();
        let store = self.session_host.canonical_store()?;
        let facts = store.native_recovery_facts(&self.options.conversation_id.clone().into(), &turn.operation.clone().into()).await.map_err(error)?;
        if facts.head.status != "running" || facts.execution_generation != turn.epoch as u64 {
            return Err(error("recovery input ownership changed"));
        }
        let capabilities = self.capability_state.snapshot().map_err(error)?;
        for input in recovery.as_ref().map(|recovery| recovery.prepared_inputs()).unwrap_or_default() {
            self.skills.validate_active(&input.inject_skills, &capabilities.active)?;
            let selected = self.skills.explicit_instructions(&input.inject_skills)?;
            if input.prepared_skill_instructions != selected {
                return Err(error("recovery Skill body differs from frozen selected Skill"));
            }
            turn.steering.seen.insert(input.receipt_operation_id.clone(), input.journal_record());
            turn.steering.prepared_image_count += input.image_count;
            turn.steering.prepared_image_bytes += input.prepared_images.iter().map(|part| match part {
                nomifun_chat_model_broker::ChatContentPart::Image { data_base64, .. } => data_base64.len(), _ => 0,
            }).sum::<usize>();
            turn.steering.prepared_skill_bytes += selected.iter().map(String::len).sum::<usize>();
        }
        if turn.steering.prepared_skill_bytes > 24 * 1024 {
            return Err(error("recovery steering selected Skill context exceeds its 24 KiB budget"));
        }
        for event in facts.events.iter().filter(|event| event.kind.0 == "turn/steer-accepted" && event.correlation_id.as_ref() == turn.operation) {
            if turn.steering.seen.contains_key(event.event_id.as_ref()) { continue; }
            let(original_input,value)=nomifun_agent_session::canonical_steering_input(&facts,event).map_err(error)?;
            let value=&value.0;
            let text = value.get("content").and_then(serde_json::Value::as_str).ok_or_else(|| error("recovery steering text is missing"))?;
            let files = super::super::runtime_attachments::references(value)?;
            let inject_skills = super::super::runtime_attachments::selected_skills(value)?;
            self.skills.validate_active(&inject_skills, &capabilities.active)?;
            let prepared_skill_instructions = self.skills.explicit_instructions(&inject_skills)?;
            let images = super::super::runtime_attachments::prepare_images(&files, &self.options.extra, self.route_image_input).await?;
            let input = AgentSteeringInput { receipt_operation_id: event.event_id.as_ref().to_owned(), message_id: original_input.as_ref().to_owned(),
                text: text.to_owned(), files, inject_skills, image_count: images.len(), prepared_images: images,
                prepared_skill_instructions };
            input.validate().map_err(error)?;
            turn.steering.prepared_image_count += input.image_count;
            turn.steering.prepared_image_bytes += input.prepared_images.iter().map(|part| match part {
                nomifun_chat_model_broker::ChatContentPart::Image { data_base64, .. } => data_base64.len(), _ => 0,
            }).sum::<usize>();
            turn.steering.prepared_skill_bytes += input.prepared_skill_instructions.iter().map(String::len).sum::<usize>();
            if turn.steering.seen.len() >= 16 || turn.steering.prepared_image_count > 4
                || turn.steering.prepared_image_bytes > 4 * 1024 * 1024
                || turn.steering.prepared_skill_bytes > 24 * 1024 {
                return Err(error("recovery steering exceeds its admitted budget"));
            }
            turn.steering.seen.insert(input.receipt_operation_id.clone(), input.journal_record());
            if turn.voice_control.is_some()&&facts.event_payloads.get(event.event_id.as_ref()).and_then(|payload|payload.get("voice_model_step_supersede")).and_then(serde_json::Value::as_bool)==Some(true) {
                turn.steering.immediate_receipts.insert(input.receipt_operation_id.clone());
            }
            turn.steering.pending.push(input);
        }
        turn.steering.generation = Some(turn.epoch as u64);
        turn.steering.open = recovery.is_some();
        Ok(())
    }

    pub(super) async fn admit_steerable_tool(
        &self,
        message: &nomifun_ai_agent::types::SendMessageData,
        event: &AgentEngineEvent,
    ) -> Result<bool, AppError> {
        if !matches!(event, AgentEngineEvent::ToolStarted { step, .. } if *step > 0) {
            return Err(error(
                "model tool admission requires a positive-step ToolStarted",
            ));
        }
        let mut active = self.active.lock().await;
        let turn = active
            .as_mut()
            .ok_or_else(|| error("tool admission without active turn"))?;
        if turn.root
            != message
                .source_message_id
                .as_deref()
                .unwrap_or(&message.msg_id)
            || turn.wire_id != message.msg_id
            || turn.journal.sequence() < 2
            || turn.cleanup_started
            || turn.cancellation.is_cancelled()
            || !turn.steering.open
            || self
                .activation_failed
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(error("tool admission differs from the live input scope"));
        }
        if !turn.steering.pending.is_empty() {
            return Ok(false);
        }
        // Same lock as accept_steer/take/close, held through durable admission
        // but never through execution. Later steering cannot revoke this call.
        let admission = async {
            self.flush_steering_buffer(turn).await?;
            if turn.cancellation.is_cancelled() {
                return Err(error("turn cancelled before tool admission"));
            }
            self.append_locked_record(
                turn,
                serde_json::to_string(event).map_err(error)?,
                None,
                false,
            )
            .await
        }
        .await;
        if let Err(error) = admission {
            // An uncertain write must not be normalized into a recoverable
            // model tool error followed by further effects in this turn.
            turn.steering.open = false;
            turn.cancellation.cancel();
            return Err(error);
        }
        if turn.voice_control.is_some()&&let AgentEngineEvent::ToolStarted {step,..}=event {turn.steering.admitted_model_steps.insert(*step);}
        Ok(true)
    }

    pub(super) async fn open_steering(&self) -> Result<(), AppError> {
        let mut active = self.active.lock().await;
        let turn = active
            .as_mut()
            .ok_or_else(|| error("no admitted input scope"))?;
        if turn.journal.sequence() != 1 {
            return Err(error("input scope must follow the durable turn root"));
        }
        let payload = serde_json::to_string(&AgentEngineEvent::TurnInputScope {
            wire_turn_id: turn.wire_id.clone(),
        })
        .map_err(error)?;
        self.append_locked_record(turn, payload, None, false)
            .await?;
        turn.steering.open = true;
        Ok(())
    }

    pub(super) async fn accept_steer(
        &self,
        delivery: RuntimeSteerDelivery,
    ) -> Result<bool, AppError> {
        let mut active = self.active.lock().await;
        let Some(turn) = active.as_mut() else {
            return Ok(false);
        };
        self.accept_steer_locked(turn,delivery,None,None).await
    }

    pub(super) async fn admit_steer_atomic(&self,admission:nomifun_ai_agent::RuntimeSteerAdmission)
        ->Result<nomifun_ai_agent::RuntimeSteerAdmissionOutcome,AppError> {
        let host=self.self_reference.upgrade().ok_or_else(||error("input admission owner has shut down"))?;
        // The existing resource task group retains the join witness. Dropping
        // an HTTP waiter cannot cut between durable acceptance and inbox push.
        let operation=self.resources.own_input_operation(async move {host.admit_steer_owned(admission).await})?;
        operation.result().await.map_err(|_|AppError::Internal("owned input operation ended without a confirmed result; retry the original key".into()))?
    }

    async fn admit_steer_owned(&self,admission:nomifun_ai_agent::RuntimeSteerAdmission)
        ->Result<nomifun_ai_agent::RuntimeSteerAdmissionOutcome,AppError> {
        let mut active=self.active.lock().await;
        let turn=active.as_mut().ok_or_else(||error("no live input scope before steering admission"))?;
        if !turn.steering.open || turn.cleanup_started || turn.cancellation.is_cancelled()
            || self.activation_failed.load(std::sync::atomic::Ordering::Acquire) || turn.journal.sequence()==0
            || turn.wire_id!=admission.wire_turn_id || turn.epoch as u64!=admission.turn_generation {
            return Err(error("the exact Runtime input scope closed or changed before admission"));
        }
        let prepared=self.prepare_steer_content_locked(turn,admission.text,admission.files,admission.inject_skills).await.map_err(|failure|failure.error)?;
        turn.journal.verify_execution_lease().await.map_err(error)?;
        // take(true), tool admission and cleanup use this same mutex. The
        // canonical write cannot fall into a closed-memory/open-DB window.
        let committed=admission.commit.await?;
        let delivery=if committed.delivery.wire_turn_id!=turn.wire_id
            || committed.delivery.turn_generation!=turn.epoch as u64
            || committed.acknowledgement.agent_session_id.as_ref()!=self.options.conversation_id
            || committed.acknowledgement.cursor.agent_session_id!=committed.acknowledgement.agent_session_id
            || committed.acknowledgement.seq!=committed.acknowledgement.cursor.seq
            || committed.acknowledgement.event_id.as_ref()!=committed.delivery.receipt_operation_id {
            Err(error("committed steering differs from its exact admission scope"))
        } else {self.accept_steer_locked(turn,committed.delivery,Some(&committed.acknowledgement),Some(prepared)).await};
        Ok(nomifun_ai_agent::RuntimeSteerAdmissionOutcome {acknowledgement:committed.acknowledgement,
            duplicate:committed.duplicate,delivery})
    }

    async fn prepare_steer_content_locked(&self,turn:&ActiveTurn,text:String,files:Vec<String>,inject_skills:Vec<String>)
        ->Result<PreparedSteeringContent,SteeringPreparationFailure> {
        if text.len()>16*1024 || (text.trim().is_empty()&&files.is_empty()&&inject_skills.is_empty()) {
            return Err(preparation_budget(error("steering requires nonempty content and text <=16 KiB")));
        }
        // The empty identities below reserve only wire size; they are never
        // accepted, persisted or delivered as a receipt. The writer supplies
        // two UUID identities (36 bytes each) after successful preparation.
        #[derive(serde::Serialize)]
        struct Envelope<'a> {receipt_operation_id:&'a str,message_id:&'a str,text:&'a str,
            #[serde(skip_serializing_if="Vec::is_empty")] files:&'a Vec<String>,
            #[serde(skip_serializing_if="Vec::is_empty")] inject_skills:&'a Vec<String>,image_count:usize}
        let envelope=serde_json::to_vec(&Envelope {receipt_operation_id:"",message_id:"",text:&text,
            files:&files,inject_skills:&inject_skills,image_count:0}).map_err(error)?;
        if envelope.len().saturating_add(72)>64*1024 {return Err(preparation_budget(error("steering input exceeds its bounded receipt envelope")));}
        let value=serde_json::json!({"files":files,"inject_skills":inject_skills});
        let files=super::super::runtime_attachments::references(&value)?;
        let inject_skills=super::super::runtime_attachments::selected_skills(&value)?;
        if turn.steering.seen.len()>=16 {return Err(preparation_budget(error("turn steering limit reached (16 inputs)")));}
        let capabilities=self.capability_state.snapshot().map_err(|failure|preparation_skill(error(failure)))?;
        self.skills.validate_active(&inject_skills,&capabilities.active).map_err(preparation_skill)?;
        let prepared_skill_instructions=self.skills.explicit_instructions(&inject_skills).map_err(preparation_skill)?;
        let prepared_images=tokio::select! { biased;
            _=turn.cancellation.cancelled()=>return Err(SteeringPreparationFailure {error:error("Turn cancelled before steering preparation completed"),reason:None}),
            result=tokio::time::timeout(std::time::Duration::from_secs(10),
                super::super::runtime_attachments::prepare_images(&files,&self.options.extra,self.route_image_input))=>
                result.map_err(|_|error("steering image preparation timed out before input admission"))??,
        };
        let image_bytes=prepared_images.iter().map(|part|match part {
            nomifun_chat_model_broker::ChatContentPart::Image {data_base64,..}=>data_base64.len(),_=>0,
        }).sum::<usize>();
        let next_image_bytes=turn.steering.prepared_image_bytes.saturating_add(image_bytes);
        let next_image_count=turn.steering.prepared_image_count.saturating_add(prepared_images.len());
        let next_skill_bytes=turn.steering.prepared_skill_bytes.saturating_add(prepared_skill_instructions.iter().map(String::len).sum::<usize>());
        if next_image_bytes>4*1024*1024 || next_image_count>4 || next_skill_bytes>24*1024 {
            return Err(preparation_budget(error("turn steering attachment or selected Skill context budget exceeded before input admission")));
        }
        Ok(PreparedSteeringContent {text,files,inject_skills,prepared_images,prepared_skill_instructions,
            next_image_bytes,next_image_count,next_skill_bytes})
    }

    async fn accept_steer_locked(&self,turn:&mut ActiveTurn,delivery:RuntimeSteerDelivery,ack:Option<&nomifun_agent_contracts::SessionEventAck>,prepared:Option<PreparedSteeringContent>)->Result<bool,AppError> {
        if delivery.receipt_operation_id.len()>1024 || delivery.text.len()>16*1024 {
            return Err(error("steering requires bounded receipt identity and text <=16 KiB"));
        }
        if !turn.steering.open
            || self
                .activation_failed
                .load(std::sync::atomic::Ordering::Acquire)
            || turn.cleanup_started
            || turn.cancellation.is_cancelled()
            || turn.journal.sequence() == 0
            || turn.wire_id != delivery.wire_turn_id
        {
            return Ok(false);
        }
        let store = self.session_host.canonical_store()?;
        let facts = store
            .chat_causality_facts(
                &self.options.conversation_id.clone().into(),
                &turn.operation.clone().into(),
            )
            .await
            .map_err(error)?;
        if facts.head.status != "running"
            || facts.head.active_turn_id.as_deref() != Some(turn.operation.as_str())
        {
            return Ok(false);
        }
        let steering = facts
            .events
            .iter()
            .find(|event| {
                event.event_id.as_ref() == delivery.receipt_operation_id
                    && event.kind.0 == "turn/steer-accepted"
                    && event.correlation_id.as_ref() == turn.operation
            })
            .ok_or_else(|| error("no admitted steering receipt for this turn"))?;
        if ack.is_some_and(|ack|ack.seq!=steering.seq) {return Err(error("steering acknowledgement cursor differs from its original event"));}
        let(original_input,value)=nomifun_agent_session::canonical_steering_input(&facts,steering).map_err(error)?;
        let value=value.0;
        let message_id=original_input.as_ref().to_owned();
        if value.get("content").and_then(|v| v.as_str()) != Some(delivery.text.as_str())
            || delivery.wire_turn_id != turn.wire_id
            || delivery.turn_generation != turn.epoch as u64
            || turn
                .steering
                .generation
                .is_some_and(|generation| generation != delivery.turn_generation)
        {
            return Err(error(
                "steering text/scope differs from its committed receipt",
            ));
        }
        let files = super::super::runtime_attachments::references(&value)?;
        let inject_skills = super::super::runtime_attachments::selected_skills(&value)?;
        if files != delivery.files || inject_skills != delivery.inject_skills {
            return Err(error("steering context differs from its committed receipt"));
        }
        if let Some(previous)=turn.steering.seen.get(&delivery.receipt_operation_id) {
            return if previous.message_id==message_id && previous.text==delivery.text && previous.files==files
                && previous.inject_skills==inject_skills {Ok(true)} else {Err(error("steering identity was reused"))};
        }
        let prepared=match prepared {
            Some(prepared)=>prepared,
            None=>self.prepare_steer_content_locked(turn,delivery.text.clone(),files.clone(),inject_skills.clone()).await.map_err(|failure|failure.error)?,
        };
        if prepared.text!=delivery.text || prepared.files!=files || prepared.inject_skills!=inject_skills {
            return Err(error("committed steering differs from its prepared input"));
        }
        let input=AgentSteeringInput {receipt_operation_id:delivery.receipt_operation_id,message_id,
            text:prepared.text,files:prepared.files,inject_skills:prepared.inject_skills,
            image_count:prepared.prepared_images.len(),prepared_images:prepared.prepared_images,
            prepared_skill_instructions:prepared.prepared_skill_instructions};
        input.validate().map_err(error)?;
        // Attachment reads may outlive a database-side stop. Recheck the
        // durable receipt/turn authority before the in-memory acknowledgement.
        let still_admitted = store
            .read_turn_receipt(
                &self.options.conversation_id.clone().into(),
                &turn.operation.clone().into(),
            )
            .await
            .map_err(error)?;
        if still_admitted.status != nomifun_agent_session::TurnReceiptStatus::Running {
            return Ok(false);
        }
        if turn.cancellation.is_cancelled() {
            return Ok(false);
        }
        // Receipt was already committed by the Conversation owner. No await
        // between queue insertion and acknowledgement; read failures queued nothing.
        turn.steering.generation = Some(delivery.turn_generation);
        turn.steering.prepared_image_bytes = prepared.next_image_bytes;
        turn.steering.prepared_image_count = prepared.next_image_count;
        turn.steering.prepared_skill_bytes = prepared.next_skill_bytes;
        let recorded = input.journal_record();
        turn.steering
            .seen
            .insert(input.receipt_operation_id.clone(), recorded);
        turn.steering.pending.push(input);
        turn.event_buffer.content_boundary();
        turn.steering.changed.notify_waiters();
        if turn.voice_control.is_some()&&facts.event_payloads.get(steering.event_id.as_ref()).and_then(|payload|payload.get("voice_model_step_supersede")).and_then(serde_json::Value::as_bool)==Some(true) {
            turn.steering.immediate_receipts.insert(steering.event_id.as_ref().to_owned());
            if let Some(port)=&turn.voice_control{port.notify();}
        }
        Ok(true)
    }

    pub(super) async fn close_steering(&self) -> Result<(), AppError> {
        let mut active = self.active.lock().await;
        let Some(turn) = active.as_mut() else {
            return Ok(());
        };
        turn.steering.open = false;
        turn.cleanup_started = true;
        if !turn.steering.pending.is_empty() {
            self.flush_steering_buffer(turn).await?;
            let event = AgentEngineEvent::SteeringDeferred { inputs: turn.steering.pending.iter().map(AgentSteeringInput::journal_record).collect(),
                reason: "Turn ended or was cancelled before these queued instructions reached the next model boundary. No automatic retry or new turn was started.".into() };
            self.append_locked_record(
                turn,
                serde_json::to_string(&event).map_err(error)?,
                None,
                false,
            )
            .await?;
            turn.steering.pending.clear();
            turn.steering.immediate_receipts.clear();
        }
        Ok(())
    }

    async fn flush_steering_buffer(&self, turn: &mut ActiveTurn) -> Result<(), AppError> {
        let mut records = Vec::new();
        turn.event_buffer.flush(&mut records);
        if turn.cleanup_started {
            // Cleanup may retry only its exact durable record. Retain each
            // projected tail until acknowledgement instead of consuming a
            // temporary vector before a fallible write.
            turn.cleanup_records.extend(records);
            return self.flush_cleanup_records(turn).await;
        }
        for event in records {
            self.append_locked_record(
                turn,
                serde_json::to_string(&event).map_err(error)?,
                None,
                false,
            )
            .await?;
        }
        Ok(())
    }
}

#[async_trait]
impl AgentInputPort for HostPort {
    async fn take(
        &self,
        causality: &ChatCausality,
        close_if_empty: bool,
    ) -> Result<Vec<AgentSteeringInput>, AgentEngineError> {
        let host = self
            .0
            .upgrade()
            .ok_or_else(|| engine_error("Session host has shut down"))?;
        let causality=causality.clone();let owned_host=host.clone();
        // Closing/claiming/delivering is one owned safe-boundary operation.
        // Cancelling the model waiter cannot strand a durable claim between
        // the canonical Store and its native SteeringInputs reference.
        let operation=host.resources.own_input_operation(async move {
            owned_host.take_session_inputs_owned(causality,close_if_empty).await
        }).map_err(engine_error)?;
        operation.result().await.map_err(engine_error)?
    }

    async fn has_pending(&self, causality: &ChatCausality) -> Result<bool, AgentEngineError> {
        let host = self
            .0
            .upgrade()
            .ok_or_else(|| engine_error("Session host has shut down"))?;
        let active = host.active.lock().await;
        let turn = active
            .as_ref()
            .ok_or_else(|| engine_error("no active turn"))?;
        admitted(turn, &host, causality)?;
        if !turn.steering.pending.is_empty() {return Ok(true);}
        if !turn.steering.open {return Ok(false);}
        let session=host.options.conversation_id.clone().into();let store=host.session_host.canonical_store().map_err(engine_error)?;
        if store.has_unsettled_effects(&session).await.map_err(engine_error)? {return Ok(false);}
        if host.session_host.session_owner().map_err(engine_error)?.require_ordinary_owner_input(&host.principal,&session).await.is_err() {return Ok(false);}
        let operation=turn.operation.clone().into();
        let mut inputs=store.pending_session_inputs_for_steering(&session,&operation,None).await.map_err(engine_error)?;
        for id in &turn.steering.retry_inputs {
            inputs.extend(store.pending_session_inputs_for_steering(&session,&operation,Some(&id.clone().into())).await.map_err(engine_error)?);
        }
        for input in inputs {
            if !turn.steering.deferred_inputs.contains(input.event.event_id.as_ref()) {return Ok(true);}
        }
        Ok(false)
    }
}

#[async_trait]
impl nomifun_agent_runtime::AgentWaitPort for HostPort {
    async fn wait(&self, causality: &ChatCausality, request: &nomifun_agent_runtime::AgentWaitRequest,
        cancellation: tokio_util::sync::CancellationToken)
        -> Result<nomifun_agent_runtime::AgentWaitWake, AgentEngineError> {
        let host = self.0.upgrade().ok_or_else(|| engine_error("Session host has shut down"))?;
        let (changed, operation) = {
            let active = host.active.lock().await;
            let turn = active.as_ref().ok_or_else(|| engine_error("wait has no active Turn"))?;
            admitted(turn, &host, causality)?;
            turn.journal.verify_execution_lease().await.map_err(engine_error)?;
            (turn.steering.changed.clone(), turn.operation.clone())
        };
        if cancellation.is_cancelled() { return Err(AgentEngineError::Cancelled); }
        host.resources.yield_browser_for_parked_wait(&operation, &cancellation).await
            .map_err(|_| AgentEngineError::WaitOwnerUnavailable)?;
        loop {
            // Enable the notification before checking the durable receipt-backed
            // inbox; input accepted on either side of this boundary cannot vanish.
            let notified = changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if cancellation.is_cancelled() { return Err(AgentEngineError::Cancelled); }
            if self.has_pending(causality).await? { return Ok(nomifun_agent_runtime::AgentWaitWake::InputsAvailable); }
            tokio::select! { biased;
                _ = cancellation.cancelled() => return Err(AgentEngineError::Cancelled),
                _ = &mut notified => {},
                observed = host.resources.wait_processes(&operation, &request.processes, request.wake_policy, cancellation.clone()), if !request.processes.is_empty() => {
                    return observed.map(nomifun_agent_runtime::AgentWaitWake::ProcessesChanged)
                        .map_err(|_| AgentEngineError::WaitOwnerUnavailable);
                },
            }
        }
    }
}
