//! Opt-in voice consumer of the original canonical work and decision owners.
//! No Message projection, private transcript, tool dispatcher, or Runtime.
use std::collections::{BTreeMap,BTreeSet};
use std::sync::{Arc,Mutex};
use async_trait::async_trait;
use nomifun_agent_contracts::{AgentSessionId,AgentBindingValue,OperationId,PrincipalRef,SessionEventRecord,UserId,digest_bytes};
use nomifun_agent_session::{ChatCausalityFacts,NativeTurnMutationFence,TurnReceiptStatus};
use nomifun_api_types::{AgentBindingValueDto,SendMessageRequest};
use nomifun_agent_control_plane::AgentControlPlane;
use nomifun_voice::{SharedVoiceJournal,VoicePendingIntent,VoicePendingPhase,VoiceWorkContext,VoiceWorkPort};
use nomifun_voice_contracts::*;
use serde::Deserialize;
use serde_json::{Value,json};
use tokio_util::sync::CancellationToken;
use super::nomi_core_session::NomiCoreSessionOwner;

#[derive(Clone)]
pub(crate) struct AppVoiceWorkHost {
    sessions:Arc<NomiCoreSessionOwner>,journal:Arc<SharedVoiceJournal>,control_plane:Arc<AgentControlPlane>,
    execution:Option<Arc<nomifun_agent_execution::AgentExecutionEngine>>,
    workers:Arc<Mutex<BTreeSet<(String,String)>>>,stop:CancellationToken,
}
impl AppVoiceWorkHost {
    pub(crate) fn new(sessions:Arc<NomiCoreSessionOwner>,journal:Arc<SharedVoiceJournal>,control_plane:Arc<AgentControlPlane>,execution:Option<Arc<nomifun_agent_execution::AgentExecutionEngine>>)->Self {
        Self {sessions,journal,control_plane,execution,workers:Default::default(),stop:CancellationToken::new()}
    }
    pub(crate) fn with_shutdown_token(mut self,stop:CancellationToken)->Self {self.stop=stop;self}
    fn principal(owner:&str)->PrincipalRef {PrincipalRef {principal_kind:"user".into(),principal_id:owner.into()}}
    async fn owned_facts(&self,owner:&str,session:&AgentSessionId)->Result<ChatCausalityFacts,VoiceError> {
        let store=self.sessions.canonical().store();let owned=store.get_live_session(session).await.map_err(work_error)?;
        if owned.owner_ref!=Self::principal(owner){return Err(VoiceError::new(VoiceErrorKind::Authentication,"voice context belongs to another owner"));}
        let facts=store.chat_causality_facts(session,&OperationId::from("voice-context-reader")).await.map_err(work_error)?;
        if facts.session.owner_ref!=Self::principal(owner){return Err(VoiceError::new(VoiceErrorKind::Authentication,"voice context owner changed"));}Ok(facts)
    }
    async fn binding(&self,owner:&str,session:&AgentSessionId,version:u64)->Result<ChatCausalityFacts,VoiceError> {
        let facts=self.owned_facts(owner,session).await?;
        if version==0||facts.session.agent_binding.binding_version!=version{return Err(VoiceError::new(VoiceErrorKind::StaleBinding,"voice context no longer matches its exact Agent binding"));}Ok(facts)
    }
    async fn instructions(&self,owner:&str,binding:&AgentBindingValue)->Result<String,VoiceError> {
        let wire:AgentBindingValueDto=serde_json::from_value(serde_json::to_value(binding).map_err(work_error)?).map_err(work_error)?;
        let (saved,revision,snapshot)=self.control_plane.saved_binding_artifacts(&UserId::from(owner.to_owned()),&wire).await.map_err(work_error)?;
        if saved!=*binding||snapshot.snapshot_ref!=binding.resolved_snapshot_ref||revision.reference!=binding.preset_revision_ref{return Err(VoiceError::new(VoiceErrorKind::StaleBinding,"frozen Agent instructions do not match the canonical binding"));}
        Ok(bounded(&format!("{}\n\n{}",revision.payload.persona,revision.payload.instructions),16*1024))
    }
    async fn target(&self,owner:&str,session:&AgentSessionId,operation:&OperationId)->Result<WorkTarget,VoiceError> {
        let facts=self.sessions.canonical().store().turn_output_facts(session,operation).await.map_err(work_error)?;
        if facts.session.owner_ref!=Self::principal(owner){return Err(VoiceError::new(VoiceErrorKind::Authentication,"work belongs to another owner"));}
        Ok(WorkTarget {agent_session_id:session.clone(),binding_version:facts.session.agent_binding.binding_version,turn_operation_id:operation.clone(),execution_generation:facts.execution_generation})
    }
    async fn exact_facts(&self,owner:&str,target:&WorkTarget)->Result<ChatCausalityFacts,VoiceError> {
        let facts=self.sessions.canonical().store().turn_output_facts(&target.agent_session_id,&target.turn_operation_id).await.map_err(work_error)?;
        if facts.session.owner_ref!=Self::principal(owner)||facts.session.agent_binding.binding_version!=target.binding_version||facts.execution_generation!=target.execution_generation {
            return Err(VoiceError::new(VoiceErrorKind::StaleBinding,"work binding or native generation changed; the original target was not replaced"));
        }Ok(facts)
    }
    fn pending_id(key:&str)->String {format!("voice-input:{key}")}
    async fn input_receipt(&self,input:VoicePendingIntent,duplicate:bool)->Result<VoiceWorkReceipt,VoiceError> {
        let session=AgentSessionId::from(input.agent_session_id.clone());
        // Always query this input's original operation key. A queued input can
        // become canonical while its dispatcher is still awaiting acknowledgement.
        let turn=self.sessions.voice_operation_receipt(&input.owner_id,&session,&input.operation_key).await.map_err(work_error)?;
        if turn.status!=TurnReceiptStatus::NotFound {
            let target=self.target(&input.owner_id,&session,&turn.operation_id).await?;let mut receipt=self.observe(&input.owner_id,&target).await?;
            receipt.operation_key=input.operation_key.clone();receipt.pending_input_id=Some(Self::pending_id(&input.operation_key));receipt.pending_input_revision=Some(input.revision);receipt.duplicate=duplicate;return Ok(receipt);
        }
        let (status,summary)=match input.phase {
            VoicePendingPhase::Queued=>(VoiceWorkStatus::Queued,"The explicit voice input is queued; no canonical task admission is confirmed."),
            VoicePendingPhase::Dispatched=>(VoiceWorkStatus::PendingBoundary,"The original dispatch is awaiting a canonical receipt. It will not be blindly submitted again."),
            VoicePendingPhase::Cancelled=>(VoiceWorkStatus::Terminal,"Only this unadmitted voice input was cancelled. Desktop work continues."),
            VoicePendingPhase::Deferred=>(VoiceWorkStatus::Deferred,"The unadmitted voice input was deferred. It did not start with another binding or context."),
        };
        Ok(VoiceWorkReceipt {receipt_id:Self::pending_id(&input.operation_key),operation_key:input.operation_key.clone(),target:None,pending_input_id:Some(Self::pending_id(&input.operation_key)),status,
            summary:summary.into(),duplicate,speech:None,pending_input_revision:Some(input.revision),first_claim_generation:None})
    }
    fn schedule(&self,owner:String,session:String) {
        let identity=(owner.clone(),session.clone());if self.stop.is_cancelled(){return;}
        if !self.workers.lock().unwrap_or_else(|error|error.into_inner()).insert(identity.clone()){return;}
        let host=self.clone();let worker_identity=identity.clone();
        if !self.sessions.voice_register_task(Box::pin(async move {
            host.drain_inputs(owner,session).await;
            host.workers.lock().unwrap_or_else(|error|error.into_inner()).remove(&worker_identity);
        })) {self.workers.lock().unwrap_or_else(|error|error.into_inner()).remove(&identity);}
    }
    async fn input_namespace_matches(&self,journal:&nomifun_voice::VoiceJournal,input:&VoicePendingIntent)->bool {
        let Ok(Some(original))=journal.original_input_namespace(input.owner_id.clone(),input.agent_session_id.clone(),input.operation_key.clone()).await else{return false;};
        let data_dir=self.journal.data_dir().to_owned();
        matches!(tokio::task::spawn_blocking(move||super::mobile_voice_authority::current_voice_namespace(&data_dir)).await,Ok(Ok(current)) if current==original)
    }
    pub(super) async fn drain_inputs(&self,owner:String,session:String) {
        let Some(journal)=self.journal.opened().await else{return;};let id=AgentSessionId::from(session.clone());
        loop {
            if self.stop.is_cancelled(){break;}
            let queued=match journal.queued_intents().await {Ok(inputs)=>inputs,Err(_)=>return};
            let Some(input)=queued.into_iter().find(|input|input.owner_id==owner&&input.agent_session_id==session) else{return;};
            match self.sessions.voice_operation_receipt(&owner,&id,&input.operation_key).await {
                Ok(turn) if turn.status!=TurnReceiptStatus::NotFound=>{
                    // Canonical admission is authoritative even if its old
                    // activation/profile no longer matches. Repair by lookup
                    // of this original key, never replayed send or a new driver.
                    let _=journal.begin_dispatch_intent(owner.clone(),session.clone(),input.operation_key.clone(),input.revision).await;
                    let _=journal.mark_dispatched(owner.clone(),session.clone(),input.operation_key.clone(),input.revision,turn.operation_id.as_ref().into(),turn.operation_id.as_ref().into()).await;continue;
                }
                Err(_)=>return,_=>{}
            }
            if !self.input_namespace_matches(&journal,&input).await {
                let _=journal.defer_intent(owner.clone(),session.clone(),input.operation_key.clone()).await;continue;
            }
            let facts=match self.binding(&owner,&id,input.binding_version).await {Ok(facts)=>facts,Err(_)=>{
                let _=journal.defer_intent(owner.clone(),session.clone(),input.operation_key.clone()).await;continue;
            }};
            if input.context_floor!=Some(context_floor(&facts)) {let _=journal.defer_intent(owner.clone(),session.clone(),input.operation_key.clone()).await;continue;}
            if facts.head.status!="ready"||facts.head.active_turn_id.is_some() {
                tokio::select! {biased;_ = self.stop.cancelled()=>break,_ = tokio::time::sleep(std::time::Duration::from_millis(250))=>{}}continue;
            }
            if !matches!(journal.begin_dispatch_intent(owner.clone(),session.clone(),input.operation_key.clone(),input.revision).await,Ok(true)){continue;}
            // Recheck immediately before crossing databases. A restoration
            // during a boundary wait cannot authorize the old sidecar input.
            if !self.input_namespace_matches(&journal,&input).await {
                let _=journal.defer_intent(owner.clone(),session.clone(),input.operation_key.clone()).await;continue;
            }
            let result=self.sessions.voice_start_message_with_policy(&owner,&id,&input.operation_key,voice_message(input.text.clone()),input.binding_version,input.context_floor.unwrap_or(0),&self.stop,input.work_steering_policy==WorkSteeringPolicy::SupersedeModelStep).await;
            let turn=self.sessions.voice_operation_receipt(&owner,&id,&input.operation_key).await;
            match turn {
                Ok(turn) if turn.status!=TurnReceiptStatus::NotFound=>{let _=journal.mark_dispatched(owner.clone(),session.clone(),input.operation_key.clone(),input.revision,turn.operation_id.as_ref().into(),turn.operation_id.as_ref().into()).await;}
                Ok(_) if matches!(result,Ok(None))&&!self.stop.is_cancelled()=>{let _=journal.requeue_without_admission(owner.clone(),session.clone(),input.operation_key.clone(),input.revision).await;
                    tokio::select! {biased;_ = self.stop.cancelled()=>break,_ = tokio::time::sleep(std::time::Duration::from_millis(250))=>{}}
                }
                Ok(_) if self.stop.is_cancelled()=>{let _=journal.defer_intent(owner.clone(),session.clone(),input.operation_key.clone()).await;}
                // An ambiguous dispatch remains a frozen input fact. Lookup
                // may repair it; absence never authorizes a second submission.
                _=>{}
            }
        }
        if let Ok(inputs)=journal.queued_intents().await {for input in inputs.into_iter().filter(|input|input.owner_id==owner&&input.agent_session_id==session) {
            let _=journal.defer_intent(owner.clone(),session.clone(),input.operation_key).await;
        }}
    }

    async fn control_input(&self,owner:&str,session:&str,key:&str,pending:&str,revised:Option<(u64,String)>)->Result<VoiceWorkReceipt,VoiceError> {
        let original=pending.strip_prefix("voice-input:").filter(|key|!key.is_empty()).ok_or_else(||VoiceError::new(VoiceErrorKind::Configuration,"voice queue control requires its original input fact"))?;
        let journal=self.journal.opened().await.ok_or_else(||work_error("voice input journal is not open"))?;
        let turn=self.sessions.voice_operation_receipt(owner,&session.into(),original).await.map_err(work_error)?;
        if turn.status!=TurnReceiptStatus::NotFound{return Err(VoiceError::new(VoiceErrorKind::StaleBinding,"This input crossed canonical admission. Observe its original Turn before controlling work."));}
        let (input,duplicate)=journal.control_pending_intent(owner.into(),session.into(),original.into(),key.into(),revised,false).await?;
        let mut receipt=self.input_receipt(input,duplicate).await?;receipt.operation_key=key.into();self.schedule(owner.into(),session.into());Ok(receipt)
    }
    async fn control_record(&self,owner:&str,session:&str,key:&str)->Result<Option<(SessionEventRecord,Value)>,VoiceError> {
        let facts=self.owned_facts(owner,&session.into()).await?;
        for suffix in ["cancel","steer"] {
            let scoped=format!("user:{owner}:{session}:{key}:{suffix}");
            if let Some(event)=facts.events.iter().find(|event|event.producer_id.as_ref()=="session_api"&&event.idempotency_key.as_ref()==scoped
                &&matches!(event.kind.0.as_str(),"turn/cancelled"|"turn/steer-accepted")) {
                let payload=facts.event_payloads.get(event.event_id.as_ref()).ok_or_else(||work_error("control has no payload"))?.clone();
                if payload.get("native_target_fence").is_none(){return Err(work_error("voice key belongs to a different command source"));}
                return Ok(Some((event.clone(),payload)));
            }
        }Ok(None)
    }
    async fn validate_control_replay(&self,owner:&str,session:&str,key:&str,target:&WorkTarget,input:Option<&SendMessageRequest>,supersede:bool)->Result<(),VoiceError> {
        if let Some((event,payload))=self.control_record(owner,session,key).await? {
            let expected_kind=if input.is_some(){"turn/steer-accepted"}else{"turn/cancelled"};
            if event.kind.0!=expected_kind||event.correlation_id.as_ref()!=target.turn_operation_id.as_ref()
                ||payload.get("native_target_fence")!=Some(&serde_json::to_value(fence(target)).map_err(work_error)?)
                ||input.is_some()&&payload.get("voice_model_step_supersede").and_then(Value::as_bool).unwrap_or(false)!=supersede
                ||input.is_some_and(|input|payload.get("input")!=Some(&super::nomi_core_session::canonical_turn_input(input))) {
                return Err(VoiceError::new(VoiceErrorKind::Configuration,"The operation key belongs to its original complete command and target."));
            }
        }Ok(())
    }
    async fn lookup_control(&self,owner:&str,session:&str,key:&str)->Result<Option<VoiceWorkReceipt>,VoiceError> {
        if let Some((event,payload))=self.control_record(owner,session,key).await? {
            let original:NativeTurnMutationFence=serde_json::from_value(payload.get("native_target_fence").cloned().ok_or_else(||work_error("missing native proof"))?).map_err(work_error)?;
            let operation=OperationId::from(event.correlation_id.as_ref());let facts=self.owned_facts(owner,&session.into()).await?;
            let turn=self.sessions.canonical().turn_receipt(&Self::principal(owner),&session.into(),&operation).await.map_err(work_error)?;
            let output=self.sessions.canonical().store().turn_output_facts(&session.into(),&operation).await.map_err(work_error)?;
            let target=WorkTarget {agent_session_id:session.into(),binding_version:original.binding_version,turn_operation_id:operation.clone(),execution_generation:output.execution_generation};
            let status=if event.kind.0=="turn/cancelled" {VoiceWorkStatus::Terminal}else {steering_status(&facts,&operation,event.event_id.as_ref(),turn.terminal_event.is_some())?};
            return Ok(Some(VoiceWorkReceipt {receipt_id:operation.as_ref().into(),operation_key:key.into(),target:Some(target),pending_input_id:None,status,
                summary:match status {VoiceWorkStatus::Applied=>"The complete correction reached a recorded later model boundary; no tool execution is claimed.",VoiceWorkStatus::Deferred=>"The correction did not reach another recorded model boundary before the task closed.",VoiceWorkStatus::Terminal=>"The exact canonical Turn has a cancellation receipt. Existing owners retain effect cleanup.",
                    _=>"The complete correction is recorded for the original Turn and is pending its normal safe boundary. Immediate interruption is not proven."}.into(),duplicate:true,speech:None,pending_input_revision:None,first_claim_generation:first_native_generation(&output,&operation)?}));
        }
        self.lookup_approval(owner,session,key).await
    }
    fn retry_control_delivery(&self,owner:String,session:String,key:String,target:WorkTarget,text:String,supersede:bool) {
        let identity=(owner.clone(),format!("control:{session}:{key}"));if !self.workers.lock().unwrap_or_else(|error|error.into_inner()).insert(identity.clone()){return;}
        let host=self.clone();let worker_identity=identity.clone();
        if !self.sessions.voice_register_task(Box::pin(async move {
            for _ in 0..100 {
                if host.stop.is_cancelled(){break;}
                let existing=host.sessions.canonical().turn_receipt(&Self::principal(&owner),&target.agent_session_id,&target.turn_operation_id).await;
                if !matches!(existing,Ok(ref turn) if turn.status==TurnReceiptStatus::Running){break;}
                if host.sessions.voice_steer_exact_native_with_policy(&owner,&target.agent_session_id,&key,&target.turn_operation_id,voice_message(text.clone()),&fence(&target),supersede).await.is_ok(){break;}
                tokio::select! {biased;_ = host.stop.cancelled()=>break,_ = tokio::time::sleep(std::time::Duration::from_millis(100))=>{}}
            }
            host.workers.lock().unwrap_or_else(|error|error.into_inner()).remove(&worker_identity);
        })) {self.workers.lock().unwrap_or_else(|error|error.into_inner()).remove(&identity);}
    }
}

fn work_error(error:impl std::fmt::Display)->VoiceError {
    // Canonical source errors do not transport provider wire, raw tools or SQL
    // to the conversational model. Callers get an honest unconfirmed outcome.
    let _=error;VoiceError::new(VoiceErrorKind::StaleBinding,"The canonical owner could not confirm this exact voice work operation.")
}
fn bounded(text:&str,max:usize)->String {let mut end=text.len().min(max);while !text.is_char_boundary(end){end-=1;}text[..end].into()}
fn voice_message(content:String)->SendMessageRequest {SendMessageRequest {content,files:vec![],inject_skills:vec![],hidden:false,origin:Some("mobile_voice".into()),channel_platform:None,plugin_delivery:None}}
fn fence(target:&WorkTarget)->NativeTurnMutationFence {NativeTurnMutationFence {binding_version:target.binding_version,execution_generation:target.execution_generation}}
fn require_target(target:&WorkTarget,session:&AgentSessionId,version:u64)->Result<(),VoiceError> {
    if target.agent_session_id!=*session||target.binding_version!=version{return Err(VoiceError::new(VoiceErrorKind::StaleBinding,"The command belongs to its exact voice interaction Session and binding."));}Ok(())
}
fn steering_status(facts:&ChatCausalityFacts,operation:&OperationId,receipt:&str,closed:bool)->Result<VoiceWorkStatus,VoiceError> {
    let mut taken=false;let mut applied=false;let mut deferred=false;
    for event in facts.events.iter().filter(|event|event.kind.0=="runtime/progress-recorded"&&event.correlation_id.as_ref()==operation.as_ref()) {
        let Some(value)=facts.event_payloads.get(event.event_id.as_ref()).and_then(|payload|payload.get("event")) else{continue;};
        if !matches!(value.get("event").and_then(Value::as_str),Some("steering_inputs"|"steering_deferred"|"model_step_started")){continue;}
        match serde_json::from_value::<nomifun_agent_runtime::AgentEngineEvent>(value.clone()).map_err(work_error)? {
            nomifun_agent_runtime::AgentEngineEvent::SteeringInputs {inputs} if inputs.iter().any(|input|input.receipt_operation_id==receipt)=>taken=true,
            nomifun_agent_runtime::AgentEngineEvent::SteeringDeferred {inputs,..} if inputs.iter().any(|input|input.receipt_operation_id==receipt)=>deferred=true,
            nomifun_agent_runtime::AgentEngineEvent::ModelStepStarted {..} if taken=>applied=true,_=>{}
        }
    }
    Ok(if applied {VoiceWorkStatus::Applied}else if deferred||closed {VoiceWorkStatus::Deferred}else {VoiceWorkStatus::PendingBoundary})
}
fn context_floor(facts:&ChatCausalityFacts)->u64 {facts.events.iter().filter(|event|event.kind.0=="context/cleared").map(|event|event.seq).max().unwrap_or(0)}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FirstNativeClaim {operation_id:OperationId,execution_fence:u64,holder_digest:DigestHex}
fn first_native_generation(facts:&ChatCausalityFacts,operation:&OperationId)->Result<Option<u64>,VoiceError> {
    let Some(start)=facts.events.iter().find(|event|event.kind.0=="turn/started"&&event.kind_version==1&&event.correlation_id.as_ref()==operation.as_ref()) else{return Ok(None);};
    let Some(first)=facts.events.iter().filter(|event|event.kind.0=="runtime/execution-claimed"&&event.correlation_id.as_ref()==operation.as_ref()).min_by_key(|event|event.seq) else{return Ok(None);};
    if first.kind_version!=1||first.producer_id.as_ref()!="runtime_supervisor"||first.causation_event_id.as_ref()!=Some(&start.event_id){return Ok(None);}
    let claim:FirstNativeClaim=serde_json::from_value(facts.event_payloads.get(first.event_id.as_ref()).cloned().ok_or_else(||work_error("native claim has no payload"))?).map_err(work_error)?;
    // The existing canonical native writer gives the ordinary first claim the
    // Turn-start sequence. Recovery uses a later claim sequence and a nonzero
    // fence; it must never be advertised as renewed source authority.
    if claim.operation_id!=*operation||claim.execution_fence!=0||claim.holder_digest.as_ref().len()!=64{return Ok(None);}
    Ok(Some(start.seq))
}

#[derive(Clone,Debug)]
struct PublicMessage {id:String,event_id:String,seq:u64,last_seq:u64,text:String,speaker:VoiceSpeaker,root:Option<String>}
#[derive(Deserialize)]
struct AcceptedText {content:String,#[serde(default)]hidden:bool}
#[derive(Deserialize)]
struct Part {content:String,turn_id:String}
#[derive(Deserialize)]
struct Completed {part_count:u64,content_digest:String}

/// Only a typed, owned model-step supersede can withdraw its one public
/// assistant identity. Neither a Turn status nor an unrelated correction
/// hides other completed steps or changes canonical history.
fn voice_superseded_message_ids(facts:&ChatCausalityFacts)->Result<BTreeSet<String>,VoiceError>{
    use nomifun_agent_runtime::AgentEngineEvent;
    let mut steps=BTreeMap::<(String,u16),(OperationId,u64)>::new();let mut withdrawn=BTreeSet::new();
    let mut ordered=facts.events.iter().collect::<Vec<_>>();ordered.sort_by_key(|event|event.seq);
    for event in ordered {
        if event.kind.0!="runtime/progress-recorded"{continue;}
        let Some(value)=facts.event_payloads.get(event.event_id.as_ref()).and_then(|payload|payload.get("event")) else{continue;};
        if !matches!(value.get("event").and_then(Value::as_str),Some("model_step_started"|"voice_model_step_superseded")){continue;}
        if event.kind_version!=1{return Err(work_error("unsupported voice model source event"));}
        match serde_json::from_value::<AgentEngineEvent>(value.clone()).map_err(work_error)? {
            AgentEngineEvent::ModelStepStarted{step,operation_id}=>{steps.insert((event.correlation_id.as_ref().into(),step),(operation_id,event.seq));},
            AgentEngineEvent::VoiceModelStepSuperseded{step,model_operation_id,steering_receipt_ids,cleanup,..}=>{
                let start=steps.get(&(event.correlation_id.as_ref().into(),step)).ok_or_else(||work_error("voice withdrawal has no exact model start"))?;
                if start.0!=model_operation_id||start.1>=event.seq||cleanup.operation_id!=model_operation_id||cleanup.task_id.is_empty()||steering_receipt_ids.is_empty(){return Err(work_error("voice withdrawal lacks its exact owned model closure"));}
                let turn=facts.events.iter().find(|turn|turn.kind.0=="turn/started"&&turn.kind_version==1&&turn.correlation_id==event.correlation_id&&turn.seq<start.1)
                    .ok_or_else(||work_error("voice withdrawal has no original canonical Turn"))?;
                let root=facts.event_payloads.get(turn.event_id.as_ref()).and_then(|payload|payload.get("source_message_id")).and_then(Value::as_str)
                    .ok_or_else(||work_error("voice withdrawal has no original canonical input"))?;
                if !facts.events.iter().any(|input|input.event_id.as_ref()==root&&input.kind.0=="message/user-accepted"&&input.kind_version==1&&input.seq<=turn.seq){return Err(work_error("voice withdrawal input source is unavailable"));}
                withdrawn.insert(super::engine_journal::canonical_assistant_step_message_id(root,step).map_err(work_error)?);
            },_=>{}
        }
    }Ok(withdrawn)
}

/// Data-only presentation from typed canonical Event/Payload records. No
/// projection mirrors, inferred roles, reasoning, tools, or old transcript.
fn public_messages(facts:&ChatCausalityFacts)->Result<Vec<PublicMessage>,VoiceError> {
    let floor=facts.events.iter().filter(|event|event.kind.0=="context/cleared").map(|event|event.seq).max().unwrap_or(0);
    let mut hidden=BTreeSet::new();let mut hidden_messages=voice_superseded_message_ids(facts)?;let mut parts=BTreeMap::<String,(PublicMessage,u64)>::new();let mut output=Vec::new();
    let mut ordered=facts.events.iter().filter(|event|event.seq>floor).collect::<Vec<_>>();ordered.sort_by_key(|event|event.seq);
    for event in ordered {
        if !matches!(event.kind.0.as_str(),"message/user-accepted"|"turn/steer-accepted"|"message/assistant-projected"|"message/content-part"|"message/completed"){continue;}
        if event.kind_version!=1{return Err(work_error("unsupported public message event"));}
        let value=facts.event_payloads.get(event.event_id.as_ref()).ok_or_else(||work_error("missing canonical public payload"))?;
        match event.kind.0.as_str() {
            "message/user-accepted"|"turn/steer-accepted"=>{
                let value=if event.kind.0=="turn/steer-accepted" {value.get("input").ok_or_else(||work_error("steering has no input"))?}else{value};
                let input:AcceptedText=serde_json::from_value(value.clone()).map_err(work_error)?;
                if input.hidden {hidden.insert(event.event_id.as_ref().to_owned());continue;}
                if !input.content.trim().is_empty(){output.push(PublicMessage {id:event.event_id.as_ref().into(),event_id:event.event_id.as_ref().into(),seq:event.seq,last_seq:event.seq,text:input.content,speaker:VoiceSpeaker::User,root:None});}
            }
            "message/assistant-projected"=>{
                let input:AcceptedText=serde_json::from_value(value.clone()).map_err(work_error)?;
                if input.hidden{hidden_messages.insert(event.correlation_id.as_ref().to_owned());continue;}
                if !input.hidden&&!input.content.trim().is_empty(){output.push(PublicMessage {id:event.correlation_id.as_ref().into(),event_id:event.event_id.as_ref().into(),seq:event.seq,last_seq:event.seq,text:input.content,speaker:VoiceSpeaker::Assistant,root:None});}
            }
            "message/content-part"=>{
                let part:Part=serde_json::from_value(value.clone()).map_err(work_error)?;if hidden.contains(&part.turn_id){hidden_messages.insert(event.correlation_id.as_ref().to_owned());continue;}
                let current=parts.entry(event.correlation_id.as_ref().into()).or_insert_with(||(PublicMessage {id:event.correlation_id.as_ref().into(),event_id:event.event_id.as_ref().into(),seq:event.seq,last_seq:event.seq,text:String::new(),speaker:VoiceSpeaker::Assistant,root:Some(part.turn_id.clone())},0));
                if current.0.root.as_deref()!=Some(part.turn_id.as_str())||current.0.text.len().saturating_add(part.content.len())>8*1024*1024{return Err(work_error("invalid public message part identity or budget"));}
                current.0.text.push_str(&part.content);current.0.last_seq=event.seq;current.0.event_id=event.event_id.as_ref().into();current.1+=1;
            }
            "message/completed"=>{
                let complete:Completed=serde_json::from_value(value.clone()).map_err(work_error)?;
                if hidden_messages.contains(event.correlation_id.as_ref()){continue;}
                if let Some((mut message,count))=parts.remove(event.correlation_id.as_ref()) {
                    if count!=complete.part_count||digest_bytes(message.text.as_bytes()).as_ref()!=complete.content_digest{return Err(work_error("public message completion differs from its actual parts"));}
                    message.last_seq=event.seq;message.event_id=event.event_id.as_ref().into();if !message.text.trim().is_empty(){output.push(message);}
                }else if complete.part_count!=0||complete.content_digest!=digest_bytes(b"").as_ref(){return Err(work_error("completed public message has no canonical parts"));}
            }
            _=>{}
        }
    }
    // Unfinished model output is not promoted to a delivered voice fact. Its
    // state remains visible through the canonical work status observation.
    output.retain(|message|!hidden_messages.contains(&message.id));
    output.sort_by_key(|message|message.seq);Ok(output)
}
fn public_message_fact(facts:&ChatCausalityFacts,message:PublicMessage)->VerifiedVoiceFact{
        let speech_source=(message.speaker==VoiceSpeaker::Assistant).then(||VoiceSpeechSourceRef {message_id:message.id.clone(),revision:message.last_seq,through_seq:message.last_seq});
        let excerpt=bounded(&message.text,2048);
        VerifiedVoiceFact {
        correlation_id:format!("initial:{}",message.event_id),upstream_trigger_id:None,canonical_receipt_id:message.event_id.clone(),
        content:json!({"kind":"canonical_public_message","speaker":message.speaker,"source_event_id":message.event_id,"message_id":message.id,"through_seq":message.last_seq,
            "agent_session_id":facts.session.agent_session_id,"binding_version":facts.session.agent_binding.binding_version,"context_floor":context_floor(facts),"content":excerpt,
            "truncated":message.text.len()>2048,"original_bytes":message.text.len(),"excerpt_not_complete_when_truncated":true,"data_only":true}).to_string(),
        speak:false,output_generation:None,work_context:None,speech_source,
    }
}
fn initial_facts(facts:&ChatCausalityFacts)->Result<Vec<VerifiedVoiceFact>,VoiceError> {
    Ok(public_messages(facts)?.into_iter().rev().take(12).collect::<Vec<_>>().into_iter().rev().map(|message|public_message_fact(facts,message)).collect())
}
fn source_message<'a>(messages:&'a[PublicMessage],source:&VoiceSpeechSourceRef)->Option<&'a PublicMessage>{
    source.validate().ok()?;
    // A prior projection with the same ID does not certify the latest
    // wording. The exact completed source revision is the only valid one.
    messages.iter().filter(|message|message.id==source.message_id&&message.speaker==VoiceSpeaker::Assistant)
        .max_by_key(|message|message.last_seq)
        .filter(|message|message.last_seq==source.revision&&message.last_seq==source.through_seq)
}
fn historical_execution_generation(actual:&ChatCausalityFacts,target:&WorkTarget,started_seq:Option<u64>)->bool{
    target.execution_generation==0||target.execution_generation==actual.execution_generation||started_seq==Some(target.execution_generation)
        ||actual.events.iter().any(|event|event.kind.0=="runtime/execution-claimed"&&event.correlation_id.as_ref()==target.turn_operation_id.as_ref()&&event.seq==target.execution_generation)
}

#[async_trait]
impl VoiceWorkPort for AppVoiceWorkHost {
    async fn recover_pending_inputs(&self,owner:&str,session:&str)->Result<(),VoiceError> {
        if self.stop.is_cancelled(){return Err(VoiceError::new(VoiceErrorKind::Closed,"voice work recovery is shutting down"));}
        self.owned_facts(owner,&session.into()).await?;
        let Some(journal)=self.journal.opened().await else{return Ok(());};
        if journal.queued_intents().await?.iter().any(|input|input.owner_id==owner&&input.agent_session_id==session){self.schedule(owner.into(),session.into());}
        Ok(())
    }
    async fn shutdown_pending_inputs(&self)->Result<(),VoiceError> {
        self.stop.cancel();if let Some(journal)=self.journal.opened().await {for input in journal.queued_intents().await? {
            journal.defer_intent(input.owner_id,input.agent_session_id,input.operation_key).await?;
        }}Ok(())
    }
    async fn validate_initial_facts(&self,owner:&str,session:&str,facts:&[VerifiedVoiceFact])->Result<bool,VoiceError> {
        let authority=self.owned_facts(owner,&session.into()).await?;
        // The initial display window is bounded to twelve messages. Source
        // validity is established against all typed canonical public facts,
        // so normal later messages do not revoke an earlier immutable fact.
        let current=public_messages(&authority)?.into_iter().map(|message|public_message_fact(&authority,message)).collect::<Vec<_>>();
        for fact in facts {
            if fact.speak||fact.upstream_trigger_id.is_some()||fact.output_generation.is_some(){return Ok(false);}
            if current.contains(fact){continue;}
            if let Some(context)=&fact.work_context {
                let receipt:VoiceWorkReceipt=match serde_json::from_str(&fact.content){Ok(receipt)=>receipt,Err(_)=>return Ok(false)};
                let Some(target)=receipt.target.as_ref() else{return Ok(false);};
                if context.target.as_ref()!=Some(target)||target.agent_session_id.as_ref()!=session||fact.canonical_receipt_id!=target.turn_operation_id.as_ref()
                    ||receipt.receipt_id!=target.turn_operation_id.as_ref()||fact.correlation_id!=receipt.operation_key||receipt.operation_key!=target.turn_operation_id.as_ref()
                    ||receipt.pending_input_id.is_some()||receipt.pending_input_revision.is_some(){return Ok(false);}
                let actual=match self.sessions.canonical().store().turn_output_facts(&target.agent_session_id,&target.turn_operation_id).await {Ok(actual)=>actual,Err(_)=>return Ok(false)};
                if actual.session.owner_ref!=Self::principal(owner)||actual.session.agent_binding.binding_version!=target.binding_version{return Ok(false);}
                let turn=self.sessions.canonical().turn_receipt(&Self::principal(owner),&target.agent_session_id,&target.turn_operation_id).await.map_err(work_error)?;
                // Historical acceptance before the first model claim remains
                // a valid fact. Never reuse it as a current mutation fence.
                if !historical_execution_generation(&actual,target,turn.started_event.as_ref().map(|event|event.seq)){return Ok(false);}
                let delivery=nomifun_agent_execution::canonical_turn_delivery(&actual,&turn,true).map_err(work_error)?;
                let historical=match receipt.status {
                    VoiceWorkStatus::Accepted=>turn.started_event.is_some()&&receipt.summary==if target.execution_generation==0 {"The canonical task is accepted and has not claimed a native model generation."}else{"The canonical task remains active."},
                    VoiceWorkStatus::Terminal=>turn.terminal_event.is_some()&&receipt.summary==if delivery.completed&&delivery.result_ok==Some(true) {"The exact canonical task completed."}else{"The exact canonical task closed; no further execution is claimed."},
                    VoiceWorkStatus::PendingBoundary=>(turn.terminal_event.is_some()||actual.events.iter().any(|event|event.kind.0=="turn/paused"&&event.correlation_id.as_ref()==target.turn_operation_id.as_ref()))
                        &&matches!(receipt.summary.as_str(),"The exact task is paused at its canonical boundary."|"The exact canonical task closed; no further execution is claimed."),
                    _=>false,
                };
                if !historical{return Ok(false);}
                if let Some(speech)=&receipt.speech {
                    if !self.validate_speech(owner,session,speech).await?||fact.speech_source.as_ref()!=Some(&VoiceSpeechSourceRef {message_id:speech.message_id.clone(),revision:speech.revision,through_seq:speech.through_seq}){return Ok(false);}
                }else if fact.speech_source.is_some(){return Ok(false);}
                continue;
            }
            let presentation:VoiceApprovalPresentation=match serde_json::from_str(&fact.content){Ok(presentation)=>presentation,Err(_)=>return Ok(false)};
            if fact.speech_source.is_some()||fact.correlation_id!=format!("approval:{}",presentation.target.presentation_id)
                ||fact.canonical_receipt_id!=format!("{}:{}",presentation.target.execution_id,presentation.target.request_event_sequence)
                ||!self.approvals(owner,session,authority.session.agent_binding.binding_version).await?.contains(&presentation){return Ok(false);}
        }Ok(true)
    }
    async fn validate_speech(&self,owner:&str,session:&str,speech:&VoiceSpeechProjection)->Result<bool,VoiceError> {
        let source=VoiceSpeechSourceRef{message_id:speech.message_id.clone(),revision:speech.revision,through_seq:speech.through_seq};
        let messages=public_messages(&self.owned_facts(owner,&session.into()).await?)?;
        Ok(!speech.text.trim().is_empty()&&source_message(&messages,&source).is_some_and(|message|message.text.starts_with(&speech.text)))
    }
    async fn validate_source_ref(&self,owner:&str,session:&str,source:&VoiceSpeechSourceRef)->Result<bool,VoiceError>{
        Ok(source_message(&public_messages(&self.owned_facts(owner,&session.into()).await?)?,source).is_some())
    }
    async fn validate_source_refs(&self,owner:&str,session:&str,sources:&[VoiceSpeechSourceRef])->Result<Vec<VoiceSpeechSourceRef>,VoiceError>{
        let messages=public_messages(&self.owned_facts(owner,&session.into()).await?)?;
        let mut latest=BTreeMap::<&str,&PublicMessage>::new();
        for message in &messages{if message.speaker==VoiceSpeaker::Assistant&&latest.get(message.id.as_str()).is_none_or(|prior|prior.last_seq<message.last_seq){latest.insert(message.id.as_str(),message);}}
        Ok(sources.iter().filter(|source|source.validate().is_err()||!latest.get(source.message_id.as_str()).is_some_and(|message|message.last_seq==source.revision&&message.last_seq==source.through_seq)).cloned().collect())
    }
    async fn context(&self,owner:&str,session:&str,version:u64)->Result<VoiceWorkContext,VoiceError> {
        let id=AgentSessionId::from(session);let facts=self.binding(owner,&id,version).await?;
        let observed_target=match facts.head.active_turn_id.as_ref(){Some(operation)=>Some(self.target(owner,&id,&operation.clone().into()).await?),None=>None};
        let mut work=Vec::new();if let Some(target)=&observed_target {work.push(self.observe(owner,target).await?);}
        let context_reference=facts.events.iter().max_by_key(|event|event.seq).map(|event|event.event_id.as_ref().to_owned()).ok_or_else(||work_error("canonical context has no actual event reference"))?;
        Ok(VoiceWorkContext {instructions:self.instructions(owner,&facts.session.agent_binding).await?,context_reference,context_floor:context_floor(&facts),initial_facts:initial_facts(&facts)?,observed_target,facts:work,approvals:self.approvals(owner,session,version).await?})
    }
    async fn interact(&self,owner:&str,session:&str,version:u64,key:&str,request:VoiceWorkRequest)->Result<VoiceWorkReceipt,VoiceError> {
        self.interact_with_policy(owner,session,version,key,request,WorkSteeringPolicy::SafeBoundary).await
    }
    async fn interact_with_policy(&self,owner:&str,session:&str,version:u64,key:&str,request:VoiceWorkRequest,policy:WorkSteeringPolicy)->Result<VoiceWorkReceipt,VoiceError> {
        let id=AgentSessionId::from(session);let scope=self.binding(owner,&id,version).await?;
        if key.is_empty()||key.len()>256{return Err(VoiceError::new(VoiceErrorKind::Configuration,"voice work requires a stable bounded operation key"));}
        match request {
            VoiceWorkRequest::Start {text}=>{
                let journal=self.journal.get_or_open().await?;let (input,duplicate)=journal.record_pending_intent_with_policy(owner.into(),session.into(),version,context_floor(&scope),key.into(),text,policy).await?;
                self.schedule(owner.into(),session.into());self.input_receipt(input,duplicate).await
            }
            VoiceWorkRequest::CancelQueued {pending_input_id}=>self.control_input(owner,session,key,&pending_input_id,None).await,
            VoiceWorkRequest::ReviseQueued {pending_input_id,text,expected_revision}=>self.control_input(owner,session,key,&pending_input_id,Some((expected_revision,text))).await,
            VoiceWorkRequest::Observe {target}=>{
                require_target(&target,&id,version)?;self.observe(owner,&target).await
            }
            VoiceWorkRequest::Cancel {target}=>{
                require_target(&target,&id,version)?;
                self.validate_control_replay(owner,session,key,&target,None,false).await?;
                let result=self.sessions.voice_cancel_exact_native_turn(owner,&id,key,&target.turn_operation_id,&fence(&target)).await;
                match self.lookup_control(owner,session,key).await? {Some(receipt)=>Ok(receipt),None=>{let mutation=result.map_err(work_error)?;let mut receipt=self.observe(owner,&target).await?;receipt.operation_key=key.into();receipt.duplicate=mutation.duplicate;Ok(receipt)}}
            }
            VoiceWorkRequest::Steer {target,text}=>{
                require_target(&target,&id,version)?;
                if text.trim().is_empty()||text.len()>16*1024{return Err(VoiceError::new(VoiceErrorKind::Configuration,"correction requires complete bounded wording"));}
                let supersede=policy==WorkSteeringPolicy::SupersedeModelStep&&scope.events.iter().any(|event|event.kind.0=="turn/started"&&event.correlation_id.as_ref()==target.turn_operation_id.as_ref()
                    &&scope.event_payloads.get(event.event_id.as_ref()).and_then(|value|value.pointer("/admission/voice_input_context/supersede_model_step")).and_then(Value::as_bool)==Some(true));
                let input=voice_message(text.clone());self.validate_control_replay(owner,session,key,&target,Some(&input),supersede).await?;
                let result=self.sessions.voice_steer_exact_native_with_policy(owner,&id,key,&target.turn_operation_id,input,&fence(&target),supersede).await;
                if let Some(receipt)=self.lookup_control(owner,session,key).await? {
                    if result.is_err() {self.retry_control_delivery(owner.into(),session.into(),key.into(),target,text,supersede);}
                    return Ok(receipt);
                }
                result.map_err(work_error)?;Err(work_error("steering receipt was not observed"))
            }
            VoiceWorkRequest::AnswerApproval {answer}=>self.answer_approval(owner,key,answer).await,
        }
    }
    async fn lookup_operation(&self,owner:&str,session:&str,key:&str)->Result<Option<VoiceWorkReceipt>,VoiceError> {
        self.owned_facts(owner,&session.into()).await?;let Some(journal)=self.journal.opened().await else{return Ok(None);};
        if let Some(input)=journal.pending_intent(owner.into(),session.into(),key.into()).await? {return self.input_receipt(input,true).await.map(Some);}
        if let Some(input)=journal.pending_control(owner.into(),session.into(),key.into()).await? {let mut receipt=self.input_receipt(input,true).await?;receipt.operation_key=key.into();return Ok(Some(receipt));}
        self.lookup_control(owner,session,key).await
    }
    async fn observe(&self,owner:&str,target:&WorkTarget)->Result<VoiceWorkReceipt,VoiceError> {
        let facts=self.exact_facts(owner,target).await?;let turn=self.sessions.canonical().turn_receipt(&Self::principal(owner),&target.agent_session_id,&target.turn_operation_id).await.map_err(work_error)?;
        let delivery=nomifun_agent_execution::canonical_turn_delivery(&facts,&turn,true).map_err(work_error)?;
        let unsettled=self.sessions.canonical().store().list_effects(&target.agent_session_id).await.map_err(work_error)?.iter()
            .any(|effect|effect.turn_id==target.turn_operation_id&&matches!(effect.state,nomifun_agent_session::AgentEffectState::Pending|nomifun_agent_session::AgentEffectState::Unknown));
        let state=if turn.status==TurnReceiptStatus::Cancelled&&unsettled {VoiceWorkStatus::PendingBoundary}else if turn.terminal_event.is_some(){VoiceWorkStatus::Terminal}
            else if facts.head.status=="paused" {VoiceWorkStatus::PendingBoundary}else{VoiceWorkStatus::Accepted};
        let public=self.owned_facts(owner,&target.agent_session_id).await?;let root=turn.started_event.as_ref().and_then(|start|facts.event_payloads.get(start.event_id.as_ref())).and_then(|value|value.get("source_message_id")).and_then(Value::as_str);
        let speech=public_messages(&public)?.into_iter().rev().find(|message|root.is_some()&&message.speaker==VoiceSpeaker::Assistant&&message.root.as_deref()==root)
            .map(|message|VoiceSpeechProjection {message_id:message.id,revision:message.last_seq,through_seq:message.last_seq,text:bounded(&message.text,4096)});
        Ok(VoiceWorkReceipt {receipt_id:target.turn_operation_id.as_ref().into(),operation_key:target.turn_operation_id.as_ref().into(),target:Some(target.clone()),pending_input_id:None,status:state,
            summary:if delivery.completed {match delivery.result_ok {Some(true)=>"The exact canonical task completed.",_=>"The exact canonical task closed; no further execution is claimed."}}
                else if target.execution_generation==0 {"The canonical task is accepted and has not claimed a native model generation."}else if facts.head.status=="paused" {"The exact task is paused at its canonical boundary."}else{"The canonical task remains active."}.into(),
            duplicate:true,speech,pending_input_revision:None,first_claim_generation:first_native_generation(&facts,&target.turn_operation_id)?})
    }
    async fn answer_approval(&self,owner:&str,key:&str,answer:VoiceApprovalAnswer)->Result<VoiceWorkReceipt,VoiceError> {self.apply_approval(owner,key,answer).await}
}

impl AppVoiceWorkHost {
    async fn execution_ids(&self,owner:&str,session:&str)->Result<Vec<String>,VoiceError> {
        let Some(engine)=&self.execution else{return Ok(vec![]);};
        if let Some(id)=engine.execution_for_attempt_conversation(owner,session).await.map_err(work_error)? {return Ok(vec![id]);}
        let mut ids=Vec::new();let mut offset=0;
        loop {let page=engine.list(owner,Some(200),Some(offset)).await.map_err(work_error)?;let count=page.len();
            ids.extend(page.into_iter().filter(|execution|execution.lead_conversation_id.as_deref()==Some(session)).map(|execution|execution.execution_id));
            if count<200{break;}offset+=200;
        }Ok(ids)
    }
    async fn approvals(&self,owner:&str,session:&str,version:u64)->Result<Vec<VoiceApprovalPresentation>,VoiceError> {
        let Some(engine)=&self.execution else{return Ok(vec![]);};let mut result=Vec::new();
        for execution in self.execution_ids(owner,session).await? {
            let detail=engine.get(owner,&execution).await.map_err(work_error)?;
            if detail.execution.decision_policy!=nomifun_common::DecisionPolicy::AskUser{continue;}
            let waiting=detail.attempts.iter().filter(|attempt|attempt.status==nomifun_common::ExecutionAttemptStatus::WaitingInput).collect::<Vec<_>>();
            if waiting.is_empty(){continue;}let mut events=Vec::new();let mut after=0;
            loop {let page=engine.events(owner,&execution,Some(after),Some(128)).await.map_err(work_error)?;let next=page.last().map_or(after,|event|event.sequence);
                events.extend(page.into_iter().filter(|event|event.event_type==nomifun_common::AgentExecutionEventKind::DecisionRequested));
                if next<=after||next>=detail.execution.event_sequence{break;}after=next;
            }
            for attempt in waiting {
                let (Some(question),Some(source_session),Some(step))=(attempt.question.as_ref(),attempt.conversation_id.as_ref(),detail.steps.iter().find(|step|step.step_id==attempt.step_id)) else{continue;};
                let Some(request)=events.iter().rev().find(|event|event.attempt_id.as_deref()==Some(attempt.attempt_id.as_str())&&event.step_id.as_deref()==Some(step.step_id.as_str())
                    &&event.payload.get("question").and_then(Value::as_str)==Some(question.as_str())) else{continue;};
                let Some(stop_key)=request.payload.get("stop_turn_operation_id").and_then(Value::as_str) else{continue;};
                let source=self.owned_facts(owner,&source_session.clone().into()).await?;
                let encoded=attempt.runtime_state.as_ref().map(Value::to_string);
                let effects=nomifun_db::AttemptConversationEffects::decode(encoded.as_deref()).map_err(work_error)?;
                let pending=effects.pending_conversation_effects.iter().find_map(|effect|match effect {
                    nomifun_db::PendingConversationEffect::StopTurn {operation_id,target_operation_id} if operation_id==stop_key=>Some(target_operation_id.clone()),_=>None,
                });
                let cancelled=source.events.iter().find(|event|event.kind.0=="turn/cancelled"&&event.producer_id.as_ref()=="session_api"
                    &&event.idempotency_key.as_ref()==format!("user:{owner}:{source_session}:{stop_key}:cancel"))
                    .and_then(|event|source.event_payloads.get(event.event_id.as_ref())).and_then(|value|value.get("target_operation_id")).and_then(Value::as_str).map(str::to_owned);
                let Some(operation)=pending.or(cancelled) else{continue;};
                let Some(start)=source.events.iter().find(|event|event.kind.0=="turn/started"&&event.correlation_id.as_ref()==operation) else{continue;};
                if context_floor(&source)>start.seq {continue;}
                let Some(admission)=source.event_payloads.get(start.event_id.as_ref()).and_then(|value|value.get("admission")) else{continue;};
                if admission.get("resolved_snapshot_ref")!=Some(&serde_json::to_value(&source.session.agent_binding.resolved_snapshot_ref).map_err(work_error)?) {continue;}
                let target=self.target(owner,&source_session.clone().into(),&operation.into()).await?;
                let mut approval=ApprovalTarget {work_target:target,interaction_agent_session_id:session.into(),interaction_binding_version:version,
                    execution_id:execution.clone(),step_id:step.step_id.clone(),attempt_id:attempt.attempt_id.clone(),expected_execution_version:detail.execution.version,
                    expected_step_version:step.version,expected_attempt_version:attempt.version,request_event_sequence:request.sequence,action_id:"agent/request_user_decision".into(),
                    question_digest:digest_bytes(question.as_bytes()),presentation_id:format!("execution:{execution}:decision:{}",request.sequence),presentation_digest:digest_bytes(b""),
                    interaction_mode:if request.payload.get("force_click").and_then(Value::as_bool)==Some(true)||request.payload.get("requires_explicit_click").and_then(Value::as_bool)==Some(true)
                        {ApprovalInteractionMode::ExplicitClick}else{ApprovalInteractionMode::VoiceAllowed}};
                approval.presentation_digest=approval_digest(&approval)?;result.push(VoiceApprovalPresentation {target:approval,question:bounded(question,4096)});
            }
        }Ok(result)
    }
    async fn apply_approval(&self,owner:&str,key:&str,answer:VoiceApprovalAnswer)->Result<VoiceWorkReceipt,VoiceError> {
        let target=&answer.target;self.binding(owner,&target.interaction_agent_session_id,target.interaction_binding_version).await?;
        if target.interaction_mode!=ApprovalInteractionMode::VoiceAllowed||target.action_id.as_ref()!="agent/request_user_decision"||target.presentation_id!=answer.presented_context_id
            ||target.presentation_digest!=approval_digest(target)?||answer.answer.trim().is_empty()||answer.answer.len()>16*1024 {
            return Err(VoiceError::new(VoiceErrorKind::Unsupported,"The answer requires an exact eligible presentation. Explicit-click decisions stay with the original UI."));
        }
        let engine=self.execution.as_ref().ok_or_else(||VoiceError::new(VoiceErrorKind::Unsupported,"decision owner is unavailable"))?;
        let linked=self.execution_ids(owner,target.interaction_agent_session_id.as_ref()).await?;if !linked.contains(&target.execution_id){return Err(work_error("unlinked voice decision"));}
        let duplicate=engine.lookup_voice_decision_answer(owner,&target.execution_id,key).await.map_err(work_error)?.is_some();
        if !duplicate&&!self.approvals(owner,target.interaction_agent_session_id.as_ref(),target.interaction_binding_version).await?.iter().any(|presentation|presentation.target==*target) {
            return Err(VoiceError::new(VoiceErrorKind::StaleBinding,"The pending question, source, action, version, or presentation changed."));
        }
        engine.answer_voice_decision(owner,&nomifun_common::AgentExecutionActor::user(owner),&target.execution_id,&target.step_id,&target.attempt_id,
            nomifun_api_types::AnswerExecutionDecisionRequest {answer:answer.answer,expected_execution_version:target.expected_execution_version,expected_step_version:target.expected_step_version,expected_attempt_version:target.expected_attempt_version},
            key,target.request_event_sequence,target.question_digest.as_ref(),target.presentation_digest.as_ref(),&target.work_target.agent_session_id,&target.work_target.turn_operation_id,&fence(&target.work_target)).await.map_err(work_error)?;
        let event=engine.lookup_voice_decision_answer(owner,&target.execution_id,key).await.map_err(work_error)?.ok_or_else(||VoiceError::new(VoiceErrorKind::JournalUnavailable,"Answer outcome is unconfirmed; look up the original key before retrying."))?;
        Ok(VoiceWorkReceipt {receipt_id:format!("execution:{}:decision:{}",target.execution_id,target.request_event_sequence),operation_key:key.into(),target:Some(target.work_target.clone()),pending_input_id:None,
            status:VoiceWorkStatus::Applied,summary:format!("The exact pending decision has a durable answer event {}. No new tool permission was granted.",event.sequence),duplicate,speech:None,pending_input_revision:None,first_claim_generation:None})
    }
    async fn lookup_approval(&self,owner:&str,session:&str,key:&str)->Result<Option<VoiceWorkReceipt>,VoiceError> {
        let Some(engine)=&self.execution else{return Ok(None);};
        for execution in self.execution_ids(owner,session).await? {
            if let Some(event)=engine.lookup_voice_decision_answer(owner,&execution,key).await.map_err(work_error)? {
                let Some(request_sequence)=event.payload.get("request_event_sequence").and_then(Value::as_i64) else{return Err(work_error("answer lost its request proof"));};
                // Lookup never answers again. The stable exact request receipt
                // can repair a journal link independently of later versions.
                let target:WorkTarget=serde_json::from_value(event.payload.get("voice_source_target").cloned().ok_or_else(||work_error("voice answer lost its original source"))?).map_err(work_error)?;
                return Ok(Some(VoiceWorkReceipt {receipt_id:format!("execution:{execution}:decision:{request_sequence}"),operation_key:key.into(),target:Some(target),pending_input_id:None,
                    status:VoiceWorkStatus::Applied,summary:format!("The original decision answer has canonical event {}.",event.sequence),duplicate:true,speech:None,pending_input_revision:None,first_claim_generation:None}));
            }
        }Ok(None)
    }
}
fn approval_digest(target:&ApprovalTarget)->Result<DigestHex,VoiceError> {
    let mut value=serde_json::to_value(target).map_err(work_error)?;value.as_object_mut().ok_or_else(||work_error("invalid approval object"))?.remove("presentation_digest");
    digest_payload(&value).map_err(work_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{AgentSessionLiveRecord,SessionEventKind,SessionEventPayloadRef,StrictJsonValue};
    use nomifun_agent_session::SessionHeadProjection;

    fn facts()->ChatCausalityFacts {
        let session:AgentSessionLiveRecord=serde_json::from_value(json!({"agent_session_id":"019b0000-0000-7000-8000-000000000001","owner_ref":{"principal_kind":"user","principal_id":"owner"},
            "metadata":{"archived":false,"pinned":false},"agent_binding":{"preset_revision_ref":{"preset_id":"preset","revision":1,"revision_digest":"a".repeat(64)},
                "resolved_snapshot_ref":{"snapshot_id":"snapshot","snapshot_digest":"b".repeat(64)},"typed_resource_bindings":[],"binding_version":1},"next_seq":1})).unwrap();
        ChatCausalityFacts {head:SessionHeadProjection {session_id:session.agent_session_id.clone(),status:"ready".into(),active_turn_id:None,active_set_generation:0,last_seq:0,unread_count:0},session,
            events:vec![],event_payloads:Default::default(),operation_ids:Default::default(),turn_route_identities:Default::default(),execution_generation:0,execution_fence:0,fork_context:None}
    }

    #[test]
    fn voice_first_native_claim_proof_comes_from_the_same_turn_zero_fence_and_never_a_recovery_generation() {
        let mut data=facts();event(&mut data,"turn/started","started","task",json!({"operation_id":"task","source_message_id":"root"}));
        assert_eq!(first_native_generation(&data,&"task".into()).unwrap(),None);
        event(&mut data,"runtime/execution-claimed","first","task",json!({"operation_id":"task","execution_fence":0,"holder_digest":"a".repeat(64)}));
        data.events.last_mut().unwrap().causation_event_id=Some("started".into());assert_eq!(first_native_generation(&data,&"task".into()).unwrap(),Some(1));
        event(&mut data,"runtime/execution-claimed","recovery","task",json!({"operation_id":"task","execution_fence":1,"holder_digest":"b".repeat(64)}));data.events.last_mut().unwrap().causation_event_id=Some("started".into());data.execution_generation=3;
        assert_eq!(first_native_generation(&data,&"task".into()).unwrap(),Some(1));assert_ne!(first_native_generation(&data,&"task".into()).unwrap(),Some(data.execution_generation));
        data.events[1].causation_event_id=Some("different-turn-start".into());assert_eq!(first_native_generation(&data,&"task".into()).unwrap(),None);
    }
    fn event(facts:&mut ChatCausalityFacts,kind:&str,id:&str,correlation:&str,payload:Value) {
        let seq=facts.events.len() as u64+1;facts.head.last_seq=seq;
        facts.events.push(SessionEventRecord {agent_session_id:facts.session.agent_session_id.clone(),seq,event_id:id.into(),producer_id:"runtime_supervisor".into(),idempotency_key:id.into(),
            kind:SessionEventKind(kind.into()),kind_version:1,correlation_id:correlation.into(),causation_event_id:None,payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(payload.clone()))});
        facts.event_payloads.insert(id.into(),payload);
    }
    #[test]
    fn voice_typed_initial_facts_preserve_sources_hide_private_data_and_mark_excerpts() {
        let mut data=facts();event(&mut data,"message/user-accepted","public-user","public-user",json!({"content":"explicit user task"}));
        event(&mut data,"message/user-accepted","hidden-user","hidden-user",json!({"content":"PRIVATE_USER","hidden":true}));
        event(&mut data,"message/content-part","hidden-part","hidden-assistant",json!({"content":"PRIVATE_ASSISTANT","turn_id":"hidden-user"}));
        event(&mut data,"message/completed","hidden-completed","hidden-assistant",json!({"part_count":1,"content_digest":digest_bytes(b"PRIVATE_ASSISTANT")}));
        event(&mut data,"thinking/content-part","reasoning","reasoning",json!({"content":"PRIVATE_REASONING"}));event(&mut data,"tool/result-recorded","tool","tool",json!({"output":"RAW_TOOL"}));
        let long="公开正文".repeat(700);event(&mut data,"message/content-part","public-part","public-assistant",json!({"content":long,"turn_id":"public-user"}));
        event(&mut data,"message/completed","public-completed","public-assistant",json!({"part_count":1,"content_digest":digest_bytes(long.as_bytes())}));
        event(&mut data,"message/content-part","unfinished-part","unfinished-assistant",json!({"content":"UNFINISHED_TEXT","turn_id":"public-user"}));
        let before=serde_json::to_value(&data).unwrap();let visible=initial_facts(&data).unwrap();assert_eq!(visible.len(),2);
        let assistant=&visible[1];let source=assistant.speech_source.as_ref().unwrap();assert_eq!(source.message_id,"public-assistant");assert_eq!(source.through_seq,8);
        assert_eq!(assistant.canonical_receipt_id,"public-completed");let content:Value=serde_json::from_str(&assistant.content).unwrap();assert_eq!(content["truncated"],true);assert_eq!(content["original_bytes"],long.len());
        let encoded=serde_json::to_string(&visible).unwrap();for hidden in ["PRIVATE_USER","PRIVATE_ASSISTANT","PRIVATE_REASONING","RAW_TOOL","UNFINISHED_TEXT"] {assert!(!encoded.contains(hidden));}
        assert_eq!(serde_json::to_value(&data).unwrap(),before,"typed readers must not edit source facts");
        event(&mut data,"context/cleared","clear","session",json!({}));let cleared=initial_facts(&data).unwrap();assert!(cleared.is_empty());assert!(!visible.iter().all(|fact|cleared.contains(fact)));
    }
    #[test]
    fn voice_typed_context_fails_closed_for_missing_parts_and_bad_digest() {
        let mut data=facts();event(&mut data,"message/completed","orphan","assistant",json!({"part_count":1,"content_digest":digest_bytes(b"missing")}));assert!(public_messages(&data).is_err());
        let mut data=facts();event(&mut data,"message/content-part","part","assistant",json!({"content":"actual bytes","turn_id":"root"}));
        event(&mut data,"message/completed","completed","assistant",json!({"part_count":1,"content_digest":"f".repeat(64)}));assert!(public_messages(&data).is_err());
    }
    #[test]
    fn voice_source_validation_uses_immutable_canonical_history_instead_of_the_latest_display_window(){
        let mut data=facts();
        event(&mut data,"message/assistant-projected","original-assistant","assistant",json!({"content":"Original canonical result"}));
        let original=initial_facts(&data).unwrap().remove(0);let source=original.speech_source.as_ref().unwrap().clone();
        for index in 0..14 {let id=format!("later-user-{index}");event(&mut data,"message/user-accepted",&id,&id,json!({"content":format!("Later ordinary user message {index}")}));}
        assert!(!initial_facts(&data).unwrap().contains(&original));
        let messages=public_messages(&data).unwrap();
        assert!(source_message(&messages,&source).is_some());
        assert!(messages.into_iter().map(|message|public_message_fact(&data,message)).any(|fact|fact==original),"normal later messages must not revoke a valid initial source");
        event(&mut data,"message/assistant-projected","replacement-assistant","assistant",json!({"content":"Corrected canonical result"}));
        assert!(source_message(&public_messages(&data).unwrap(),&source).is_none(),"an older same-ID source cannot certify newer wording");
        let replacement=public_messages(&data).unwrap().into_iter().find(|message|message.id=="assistant"&&message.text=="Corrected canonical result").unwrap();
        let replacement_source=VoiceSpeechSourceRef{message_id:replacement.id,revision:replacement.last_seq,through_seq:replacement.last_seq};
        event(&mut data,"message/assistant-projected","hidden-assistant","assistant",json!({"content":"Corrected canonical result","hidden":true}));
        assert!(source_message(&public_messages(&data).unwrap(),&replacement_source).is_none(),"an explicit hidden same-ID source cannot fall back to an earlier public body");
        event(&mut data,"context/cleared","floor","session",json!({}));
        assert!(source_message(&public_messages(&data).unwrap(),&source).is_none());
    }
    #[test]
    fn voice_historical_acceptance_survives_model_claim_without_renewing_an_exact_mutation_fence(){
        let mut data=facts();data.execution_generation=10;
        let mut target=WorkTarget{agent_session_id:data.session.agent_session_id.clone(),binding_version:1,turn_operation_id:"original-turn".into(),execution_generation:0};
        assert!(historical_execution_generation(&data,&target,Some(3)),"acceptance before a model claim remains history");
        target.execution_generation=3;assert!(historical_execution_generation(&data,&target,Some(3)));
        target.execution_generation=999;assert!(!historical_execution_generation(&data,&target,Some(3)));
        event(&mut data,"runtime/execution-claimed","new-claim","original-turn",json!({"operation_id":"original-turn","execution_fence":1}));
        target.execution_generation=data.events[0].seq;assert!(historical_execution_generation(&data,&target,Some(3)));
        target.turn_operation_id="other-turn".into();assert!(!historical_execution_generation(&data,&target,Some(3)),"a different task claim is not source evidence");
        assert_ne!(target.execution_generation,data.execution_generation,"historical source validation must not certify the current work generation");
    }
    #[test]
    fn voice_owned_supersede_withdraws_only_its_exact_typed_model_step_source(){
        use nomifun_agent_runtime::AgentEngineEvent;
        use nomifun_chat_model_broker::{OwnedModelCleanupReceipt,OwnedModelCleanupStage,OwnedModelCleanupOutcome};
        let mut data=facts();let root="019b0000-0000-7000-8000-000000000020";
        event(&mut data,"message/user-accepted",root,root,json!({"content":"Explicit voice work"}));
        event(&mut data,"turn/started","turn-start","voice-turn",json!({"source_message_id":root}));
        event(&mut data,"runtime/progress-recorded","step-start","voice-turn",json!({"event":AgentEngineEvent::ModelStepStarted{step:1,operation_id:"voice-turn:model:1".into()}}));
        let withdrawn=super::super::engine_journal::canonical_assistant_step_message_id(root,1).unwrap();
        let kept=super::super::engine_journal::canonical_assistant_step_message_id(root,2).unwrap();
        event(&mut data,"message/assistant-projected","first-output",&withdrawn,json!({"content":"SUPERSEDED_WORDING"}));
        event(&mut data,"message/assistant-projected","other-output",&kept,json!({"content":"UNRELATED_VALID_WORDING"}));
        let source=initial_facts(&data).unwrap().into_iter().find_map(|fact|fact.speech_source.filter(|source|source.message_id==withdrawn)).unwrap();
        let superseded=AgentEngineEvent::VoiceModelStepSuperseded{step:1,model_operation_id:"voice-turn:model:1".into(),steering_receipt_ids:vec!["real-steering-receipt".into()],discarded_tool_call_ids:vec![],
            cleanup:OwnedModelCleanupReceipt{operation_id:"voice-turn:model:1".into(),task_id:"owned-task-7".into(),stage:OwnedModelCleanupStage::Producer,outcome:OwnedModelCleanupOutcome::AbortedJoined}};
        event(&mut data,"runtime/progress-recorded","owned-superseded","voice-turn",json!({"event":superseded}));
        let messages=public_messages(&data).unwrap();assert!(source_message(&messages,&source).is_none());
        assert!(messages.iter().any(|message|message.id==kept&&message.text=="UNRELATED_VALID_WORDING"));
        assert!(data.event_payloads.values().any(|payload|payload.get("content")==Some(&json!("SUPERSEDED_WORDING"))),"withdrawal is a view over immutable canonical facts, not history deletion");
        let record=data.event_payloads.get_mut("owned-superseded").unwrap();record["event"]["model_operation_id"]=json!("other:model:1");
        assert!(public_messages(&data).is_err(),"an unrelated owned closure cannot withdraw this step by guessed identity");
    }
    #[test]
    fn voice_steer_is_not_applied_until_an_actual_later_model_boundary() {
        use nomifun_agent_runtime::{AgentEngineEvent,AgentSteeringInput};
        let mut data=facts();let input=AgentSteeringInput {receipt_operation_id:"receipt".into(),message_id:"message".into(),text:"complete correction".into(),files:vec![],inject_skills:vec![],image_count:0,prepared_images:vec![],prepared_skill_instructions:vec![]};
        event(&mut data,"runtime/progress-recorded","taken","turn",json!({"event":AgentEngineEvent::SteeringInputs {inputs:vec![input]}}));
        assert_eq!(steering_status(&data,&"turn".into(),"receipt",false).unwrap(),VoiceWorkStatus::PendingBoundary);
        assert_eq!(steering_status(&data,&"turn".into(),"receipt",true).unwrap(),VoiceWorkStatus::Deferred);
        event(&mut data,"runtime/progress-recorded","next-model","turn",json!({"event":AgentEngineEvent::ModelStepStarted {step:1,operation_id:"next".into()}}));
        assert_eq!(steering_status(&data,&"turn".into(),"receipt",false).unwrap(),VoiceWorkStatus::Applied);
    }
}
