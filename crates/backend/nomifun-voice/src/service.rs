use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex, atomic::{AtomicBool,AtomicU64, Ordering}};
use std::time::Duration;
use async_trait::async_trait;
use nomifun_voice_contracts::voice::*;
use nomifun_voice_core::{VoiceSessionCore, VoiceOpenRequest, VoiceInputHandle,VoiceEndpointLease,VoiceEndpointPort};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, broadcast, mpsc,watch};
use tokio::task::JoinHandle;
use tokio_util::task::AbortOnDropHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use crate::{VoiceAdapterRegistry, VoiceJournal, VoiceActivationFact, VoiceWorkBridge, VoiceContextAssembler, VoiceWorkPort};
use crate::endpoint::{ProductVoiceEndpoint,VoiceOutputFrame};
mod speech_sources;
use speech_sources::SpeechSourceLedger;

#[derive(Clone)]
pub struct VoiceBindingPlan {
    pub record: VoiceRouteRecord,pub plan: ResolvedVoicePlan,
    pub profile_id:String,pub profile_revision:u64,
    pub work_steering_policy:nomifun_voice_contracts::WorkSteeringPolicy,
    pub lease_revision:String,
}
#[async_trait]
pub trait VoiceAuthorityPort: Send + Sync {
    async fn resolve_plan(&self, owner: &str, session: &str, binding_version: u64,profile_id:&str,profile_revision:u64) -> Result<VoiceBindingPlan, VoiceError>;
    async fn validate_binding(&self, owner: &str, session: &str, binding_version: u64) -> Result<(), VoiceError>;
    async fn validate_lease(&self,owner:&str,session:&str,binding_version:u64,_record:&VoiceRouteRecord,_lease_revision:&str)->Result<(),VoiceError>{self.validate_binding(owner,session,binding_version).await}
}
/// A live authorization witness supplied by the authenticated product API.
/// It contains no model credentials and cannot grant canonical work actions.
pub trait VoiceAccessLease:Send+Sync {fn is_valid(&self)->bool;}
pub use nomifun_voice_contracts::{VoiceActivationRequest,VoiceActivationResponse};
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct VoiceSessionProjection{pub state:VoiceState,pub transcripts:Vec<TranscriptFragment>,pub playback_receipts:Vec<PlaybackReceipt>,pub last_work_receipt:Option<VoiceWorkReceipt>,pub approvals:Vec<nomifun_voice_contracts::VoiceApprovalPresentation>,pub source_context_requirement:Option<nomifun_voice_contracts::VoiceSourceContextRequirement>}
pub struct VoiceMediaAttachment {
    pub frames: mpsc::Receiver<VoiceOutputFrame>, pub events: broadcast::Receiver<VoiceProductEvent>,
    pub state: VoiceState, pub max_age: Duration,
    pub controls:watch::Receiver<Option<VoiceProductEvent>>,
    pub endpoint:Arc<ProductVoiceEndpoint>,
}
pub struct ActiveVoice {
    owner: String, endpoint_id: String, attachment_token: String, attachment_expires: Instant,
    core: StdMutex<VoiceSessionCore>, pub negotiation: VoiceNegotiation,
    input: VoiceInputHandle, cancel: CancellationToken,
    events: broadcast::Sender<VoiceProductEvent>, media_rx: Mutex<Option<mpsc::Receiver<VoiceOutputFrame>>>,
    supervisor: Mutex<Option<JoinHandle<VoiceTermination>>>,
    media_attached:AtomicBool,
    foreground_lease:StdMutex<(Instant,Option<u64>)>,
    speech_sources:StdMutex<SpeechSourceLedger>,
    last_work_receipt:Arc<Mutex<Option<VoiceWorkReceipt>>>,approvals:StdMutex<Vec<nomifun_voice_contracts::VoiceApprovalPresentation>>,
    profile_id:String,profile_revision:u64,
    work_steering_policy:nomifun_voice_contracts::WorkSteeringPolicy,
    lease_record:VoiceRouteRecord,lease_revision:String,
    endpoint:Arc<ProductVoiceEndpoint>,
    access:Option<Arc<dyn VoiceAccessLease>>,
    input_control_gate:Mutex<()>,
    bridge:Arc<Mutex<VoiceWorkBridge>>,
    /// Bounded telemetry annotation, never task state or execution authority.
    work_kinds:StdMutex<BTreeMap<String,VoiceWorkRequestKind>>,
    trigger_tx:mpsc::Sender<WorkTrigger>,
}
impl ActiveVoice {
    fn state(&self) -> VoiceState { self.core.lock().unwrap_or_else(|p|p.into_inner()).state().clone() }
    fn request_kind(&self,receipt:&VoiceWorkReceipt,executed:Option<VoiceWorkRequestKind>)->Option<VoiceWorkRequestKind>{
        let mut kinds=self.work_kinds.lock().unwrap_or_else(|p|p.into_inner());
        if let Some(kind)=executed{
            if !kinds.contains_key(&receipt.operation_key)&&kinds.len()>=4096{if let Some(oldest)=kinds.keys().next().cloned(){kinds.remove(&oldest);}}
            kinds.entry(receipt.operation_key.clone()).or_insert(kind);
        }
        kinds.get(&receipt.operation_key).copied()
    }
    fn check_access(&self)->Result<(),VoiceError>{
        if self.access.as_ref().is_some_and(|lease|!lease.is_valid()){
            return Err(VoiceError::new(VoiceErrorKind::Authentication,"voice authorization lease was revoked"));
        }
        if self.media_attached.load(Ordering::Acquire)&&Instant::now()>self.foreground_lease.lock().unwrap_or_else(|p|p.into_inner()).0{
            return Err(VoiceError::new(VoiceErrorKind::Closed,"foreground voice lease expired"));
        }
        Ok(())
    }
}

/// The only live voice-session owner. Registry, canonical work and binding authority are injected.
pub struct VoiceSessionService {
    pub registry: Arc<VoiceAdapterRegistry>, work: Arc<dyn VoiceWorkPort>, authority: Arc<dyn VoiceAuthorityPort>,
    data_dir: PathBuf, journal_store:Arc<crate::SharedVoiceJournal>,journal_ready:AtomicBool,journal_reconcile:Mutex<()>,
    sessions: Mutex<BTreeMap<String,Arc<ActiveVoice>>>, lease_gate: Mutex<()>, epoch: AtomicU64,
    shutdown: CancellationToken,
}
impl VoiceSessionService {
    pub async fn probe_profile(&self,owner:&str,profile_id:&str,request:nomifun_voice_contracts::VoiceProfileProbeRequest,access:Arc<dyn VoiceAccessLease>)->Result<nomifun_voice_contracts::VoiceProfileProbeResult,VoiceError>{
        if !access.is_valid(){return Err(VoiceError::new(VoiceErrorKind::Authentication,"voice diagnostic authorization expired"));}
        let plan=self.authority.resolve_plan(owner,&request.agent_session_id,request.binding_version,profile_id,request.profile_revision).await?;
        let context=self.work.context(owner,&request.agent_session_id,request.binding_version).await?;
        let id=uuid::Uuid::now_v7().to_string();let cancel=self.shutdown.child_token();
        let open=VoiceOpenRequest{voice_session_id:id,activation_epoch:1,output_generation:1,route_identity:plan.plan.identity.clone(),model:plan.record.model.clone(),
            instructions:"Application connection diagnostic. Remain silent; no user audio, work or tool authority is provided.".into(),initial_facts:vec![],tools:vec![],transport:plan.record.transport,native_offer:request.native_offer,work_context:None,
            replay_scope:VoiceReplayScope{agent_session_id:request.agent_session_id.clone().into(),binding_version:request.binding_version,context_floor:context.context_floor},initial_local_replay:None};
        let(negotiation,termination)=self.registry.probe(&plan.record,open,cancel,Instant::now()+Duration::from_secs(20)).await?;
        self.authority.validate_lease(owner,&request.agent_session_id,request.binding_version,&plan.record,&plan.lease_revision).await?;
        if !access.is_valid(){return Err(VoiceError::new(VoiceErrorKind::Authentication,"voice diagnostic authorization changed"));}
        Ok(nomifun_voice_contracts::VoiceProfileProbeResult{profile_id:profile_id.into(),profile_revision:request.profile_revision,transport:plan.record.transport,connection_verified:termination.finalization_confirmed&&termination.reason==VoiceCloseReason::UserEnded,negotiation,termination})
    }
    pub fn new(registry:Arc<VoiceAdapterRegistry>,work:Arc<dyn VoiceWorkPort>,authority:Arc<dyn VoiceAuthorityPort>,data_dir:PathBuf,shutdown:CancellationToken)->Self {
        let journal=Arc::new(crate::SharedVoiceJournal::new(data_dir));Self::with_journal(registry,work,authority,journal,shutdown)
    }
    pub fn with_journal(registry:Arc<VoiceAdapterRegistry>,work:Arc<dyn VoiceWorkPort>,authority:Arc<dyn VoiceAuthorityPort>,journal_store:Arc<crate::SharedVoiceJournal>,shutdown:CancellationToken)->Self{
        Self{registry,work,authority,data_dir:journal_store.data_dir().into(),journal_store,journal_ready:AtomicBool::new(false),journal_reconcile:Mutex::new(()),sessions:Mutex::new(BTreeMap::new()),lease_gate:Mutex::new(()),epoch:AtomicU64::new(1),shutdown}
    }
    async fn journal(&self)->Result<VoiceJournal,VoiceError> {
        let opened=self.journal_store.get_or_open().await?;
        let _reconcile=self.journal_reconcile.lock().await;
        if self.journal_ready.load(Ordering::Acquire){return Ok((*opened).clone());}
        for session in opened.pending_deletes().await? {opened.finish_delete(session).await?;}
        for link in opened.unresolved_links().await?{
            match self.work.lookup_operation(&link.owner_id,&link.agent_session_id,&link.operation_key).await{
                Ok(Some(receipt)) if link.canonical_receipt_id.as_ref().is_none_or(|id|id==&receipt.receipt_id)=>{if let Some(reference)=receipt.canonical_reference(){opened.associate_receipt(link.operation_key,reference.into()).await?;}},
                _=>opened.quarantine(link.operation_key).await?,
            }
        }
        let cutoff=system_now_ms()-90*24*60*60*1000;opened.retain_since(cutoff).await?;
        self.journal_ready.store(true,Ordering::Release);Ok((*opened).clone())
    }
    pub async fn activate(self:&Arc<Self>,owner:&str,request:VoiceActivationRequest)->Result<VoiceActivationResponse,VoiceError> {
        self.activate_with_id(owner,request,None,None).await
    }
    pub async fn activate_with_access(self:&Arc<Self>,owner:&str,request:VoiceActivationRequest,access:Arc<dyn VoiceAccessLease>)->Result<VoiceActivationResponse,VoiceError>{self.activate_with_id(owner,request,None,Some(access)).await}
    async fn activate_with_id(self:&Arc<Self>,owner:&str,request:VoiceActivationRequest,voice_id:Option<String>,access:Option<Arc<dyn VoiceAccessLease>>)->Result<VoiceActivationResponse,VoiceError> {
        if self.shutdown.is_cancelled(){return Err(VoiceError::new(VoiceErrorKind::Closed,"application shutting down"));}
        if access.as_ref().is_some_and(|lease|!lease.is_valid()){return Err(VoiceError::new(VoiceErrorKind::Authentication,"voice authentication lease is no longer valid"));}
        if request.endpoint_id.trim().is_empty()||request.endpoint_id.len()>512{return Err(VoiceError::new(VoiceErrorKind::Configuration,"invalid endpoint identity"));}
        let _lease=self.lease_gate.lock().await;
        let plan=self.authority.resolve_plan(owner,&request.agent_session_id,request.binding_version,&request.profile_id,request.profile_revision).await?;
        plan.plan.validate().map_err(|e|VoiceError::new(VoiceErrorKind::Configuration,e))?;
        if plan.profile_id!=request.profile_id||plan.profile_revision!=request.profile_revision{return Err(VoiceError::new(VoiceErrorKind::StaleBinding,"voice profile revision differs from activation"));}
        if plan.record.identity().map_err(|e|VoiceError::new(VoiceErrorKind::Configuration,e))?!=plan.plan.identity||plan.record.transport!=request.transport {
            return Err(VoiceError::new(VoiceErrorKind::StaleBinding,"voice plan or selected transport differs from the frozen Agent binding"));
        }
        let conflicting=self.sessions.lock().await.values().filter(|s| {
            let state=s.state(); !matches!(state.connection,VoiceConnectionState::Closed|VoiceConnectionState::Failed)
                && (s.endpoint_id==request.endpoint_id||state.agent_session_id==request.agent_session_id)
        }).cloned().collect::<Vec<_>>();
        if !conflicting.is_empty()&&!request.takeover{return Err(VoiceError::new(VoiceErrorKind::StaleBinding,"voice input lease already active; explicit takeover is required"));}
        for session in conflicting {self.close_active(session,VoiceCloseReason::LeaseRevoked).await;}
        let journal=self.journal().await?;
        self.work.recover_pending_inputs(owner,&request.agent_session_id).await?;
        let context=self.work.context(owner,&request.agent_session_id,request.binding_version).await?;
        let initial_presentations=context.approvals.clone();
        let mut attachment_bytes=[0u8;32];
        getrandom::getrandom(&mut attachment_bytes).map_err(|_|VoiceError::new(VoiceErrorKind::Configuration,"attachment capability entropy unavailable"))?;
        let attachment_token=hex::encode(attachment_bytes);
        let model=self.registry.create(&plan.record).await?;
        let cancel=self.shutdown.child_token();
        let id=voice_id.unwrap_or_else(||uuid::Uuid::now_v7().to_string());
        let epoch=self.epoch.fetch_add(1,Ordering::SeqCst);
        let initial_facts=VoiceContextAssembler::initial_facts(&context);
        let replay_scope=VoiceReplayScope{agent_session_id:request.agent_session_id.clone().into(),binding_version:request.binding_version,context_floor:context.context_floor};
        let local_replay=journal.local_replay(owner.into(),replay_scope.clone(),plan.lease_revision.clone()).await?;
        let mut initial_sources=SpeechSourceLedger::default();
        for fact in &initial_facts{if let Some(source)=&fact.speech_source{initial_sources.register(source.clone(),1)?;}}
        if !self.work.validate_initial_facts(owner,&request.agent_session_id,&initial_facts).await?{return Err(VoiceError::new(VoiceErrorKind::StaleBinding,"voice context source is no longer current"));}
        let open=VoiceOpenRequest{voice_session_id:id.clone(),activation_epoch:epoch,output_generation:1,route_identity:plan.plan.identity.clone(),model:plan.record.model.clone(),
            instructions:VoiceContextAssembler::instructions(&context),initial_facts:initial_facts.clone(),replay_scope,initial_local_replay:(!local_replay.entries.is_empty()).then_some(local_replay),tools:VoiceContextAssembler::tools(),transport:request.transport,native_offer:request.native_offer,work_context:Some(VoiceWorkContextFact{target:context.observed_target.clone()})};
        let mut model_session=match tokio::time::timeout(Duration::from_secs(20),model.open(open,cancel.clone(),Instant::now()+Duration::from_secs(20))).await {
            Ok(result)=>result?,Err(_)=>{cancel.cancel();return Err(VoiceError::new(VoiceErrorKind::Deadline,"voice model activation deadline"));}
        };
        if let Err(error)=self.authority.validate_lease(owner,&request.agent_session_id,request.binding_version,&plan.record,&plan.lease_revision).await {
            model_session.shutdown(VoiceCloseReason::BindingChanged).await;return Err(error);
        }
        let current_context=self.work.context(owner,&request.agent_session_id,request.binding_version).await;
        let valid_sources=self.work.validate_initial_facts(owner,&request.agent_session_id,&initial_facts).await;
        if access.as_ref().is_some_and(|lease|!lease.is_valid())||!current_context.as_ref().is_ok_and(|current|current.observed_target==context.observed_target&&current.context_floor==context.context_floor)||!valid_sources.is_ok_and(|valid|valid){
            model_session.shutdown(VoiceCloseReason::BindingChanged).await;return Err(VoiceError::new(VoiceErrorKind::StaleBinding,"voice context changed while opening; activate again with current facts"));
        }
        let mut core=VoiceSessionCore::new(id.clone(),request.agent_session_id.clone(),request.binding_version,epoch);
        if let Err(error)=core.negotiate(&model_session.negotiation){model_session.shutdown(VoiceCloseReason::ProviderFailed).await;return Err(error);}
        core.work_running(context.observed_target.is_some()&&context.facts.iter().any(|r|!matches!(r.status,VoiceWorkStatus::Terminal|VoiceWorkStatus::Deferred|VoiceWorkStatus::Rejected)));
        if let Err(error)=journal.activate(VoiceActivationFact{voice_session_id:id.clone(),epoch,owner_id:owner.to_owned(),agent_session_id:request.agent_session_id.clone(),binding_version:request.binding_version,
            route_digest:plan.plan.identity.record_digest.0,started_ms:system_now_ms(),context_floor:Some(context.context_floor),lease_revision:Some(plan.lease_revision.clone())}).await {
            model_session.shutdown(VoiceCloseReason::JournalUnavailable).await;return Err(error);
        }
        for presentation in &initial_presentations{
            if let Err(error)=journal.append(id.clone(),epoch,format!("approval:{}",presentation.target.presentation_id),"approval_presented".into(),Some(presentation.target.presentation_id.clone()),serde_json::to_value(presentation).unwrap_or_default()).await{
                model_session.shutdown(VoiceCloseReason::JournalUnavailable).await;return Err(error);
            }
        }
        let negotiation=model_session.negotiation.clone();
        if (negotiation.native_attachment.is_some())!=(request.transport==VoiceTransportPreference::NativeWebrtc){model_session.shutdown(VoiceCloseReason::ProviderFailed).await;return Err(VoiceError::new(VoiceErrorKind::Unsupported,"negotiated transport differs from the frozen voice route"));}
        let last_receipt=Arc::new(Mutex::new(None::<VoiceWorkReceipt>));
        let work_projection=Arc::new(Mutex::new(context.facts.last().cloned()));
        let (events,_)=broadcast::channel(128);
        let lease=VoiceEndpointLease{endpoint_id:request.endpoint_id.clone(),voice_session_id:id.clone(),activation_epoch:epoch,input_spec:negotiation.input_spec.clone(),output_spec:negotiation.output_spec.clone(),native_attachment:negotiation.native_attachment.clone()};
        let(endpoint,media_rx)=ProductVoiceEndpoint::new(lease.clone(),events.clone(),cancel.clone())?;
        endpoint.attach(lease,cancel.clone(),Instant::now()+Duration::from_secs(3)).await?;
        let bridge=Arc::new(Mutex::new(VoiceWorkBridge::new(self.work.clone(),journal.clone(),owner.to_owned(),request.agent_session_id.clone(),request.binding_version,id.clone(),epoch,context).with_work_steering_policy(plan.work_steering_policy)));
        let (trigger_tx,mut trigger_rx)=mpsc::channel::<WorkTrigger>(16);
        let active=Arc::new(ActiveVoice{owner:owner.to_owned(),endpoint_id:request.endpoint_id.clone(),attachment_token,attachment_expires:Instant::now()+Duration::from_secs(60),
            core:StdMutex::new(core),negotiation:negotiation.clone(),input:model_session.input.clone(),cancel:cancel.clone(),events,media_rx:Mutex::new(Some(media_rx)),supervisor:Mutex::new(None),media_attached:AtomicBool::new(false),foreground_lease:StdMutex::new((Instant::now()+Duration::from_secs(3),None)),speech_sources:StdMutex::new(initial_sources),last_work_receipt:work_projection,approvals:StdMutex::new(initial_presentations),profile_id:plan.profile_id.clone(),profile_revision:plan.profile_revision,work_steering_policy:plan.work_steering_policy,lease_record:plan.record.clone(),lease_revision:plan.lease_revision.clone(),endpoint,access,input_control_gate:Mutex::new(()),bridge:bridge.clone(),work_kinds:StdMutex::new(BTreeMap::new()),trigger_tx:trigger_tx.clone()});
        let (media_intent_tx,mut media_intent_rx)=mpsc::channel::<(WorkTrigger,crate::VoiceBridgeMediaIntent)>(16);
        let media_active=active.clone();let media_journal=journal.clone();let media_service=Arc::downgrade(self);
        let media_worker=AbortOnDropHandle::new(tokio::spawn(async move{
            let mut media_triggers=std::collections::BTreeSet::new();
            loop{tokio::select!{biased;_=media_active.cancel.cancelled()=>break,intent=media_intent_rx.recv()=>{
                let Some((trigger,intent))=intent else{break;};let upstream_trigger_id=trigger.upstream_trigger_id().to_owned();
                if !media_triggers.insert(upstream_trigger_id.clone()){continue;}
                if media_triggers.len()>512{let _=media_active.input.try_control(VoiceControl::Close{reason:VoiceCloseReason::Backlog});break;}
                let state=media_active.state();let control=match intent{
                    crate::VoiceBridgeMediaIntent::InterruptOutput=>VoiceControl::InterruptOutput{output_generation:state.output_generation+1,played:None},
                    crate::VoiceBridgeMediaIntent::MuteInput{muted}=>VoiceControl::MuteInput{muted},
                };
                let result=match media_service.upgrade(){Some(service)=>service.control(&media_active.owner,&state.voice_session_id,state.activation_epoch,control.clone()).await,None=>Err(VoiceError::new(VoiceErrorKind::Closed,"voice owner closed"))};
                let reason=match result{
                    Ok(_)=>{
                        if media_journal.append(state.voice_session_id,state.activation_epoch,format!("media-intent:{upstream_trigger_id}"),"media_control_requested".into(),None,serde_json::to_value(control).unwrap_or_default()).await.is_err(){let _=media_active.input.try_control(VoiceControl::Close{reason:VoiceCloseReason::JournalUnavailable});break;}
                        "No work submitted. The media control was accepted by the endpoint; actual playback consumption requires its receipt. Background work continues.".to_owned()
                    },Err(error)=>format!("No work submitted. Media control rejected: {}",error.message)
                };
                let _=media_active.input.control(VoiceControl::RejectWorkTrigger{upstream_trigger_id,reason},Instant::now()+Duration::from_secs(3)).await;
            }}}
        }));
        let work_last_receipt=last_receipt.clone();
        let work_active=active.clone();let work_bridge=bridge.clone();let work_journal=journal.clone();
        let correction_tx=trigger_tx.clone();
        let work_service=Arc::downgrade(self);
        let work_worker=AbortOnDropHandle::new(tokio::spawn(async move {
            loop {tokio::select!{biased;_=work_active.cancel.cancelled()=>break,trigger=trigger_rx.recv()=>{
                let Some(trigger)=trigger else{break;};
                let upstream_trigger_id=trigger.upstream_trigger_id().to_owned();
                let prepared=work_bridge.lock().await.prepare_trigger(trigger);
                let receipt=match prepared{
                    Ok(prepared)=>match prepared.execute().await{
                        Ok(executed)=>{
                            let request_kind=executed.request_kind();
                            let completion=work_bridge.lock().await.finish_trigger(executed);
                            if let Some(error)=completion.revision_error{let _=work_active.events.send(VoiceProductEvent::Model{event:VoiceModelEvent::ControlRejected{error}});}
                            if let Some(correction)=completion.revision_correction{if correction_tx.try_send(correction).is_err(){let _=work_active.input.try_control(VoiceControl::Close{reason:VoiceCloseReason::Backlog});break;}}
                            Ok((completion.receipt,request_kind))
                        },Err(error)=>Err(error)
                    },Err(error)=>Err(error)
                };
                match receipt {
                    Ok((receipt,executed_kind))=>{
                        let request_kind=work_active.request_kind(&receipt,executed_kind);
                        let mut durable_receipt=receipt.clone();durable_receipt.duplicate=false;
                        let payload=serde_json::to_value(&durable_receipt).unwrap_or_default();
                        let digest=nomifun_agent_contracts::digest_payload(&durable_receipt).map(|d|d.0).unwrap_or_default();
                        if work_journal.append(id_for(&work_active),work_active.state().activation_epoch,format!("work:{}:{digest}",receipt.operation_key),"work_receipt".into(),Some(receipt.receipt_id.clone()),payload).await.is_err(){
                            let _=work_active.input.try_control(VoiceControl::Close{reason:VoiceCloseReason::JournalUnavailable});break;
                        }
                        *work_last_receipt.lock().await=Some(receipt.clone());
                        *work_active.last_work_receipt.lock().await=Some(receipt.clone());
                        let mut fact=match admitted_receipt_fact(&work_active,work_bridge_context_port(&work_service),&work_journal,&receipt).await{Ok(fact)=>fact,Err(_)=>{let _=work_active.input.try_control(VoiceControl::Close{reason:VoiceCloseReason::JournalUnavailable});break;}};
                        fact.upstream_trigger_id=Some(upstream_trigger_id.clone());
                        if let Ok(context)=work_bridge_context(&work_service,&work_active).await{
                            if note_context_target(&work_active,context.observed_target.clone()).await.is_ok(){
                                work_active.core.lock().unwrap_or_else(|p|p.into_inner()).work_running(context.observed_target.is_some());
                                fact.work_context=Some(VoiceWorkContextFact{target:context.observed_target});
                            }
                        }
                        let _=work_active.input.control(VoiceControl::InjectFact{fact},Instant::now()+Duration::from_secs(3)).await;
                        let _=work_active.events.send(VoiceProductEvent::WorkReceipt{receipt,upstream_trigger_id:Some(upstream_trigger_id),request_kind});
                    },Err(error)=>{
                        let fatal=error.kind==VoiceErrorKind::JournalUnavailable;
                        if error.kind==VoiceErrorKind::SourceContextRequired{
                            if let Some(requirement)=work_bridge.lock().await.required_source_context(){
                                let _=work_active.events.send(VoiceProductEvent::SourceContextChanged{requirement:Some(requirement)});
                            }
                        }
                        if !fatal{let _=work_active.input.control(VoiceControl::RejectWorkTrigger{upstream_trigger_id,reason:error.message.clone()},Instant::now()+Duration::from_secs(3)).await;}
                        let _=work_active.events.send(VoiceProductEvent::Model{event:if fatal{VoiceModelEvent::Error{error}}else{VoiceModelEvent::ControlRejected{error}}});
                        if fatal{let _=work_active.input.try_control(VoiceControl::Close{reason:VoiceCloseReason::JournalUnavailable});break;}
                    }
                }
            }}}
        }));
        let observation_service=Arc::downgrade(self);let observation_active=active.clone();let observation_authority=self.authority.clone();let observation_work=self.work.clone();let observation_journal=journal.clone();let observation_id=id.clone();let observation_bridge=bridge.clone();
        let lease_record=plan.record.clone();let lease_revision=plan.lease_revision.clone();
        let observation_worker=AbortOnDropHandle::new(tokio::spawn(async move{
            let supervisor_active=observation_active;let authority=observation_authority;let work=observation_work;let journal=observation_journal;let id=observation_id;let bridge=observation_bridge;
            let mut interval=tokio::time::interval(Duration::from_millis(250));let mut last_fact=String::new();let mut last_context_digest=String::new();let mut observation_tick=0u64;let mut reason=VoiceCloseReason::UserEnded;
            loop{tokio::select!{biased;_=supervisor_active.cancel.cancelled()=>break,
                _=interval.tick()=>{
                    let state=supervisor_active.state();
                    if supervisor_active.access.as_ref().is_some_and(|lease|!lease.is_valid()){reason=VoiceCloseReason::PermissionRevoked;break;}
                    if !supervisor_active.media_attached.load(Ordering::Acquire)&&Instant::now()>supervisor_active.attachment_expires{reason=VoiceCloseReason::LeaseRevoked;break;}
                    if supervisor_active.media_attached.load(Ordering::Acquire)&&Instant::now()>supervisor_active.foreground_lease.lock().unwrap_or_else(|p|p.into_inner()).0{reason=VoiceCloseReason::LeaseRevoked;break;}
                    if authority.validate_binding(&supervisor_active.owner,&state.agent_session_id,state.binding_version).await.is_err(){reason=VoiceCloseReason::BindingChanged;break;}
                    if authority.validate_lease(&supervisor_active.owner,&state.agent_session_id,state.binding_version,&lease_record,&lease_revision).await.is_err(){reason=VoiceCloseReason::PermissionRevoked;break;}
                    observation_tick+=1;
                    let pending=last_receipt.lock().await.clone();
                    if let Some(pending)=pending {
                        if pending.status!=VoiceWorkStatus::Terminal {
                            if let Ok(Some(updated))=work.lookup_operation(&supervisor_active.owner,&state.agent_session_id,&pending.operation_key).await {
                                bridge.lock().await.note_receipt(&updated);
                                if updated!=pending {
                                    if let Some(reference)=updated.canonical_reference(){if journal.associate_receipt(updated.operation_key.clone(),reference.into()).await.is_err(){reason=VoiceCloseReason::JournalUnavailable;break;}}
                                    let request_kind=supervisor_active.request_kind(&updated,None);
                                    let _=supervisor_active.events.send(VoiceProductEvent::WorkReceipt{receipt:updated.clone(),upstream_trigger_id:None,request_kind});
                                    *last_receipt.lock().await=Some(updated);
                                    *supervisor_active.last_work_receipt.lock().await=last_receipt.lock().await.clone();
                                }
                            }
                        }
                    }
                    if observation_tick.is_multiple_of(4) {
                        let sources=supervisor_active.speech_sources.lock().unwrap_or_else(|p|p.into_inner()).references();
                        let invalid=match work.validate_source_refs(&supervisor_active.owner,&state.agent_session_id,&sources).await{Ok(invalid)=>invalid,Err(_)=>{reason=VoiceCloseReason::BindingChanged;break;}};
                        if let Some(service)=observation_service.upgrade(){for source in invalid{service.revoke_message(&state.agent_session_id,&source.message_id,source.revision).await;}}
                        if let Ok(context)=work.context(&supervisor_active.owner,&state.agent_session_id,state.binding_version).await {
                            let verified_context=VoiceWorkContextFact{target:context.observed_target.clone()};
                            if note_context_target(&supervisor_active,context.observed_target.clone()).await.is_err(){reason=VoiceCloseReason::BindingChanged;break;}
                            supervisor_active.core.lock().unwrap_or_else(|p|p.into_inner()).work_running(context.observed_target.is_some());
                            let context_digest=nomifun_agent_contracts::digest_payload(&verified_context).map(|d|d.0).unwrap_or_default();
                            if context_digest!=last_context_digest{
                                last_context_digest=context_digest.clone();
                                let canonical_ref=context.context_reference.clone();
                                if journal.append(id.clone(),epoch,format!("context:{context_digest}"),"canonical_context_observed".into(),Some(canonical_ref.clone()),serde_json::to_value(&verified_context).unwrap_or_default()).await.is_err(){reason=VoiceCloseReason::JournalUnavailable;break;}
                                let fact=VerifiedVoiceFact{correlation_id:format!("context:{context_digest}"),upstream_trigger_id:None,canonical_receipt_id:canonical_ref,content:"The application observed the current canonical work context. An operation receipt target alone does not identify the current active task.".into(),speak:false,output_generation:Some(state.output_generation),work_context:Some(verified_context),speech_source:None};
                                let _=supervisor_active.input.control(VoiceControl::InjectFact{fact},Instant::now()+Duration::from_secs(3)).await;
                            }
                            *supervisor_active.approvals.lock().unwrap_or_else(|p|p.into_inner())=context.approvals.clone();
                            let presentations=bridge.lock().await.present_approvals(context.approvals);
                            for presentation in presentations {
                                let key=format!("approval:{}",presentation.target.presentation_id);
                                if journal.append(id.clone(),epoch,key.clone(),"approval_presented".into(),Some(presentation.target.presentation_id.clone()),serde_json::to_value(&presentation).unwrap_or_default()).await.is_err(){reason=VoiceCloseReason::JournalUnavailable;break;}
                                let fact=VerifiedVoiceFact{correlation_id:key,upstream_trigger_id:None,canonical_receipt_id:format!("{}:{}",presentation.target.execution_id,presentation.target.request_event_sequence),content:serde_json::to_string(&presentation).unwrap_or_default(),speak:true,output_generation:Some(supervisor_active.state().output_generation),work_context:None,speech_source:None};
                                let _=supervisor_active.input.control(VoiceControl::InjectFact{fact},Instant::now()+Duration::from_secs(3)).await;
                                let _=supervisor_active.events.send(VoiceProductEvent::ApprovalPresented{presentation});
                            }
                        }
                    }
                    let target=bridge.lock().await.observed_target().cloned();
                    if let Some(target)=target {
                        if let Ok(receipt)=work.observe(&supervisor_active.owner,&target).await {
                            let digest=nomifun_agent_contracts::digest_payload(&receipt).map(|d|d.0).unwrap_or_default();
                            if digest!=last_fact {
                                let payload=serde_json::to_value(&receipt).unwrap_or_default();
                                if journal.append(state.voice_session_id.clone(),epoch,format!("observed:{digest}"),"canonical_observation".into(),Some(receipt.receipt_id.clone()),payload).await.is_err(){reason=VoiceCloseReason::JournalUnavailable;break;}
                                last_fact=digest;
                                let fact=match admitted_receipt_fact(&supervisor_active,Some(work.clone()),&journal,&receipt).await{Ok(fact)=>fact,Err(error)=>{reason=if error.kind==VoiceErrorKind::Backlog{VoiceCloseReason::Backlog}else{VoiceCloseReason::JournalUnavailable};break;}};
                                let _=supervisor_active.input.control(VoiceControl::InjectFact{fact},Instant::now()+Duration::from_secs(3)).await;
                                *supervisor_active.last_work_receipt.lock().await=Some(receipt.clone());
                                let request_kind=supervisor_active.request_kind(&receipt,None);
                                let _=supervisor_active.events.send(VoiceProductEvent::WorkReceipt{receipt,upstream_trigger_id:None,request_kind});
                            }
                        }
                    }

                }
            }}
            if reason!=VoiceCloseReason::UserEnded{let _=supervisor_active.input.try_control(VoiceControl::Close{reason});}
        }));
        let supervisor_active=active.clone();
        let supervisor_id=id.clone();
        let supervisor=tokio::spawn(async move {
            let id=supervisor_id;
            let mut reason=VoiceCloseReason::UserEnded;
            loop {tokio::select!{biased;
                _=supervisor_active.cancel.cancelled()=>{reason=supervisor_active.input.requested_close_reason();break;},
                changed=model_session.termination.changed()=>{
                    if changed.is_err(){reason=VoiceCloseReason::NetworkLost;break;}
                    if let Some(termination)=model_session.termination.borrow().as_ref(){reason=termination.reason;break;}
                },
                event=model_session.events.recv()=>{
                    let Some(event)=event else{reason=VoiceCloseReason::NetworkLost;break;};
                    if matches!(&event,VoiceModelEvent::Transcript{fragment} if fragment.speaker==VoiceSpeaker::User){
                        let state=supervisor_active.state();
                        if !supervisor_active.media_attached.load(Ordering::Acquire)||state.connection!=VoiceConnectionState::Ready||state.capture!=VoiceCaptureState::Capturing{continue;}
                    }
                    let accepted=supervisor_active.core.lock().unwrap_or_else(|p|p.into_inner()).accept_event(&event);
                    match accepted{Ok(false)=>continue,Err(error)=>{let _=supervisor_active.events.send(VoiceProductEvent::Model{event:VoiceModelEvent::Error{error}});reason=VoiceCloseReason::ProviderFailed;break;},_=>{}}
                    match &event {
                        VoiceModelEvent::RelayRecovering{..}=>{
                            let _=supervisor_active.endpoint.control(VoiceControl::MuteInput{muted:true},Instant::now()+Duration::from_secs(1)).await;
                            let _=supervisor_active.events.send(VoiceProductEvent::State{state:supervisor_active.state()});
                        },
                        VoiceModelEvent::RelayRecovered{..}=>{let _=supervisor_active.events.send(VoiceProductEvent::State{state:supervisor_active.state()});},
                        VoiceModelEvent::Audio{segment_id,frame}=>{
                            let segment=supervisor_active.core.lock().unwrap_or_else(|p|p.into_inner()).segment(segment_id);
                            let Some(segment)=segment else{reason=VoiceCloseReason::ProviderFailed;break;};
                            if supervisor_active.endpoint.deliver(segment,frame.clone(),Instant::now()+Duration::from_millis(200)).await.is_err(){reason=VoiceCloseReason::Backlog;break;}
                            continue;
                        },
                        VoiceModelEvent::OutputStarted{segment}=>{if supervisor_active.endpoint.register_segment(segment.clone()).is_err(){reason=VoiceCloseReason::ProviderFailed;break;}},
                        VoiceModelEvent::ContextOpened{context_window_ref,work_context}=>{
                            if bridge.lock().await.open_context(context_window_ref.clone(),work_context.clone()).is_err(){reason=VoiceCloseReason::ProviderFailed;break;}
                        },
                        VoiceModelEvent::WorkContextCheckpoint{context_window_ref,media_range,target,canonical_receipt_id}=>{
                            if let Err(error)=bridge.lock().await.context_checkpoint(context_window_ref.clone(),media_range.clone(),target.clone(),canonical_receipt_id.clone()){
                                let fatal=error.kind==VoiceErrorKind::Backlog;
                                let _=supervisor_active.events.send(VoiceProductEvent::Model{event:VoiceModelEvent::ControlRejected{error}});
                                if fatal{reason=VoiceCloseReason::Backlog;break;}
                            }
                        },
                        VoiceModelEvent::Transcript{fragment}=>{
                            let revision=bridge.lock().await.transcript_revision(fragment.clone());
                            if fragment.speaker==VoiceSpeaker::User&&fragment.commit==TranscriptCommit::Committed{
                                if supervisor_active.input.try_control(VoiceControl::PresentInputSource{fragment:fragment.clone()}).is_err(){reason=VoiceCloseReason::Backlog;break;}
                            }
                            match revision{
                                Ok(Some(trigger))=>{if trigger_tx.try_send(trigger).is_err(){reason=VoiceCloseReason::Backlog;break;}},
                                Err(error)=>{let _=supervisor_active.events.send(VoiceProductEvent::Model{event:VoiceModelEvent::ControlRejected{error}});},
                                Ok(None)=>{}
                            }
                        },
                        VoiceModelEvent::WorkTrigger{trigger}=>{
                            let state=supervisor_active.state();
                            if !supervisor_active.media_attached.load(Ordering::Acquire)||state.connection!=VoiceConnectionState::Ready||!matches!(state.capture,VoiceCaptureState::Capturing|VoiceCaptureState::Paused){
                                let _=supervisor_active.input.try_control(VoiceControl::RejectWorkTrigger{upstream_trigger_id:trigger.upstream_trigger_id().into(),reason:"voice input lease is not attached".into()});continue;
                            }
                            // Publish the actual trigger before waking the work
                            // worker, so even a fast receipt has a causal start
                            // for supplier-neutral observation. This is no grant.
                            let _=supervisor_active.events.send(VoiceProductEvent::Model{event:event.clone()});
                            let media_intent=bridge.lock().await.delegation_media_intent(trigger);
                            if let Ok(Some(intent))=media_intent{
                                if media_intent_tx.try_send((trigger.clone(),intent)).is_err(){reason=VoiceCloseReason::Backlog;break;}
                            }else if trigger_tx.try_send(trigger.clone()).is_err(){reason=VoiceCloseReason::Backlog;break;}
                            continue;
                        },
                        VoiceModelEvent::NativeAttachmentRequired{..}=>{supervisor_active.core.lock().unwrap_or_else(|p|p.into_inner()).recovering_attachment();},
                        VoiceModelEvent::Closed{termination}=>{reason=termination.reason;},_=>{}
                    }
                    if matches!(&event,VoiceModelEvent::Transcript{..}|VoiceModelEvent::OutputStarted{..}|VoiceModelEvent::OutputInterrupted{..}) {
                        let event_key=nomifun_agent_contracts::digest_payload(&event).map(|d|d.0).unwrap_or_default();
                        if journal.append(id.clone(),epoch,event_key,"voice_event".into(),None,serde_json::to_value(&event).unwrap_or_default()).await.is_err(){reason=VoiceCloseReason::JournalUnavailable;break;}
                    }
                    let closed=matches!(&event,VoiceModelEvent::Closed{..});
                    let _=supervisor_active.events.send(VoiceProductEvent::Model{event});
                    if closed{break;}
                }
            }}
            if !matches!(supervisor_active.state().connection,VoiceConnectionState::Closing|VoiceConnectionState::Closed){supervisor_active.core.lock().unwrap_or_else(|p|p.into_inner()).close(matches!(reason,VoiceCloseReason::PermissionRevoked|VoiceCloseReason::DeviceUnavailable));}
            let _=supervisor_active.endpoint.control(VoiceControl::Close{reason},Instant::now()+Duration::from_secs(1)).await;
            supervisor_active.cancel.cancel();drop(trigger_tx);drop(media_intent_tx);
            let _=supervisor_active.endpoint.shutdown(Instant::now()+Duration::from_secs(1)).await;
            tokio::join!(bounded_join(work_worker,Duration::from_secs(5)),bounded_join(observation_worker,Duration::from_secs(5)),bounded_join(media_worker,Duration::from_secs(5)));
            let termination=model_session.shutdown(reason).await;
            let _=journal.close(id.clone(),epoch,termination.clone()).await;
            let mut core=supervisor_active.core.lock().unwrap_or_else(|p|p.into_inner());
            let _=core.accept_event(&VoiceModelEvent::Closed{termination:termination.clone()});
            let _=supervisor_active.events.send(VoiceProductEvent::State{state:core.state().clone()});
            let _=supervisor_active.events.send(VoiceProductEvent::Model{event:VoiceModelEvent::Closed{termination:termination.clone()}});
            termination
        });
        *active.supervisor.lock().await=Some(supervisor);
        let mut sessions=self.sessions.lock().await;
        if sessions.len()>=128{sessions.retain(|_,prior|!matches!(prior.state().connection,VoiceConnectionState::Closed|VoiceConnectionState::Failed));}
        sessions.insert(id.clone(),active.clone());drop(sessions);
        Ok(VoiceActivationResponse{voice_session_id:id,activation_epoch:epoch,attachment_token:active.attachment_token.clone(),state:active.state(),negotiation})
    }
    async fn get(&self,owner:&str,id:&str,epoch:u64)->Result<Arc<ActiveVoice>,VoiceError> {
        let active=self.sessions.lock().await.get(id).cloned().ok_or_else(||VoiceError::new(VoiceErrorKind::Closed,"voice session not found"))?;
        if active.owner!=owner{return Err(VoiceError::new(VoiceErrorKind::Authentication,"voice owner mismatch"));}
        active.core.lock().unwrap_or_else(|p|p.into_inner()).check_epoch(epoch)?;Ok(active)
    }
    pub async fn state(&self,owner:&str,id:&str,epoch:u64)->Result<VoiceState,VoiceError>{Ok(self.get(owner,id,epoch).await?.state())}
    pub async fn active_binding(&self,owner:&str,session:&str)->Option<serde_json::Value>{
        self.sessions.lock().await.values().find(|s|s.owner==owner&&s.state().agent_session_id==session&&!matches!(s.state().connection,VoiceConnectionState::Closed|VoiceConnectionState::Failed|VoiceConnectionState::Closing))
            .map(|s|{let state=s.state();serde_json::json!({"voice_session_id":state.voice_session_id,"activation_epoch":state.activation_epoch,"state":state})})
    }
    pub async fn projection(&self,owner:&str,id:&str,epoch:u64)->Result<VoiceSessionProjection,VoiceError>{
        let active=self.get(owner,id,epoch).await?;let state=active.state();
        let (transcripts,playback_receipts)={let core=active.core.lock().unwrap_or_else(|p|p.into_inner());(core.transcripts(),core.playback_receipts())};
        let last_work_receipt=active.last_work_receipt.lock().await.clone();
        let approvals=active.approvals.lock().unwrap_or_else(|p|p.into_inner()).clone();
        let source_context_requirement=active.bridge.lock().await.required_source_context();
        Ok(VoiceSessionProjection{state,transcripts,playback_receipts,last_work_receipt,approvals,source_context_requirement})
    }
    pub async fn confirm_source_context(&self,owner:&str,id:&str,confirmation:nomifun_voice_contracts::VoiceSourceContextConfirmation)->Result<(),VoiceError>{
        let active=self.get(owner,id,confirmation.activation_epoch).await?;active.check_access()?;
        let state=active.state();
        if state.connection!=VoiceConnectionState::Ready||!active.media_attached.load(Ordering::Acquire){return Err(VoiceError::new(VoiceErrorKind::Closed,"source confirmation requires its live attached endpoint"));}
        self.authority.validate_lease(owner,&state.agent_session_id,state.binding_version,&active.lease_record,&active.lease_revision).await?;
        let context=self.work.context(owner,&state.agent_session_id,state.binding_version).await?;
        active.check_access()?;
        if active.state().connection!=VoiceConnectionState::Ready{return Err(VoiceError::new(VoiceErrorKind::Closed,"voice closed during source confirmation"));}
        note_context_target(&active,context.observed_target).await?;
        let mut bridge=active.bridge.lock().await;
        let expected=bridge.required_source_context().ok_or_else(||VoiceError::new(VoiceErrorKind::StaleBinding,"source context confirmation expired"))?;
        if expected.source_ref!=confirmation.source_ref||expected.context.context_key!=confirmation.context_key{return Err(VoiceError::new(VoiceErrorKind::StaleBinding,"source context presentation changed"));}
        bridge.confirm_source_context(confirmation.source_ref,&confirmation.context_key)?;
        let trigger=bridge.take_confirmed_source_trigger().ok_or_else(||VoiceError::new(VoiceErrorKind::StaleBinding,"original unreserved voice request is no longer available"))?;
        if active.trigger_tx.try_send(trigger).is_err(){let _=active.input.try_control(VoiceControl::Close{reason:VoiceCloseReason::Backlog});return Err(VoiceError::new(VoiceErrorKind::Backlog,"voice retry queue exhausted"));}
        let _=active.events.send(VoiceProductEvent::SourceContextChanged{requirement:None});
        Ok(())
    }
    pub async fn attach_media(&self,owner:&str,id:&str,token:&str)->Result<VoiceMediaAttachment,VoiceError> {
        let active=self.sessions.lock().await.get(id).cloned().ok_or_else(||VoiceError::new(VoiceErrorKind::Closed,"voice session not found"))?;
        if active.owner!=owner||active.attachment_token!=token||Instant::now()>active.attachment_expires{return Err(VoiceError::new(VoiceErrorKind::Authentication,"invalid or expired voice attachment"));}
        active.check_access()?;
        let state=active.state();self.authority.validate_lease(owner,&state.agent_session_id,state.binding_version,&active.lease_record,&active.lease_revision).await?;
        if active.state().connection!=VoiceConnectionState::Ready{return Err(VoiceError::new(VoiceErrorKind::Closed,"voice activation is no longer ready"));}
        let frames=active.media_rx.lock().await.take().ok_or_else(||VoiceError::new(VoiceErrorKind::StaleEpoch,"media attachment already consumed"))?;
        {
            let mut core=active.core.lock().unwrap_or_else(|p|p.into_inner());
            if core.state().connection!=VoiceConnectionState::Ready{return Err(VoiceError::new(VoiceErrorKind::Closed,"voice activation closed during attachment"));}
            core.capture(true);*active.foreground_lease.lock().unwrap_or_else(|p|p.into_inner())=(Instant::now()+Duration::from_secs(3),None);active.media_attached.store(true,Ordering::Release);
        }
        let state=active.state();let _=active.events.send(VoiceProductEvent::State{state:state.clone()});
        Ok(VoiceMediaAttachment{frames,events:active.events.subscribe(),state,max_age:Duration::from_micros(active.negotiation.output_spec.as_ref().map_or(200_000,|s|u64::from(s.max_frame_age_us))),controls:active.endpoint.subscribe_controls(),endpoint:active.endpoint.clone()})
    }
    pub async fn capability_owner(&self,id:&str,token:&str)->Result<String,VoiceError>{
        let active=self.sessions.lock().await.get(id).cloned().ok_or_else(||VoiceError::new(VoiceErrorKind::Authentication,"invalid voice attachment"))?;
        if active.attachment_token!=token||Instant::now()>active.attachment_expires{return Err(VoiceError::new(VoiceErrorKind::Authentication,"invalid or expired voice attachment"));}
        active.check_access()?;
        Ok(active.owner.clone())
    }
    pub async fn audio(&self,owner:&str,id:&str,frame:AudioFrame)->Result<(),VoiceError> {
        let active=self.get(owner,id,frame.activation_epoch).await?;
        active.check_access()?;
        if active.state().capture==VoiceCaptureState::Paused{
            let spec=active.negotiation.input_spec.as_ref().ok_or_else(||VoiceError::new(VoiceErrorKind::Unsupported,"native media cannot accept relay input"))?;
            frame.validate_for(spec).map_err(|message|VoiceError::new(VoiceErrorKind::Configuration,message))?;
            let _=active.events.send(VoiceProductEvent::InputReleased{activation_epoch:frame.activation_epoch,sequence:frame.sequence,duration_us:frame.duration_us});return Ok(());
        }
        active.core.lock().unwrap_or_else(|p|p.into_inner()).admit_input(&frame)?;
        let credit=VoiceProductEvent::InputAdmitted{activation_epoch:frame.activation_epoch,sequence:frame.sequence,duration_us:frame.duration_us};
        if let Err(error)=active.input.try_audio(frame){let _=active.input.try_control(VoiceControl::Close{reason:VoiceCloseReason::Backlog});return Err(error);}
        let _=active.events.send(credit);Ok(())
    }
    pub async fn foreground(&self,owner:&str,id:&str,epoch:u64,sequence:u64)->Result<(),VoiceError>{
        let active=self.get(owner,id,epoch).await?;
        active.check_access()?;
        if !active.media_attached.load(Ordering::Acquire)||!matches!(active.state().connection,VoiceConnectionState::Ready|VoiceConnectionState::Recovering){return Err(VoiceError::new(VoiceErrorKind::Closed,"foreground voice lease is inactive"));}
        let mut lease=active.foreground_lease.lock().unwrap_or_else(|p|p.into_inner());
        if lease.1.is_some_and(|prior|sequence<=prior){return Err(VoiceError::new(VoiceErrorKind::StaleEpoch,"late foreground lease heartbeat"));}
        if Instant::now()>lease.0{return Err(VoiceError::new(VoiceErrorKind::Closed,"foreground voice lease expired"));}
        *lease=(Instant::now()+Duration::from_secs(3),Some(sequence));Ok(())
    }
    pub async fn work_command(&self,owner:&str,id:&str,command:nomifun_voice_contracts::VoiceWorkCommand)->Result<VoiceWorkReceipt,VoiceError>{
        let active=self.get(owner,id,command.activation_epoch).await?;let state=active.state();
        if !active.media_attached.load(Ordering::Acquire)||state.connection!=VoiceConnectionState::Ready||active.access.as_ref().is_some_and(|lease|!lease.is_valid()){
            return Err(VoiceError::new(VoiceErrorKind::Closed,"voice UI work requires its active input lease"));
        }
        active.check_access()?;
        self.authority.validate_lease(owner,&state.agent_session_id,state.binding_version,&active.lease_record,&active.lease_revision).await?;
        let journal=self.journal().await?;let digest=nomifun_agent_contracts::digest_payload(&command.request).map_err(|error|VoiceError::new(VoiceErrorKind::Configuration,error.to_string()))?.0;
        let submitted_kind=command.request.kind();
        let(link,fresh)=journal.reserve_user_command(id.into(),state.activation_epoch,command.operation_key,digest).await?;
        let receipt=match self.work.lookup_operation(owner,&state.agent_session_id,&link.operation_key).await?{
            Some(receipt)=>receipt,
            None if fresh=>self.work.interact_with_policy(owner,&state.agent_session_id,state.binding_version,&link.operation_key,command.request,active.work_steering_policy).await?,
            None=>return Err(VoiceError::new(VoiceErrorKind::JournalUnavailable,"voice UI command outcome is unconfirmed; lookup its original receipt")),
        };
        if let Some(reference)=receipt.canonical_reference(){journal.associate_receipt(link.operation_key.clone(),reference.into()).await?;}
        let request_kind=active.request_kind(&receipt,Some(submitted_kind));
        let _=active.events.send(VoiceProductEvent::WorkReceipt{receipt:receipt.clone(),upstream_trigger_id:None,request_kind});
        *active.last_work_receipt.lock().await=Some(receipt.clone());Ok(receipt)
    }
    pub async fn control(&self,owner:&str,id:&str,epoch:u64,control:VoiceControl)->Result<VoiceState,VoiceError> {
        let active=self.get(owner,id,epoch).await?;
        if !matches!(&control,VoiceControl::Close{..}|VoiceControl::Playback{..}){active.check_access()?;}
        if !active.media_attached.load(Ordering::Acquire)&&!matches!(&control,VoiceControl::Close{..}){
            return Err(VoiceError::new(VoiceErrorKind::Closed,"voice media input lease is not attached"));
        }
        if matches!(active.state().connection,VoiceConnectionState::Closing|VoiceConnectionState::Closed|VoiceConnectionState::Failed)
            && !matches!(&control,VoiceControl::Playback{..}|VoiceControl::Close{..}){
            return Err(VoiceError::new(VoiceErrorKind::Closed,"voice input lease is closed"));
        }
        match &control {
            VoiceControl::MuteInput{muted}=>{
                let _control=active.input_control_gate.lock().await;
                active.check_access()?;
                let request_id=uuid::Uuid::now_v7().to_string();let mut confirmations=active.events.subscribe();
                // Local stop is immediate. Resume waits for the exact supplier
                // or owned relay input-gate acknowledgement.
                active.core.lock().unwrap_or_else(|p|p.into_inner()).capture(false);
                let _=active.events.send(VoiceProductEvent::State{state:active.state()});
                if *muted{active.endpoint.control(control.clone(),Instant::now()+Duration::from_secs(1)).await?;}
                active.input.control(VoiceControl::SetInputMuted{muted:*muted,request_id:request_id.clone()},Instant::now()+Duration::from_secs(3)).await?;
                let confirmed=tokio::time::timeout(Duration::from_secs(3),async{
                    loop{tokio::select!{biased;_=active.cancel.cancelled()=>return Err(VoiceError::new(VoiceErrorKind::Closed,"voice closed while awaiting input control")),event=confirmations.recv()=>match event{
                        Ok(VoiceProductEvent::Model{event:VoiceModelEvent::InputMuteApplied{muted:actual,request_id:actual_id}}) if actual_id==request_id&&actual==*muted=>return Ok(()),
                        Ok(VoiceProductEvent::Model{event:VoiceModelEvent::Error{error}|VoiceModelEvent::ControlRejected{error}})=>return Err(error),
                        Ok(VoiceProductEvent::Model{event:VoiceModelEvent::Closed{..}})|Err(_)=>return Err(VoiceError::new(VoiceErrorKind::Closed,"input control acknowledgement was lost")),_=>{}
                    }}}
                }).await;
                match confirmed{
                    Ok(Ok(()))=>{},Ok(Err(error))=>{let _=active.input.try_control(VoiceControl::Close{reason:VoiceCloseReason::ProviderFailed});return Err(error);},
                    Err(_)=>{let _=active.input.try_control(VoiceControl::Close{reason:VoiceCloseReason::ProviderFailed});return Err(VoiceError::new(VoiceErrorKind::Deadline,"voice input control was not acknowledged"));}
                }
                if active.state().connection!=VoiceConnectionState::Ready{return Err(VoiceError::new(VoiceErrorKind::Closed,"voice closed after input control"));}
                if !*muted{active.endpoint.control(control.clone(),Instant::now()+Duration::from_secs(1)).await?;}
                active.core.lock().unwrap_or_else(|p|p.into_inner()).capture(!*muted);
                let state=active.state();let _=active.events.send(VoiceProductEvent::State{state:state.clone()});return Ok(state);
            },
            VoiceControl::InterruptOutput{output_generation,played}=>{
                {
                    let mut core=active.core.lock().unwrap_or_else(|p|p.into_inner());
                    if *output_generation==core.state().output_generation{return Ok(core.state().clone());}
                    if *output_generation!=core.state().output_generation+1{return Err(VoiceError::new(VoiceErrorKind::StaleEpoch,"invalid output generation"));}
                    if let Some(receipt)=played{active.endpoint.record_receipt(receipt.clone())?;core.receipt(receipt.clone())?;}
                    core.interrupt_output(active.negotiation.native_attachment.is_some());
                }
                if let Some(receipt)=played{
                    let journal=self.journal().await?;
                    if let Err(error)=journal.append_playback(id.into(),epoch,format!("interrupted:{}:{}:{}",receipt.segment_id,receipt.revision,receipt.consumed_us),receipt.clone()).await{
                        let _=active.input.try_control(VoiceControl::Close{reason:VoiceCloseReason::JournalUnavailable});return Err(error);
                    }
                }
            },
            VoiceControl::Playback{receipt}=>{
                active.endpoint.record_receipt(receipt.clone())?;
                active.core.lock().unwrap_or_else(|p|p.into_inner()).receipt(receipt.clone())?;
                if let Err(error)=self.journal().await?.append_playback(id.into(),epoch,format!("playback:{}:{}:{}:{:?}",receipt.segment_id,receipt.revision,receipt.consumed_us,receipt.state),receipt.clone()).await{
                    let _=active.input.try_control(VoiceControl::Close{reason:VoiceCloseReason::JournalUnavailable});return Err(error);
                }
                let state=active.state();let _=active.events.send(VoiceProductEvent::State{state:state.clone()});return Ok(state);
            },
            VoiceControl::Close{reason}=>{self.close_active(active.clone(),*reason).await;return Ok(active.state());},
            // Endpoint requests cannot mint verified canonical facts or rewrite frozen model instructions.
            VoiceControl::InjectFact{..}|VoiceControl::RevokeSpeech{..}|VoiceControl::UpdateConfiguration{..}|VoiceControl::RejectWorkTrigger{..}|VoiceControl::SetInputMuted{..}|VoiceControl::PresentInputSource{..}=>return Err(VoiceError::new(VoiceErrorKind::Unsupported,"control is application-owned")),
        }
        active.endpoint.control(control.clone(),Instant::now()+Duration::from_secs(1)).await?;
        active.input.control(control,Instant::now()+Duration::from_secs(3)).await?;
        let state=active.state();let _=active.events.send(VoiceProductEvent::State{state:state.clone()});Ok(state)
    }
    pub async fn rebuild_native(self:&Arc<Self>,owner:&str,id:&str,epoch:u64,offer:String)->Result<VoiceActivationResponse,VoiceError> {
        let active=self.get(owner,id,epoch).await?;let state=active.state();
        if active.negotiation.native_attachment.is_none(){return Err(VoiceError::new(VoiceErrorKind::Unsupported,"session does not use native media"));}
        self.close_active(active.clone(),VoiceCloseReason::UserEnded).await;
        self.activate_with_id(owner,VoiceActivationRequest{agent_session_id:state.agent_session_id,binding_version:state.binding_version,endpoint_id:active.endpoint_id.clone(),profile_id:active.profile_id.clone(),profile_revision:active.profile_revision,transport:VoiceTransportPreference::NativeWebrtc,native_offer:Some(offer),takeover:true},Some(id.to_owned()),active.access.clone()).await
    }
    async fn close_active(&self,active:Arc<ActiveVoice>,reason:VoiceCloseReason) {
        if !matches!(active.state().connection,VoiceConnectionState::Closed|VoiceConnectionState::Closing){
            active.core.lock().unwrap_or_else(|p|p.into_inner()).close(matches!(reason,VoiceCloseReason::DeviceUnavailable|VoiceCloseReason::PermissionRevoked));
            let _=active.events.send(VoiceProductEvent::State{state:active.state()});
            let _=active.input.try_control(VoiceControl::Close{reason});
            let _=active.endpoint.control(VoiceControl::Close{reason},Instant::now()+Duration::from_secs(1)).await;
        }
        if let Some(mut worker)=active.supervisor.lock().await.take(){
            if tokio::time::timeout(Duration::from_secs(12),&mut worker).await.is_err(){worker.abort();let _=worker.await;}
        }
    }
    pub async fn revoke_session(&self,session:&str,reason:VoiceCloseReason)->Result<(),VoiceError> {
        let sessions=self.sessions.lock().await.values().filter(|s|s.state().agent_session_id==session).cloned().collect::<Vec<_>>();
        for active in sessions{self.close_active(active,reason).await;}Ok(())
    }
    pub async fn delete_session(&self,session:&str)->Result<(),VoiceError> {
        self.revoke_session(session,VoiceCloseReason::BindingChanged).await?;
        if self.journal_store.opened().await.is_none()&&!self.data_dir.join("voice/voice.sqlite3").exists(){return Ok(());}
        let journal=self.journal().await?;journal.request_delete(session.into()).await?;journal.finish_delete(session.into()).await
    }
    pub async fn delete_owned_data(&self,owner:&str,session:&str,include_profiles:bool)->Result<(),VoiceError>{
        if self.journal_store.opened().await.is_none()&&!self.data_dir.join("voice/voice.sqlite3").exists(){return Ok(());}
        let journal=self.journal().await?;
        journal.begin_owned_session_delete(owner.into(),session.into(),include_profiles).await?;
        self.revoke_session(session,VoiceCloseReason::BindingChanged).await?;
        journal.finish_owned_session_delete(owner.into(),session.into(),include_profiles).await
    }
    pub async fn export_owned_data(&self,owner:&str)->Result<crate::VoiceSnapshotExport,VoiceError>{
        if self.journal_store.opened().await.is_none()&&!self.data_dir.join("voice/voice.sqlite3").exists(){return Err(VoiceError::new(VoiceErrorKind::Unsupported,"no voice data to export"));}
        self.journal().await?.export_snapshot(owner.into()).await
    }
    pub async fn revoke_message(&self,session:&str,message:&str,revision:u64){
        let sessions=self.sessions.lock().await.values().filter(|s|s.state().agent_session_id==session).cloned().collect::<Vec<_>>();
        for active in sessions{
            let old_state=active.state();
            if matches!(old_state.connection,VoiceConnectionState::Closing|VoiceConnectionState::Closed|VoiceConnectionState::Failed){continue;}
            let revoke=match active.speech_sources.lock().unwrap_or_else(|p|p.into_inner()).revoke(message,revision,old_state.output_generation){Ok(revoke)=>revoke,Err(_)=>{let _=active.input.try_control(VoiceControl::Close{reason:VoiceCloseReason::Backlog});continue;}};
            let control=revoke.then(||active.core.lock().unwrap_or_else(|p|p.into_inner()).interrupt_output(active.negotiation.native_attachment.is_some()));
            let state=active.state();
            if revoke{let _=active.events.send(VoiceProductEvent::SpeechRevoked{source_message_id:message.into(),source_revision:revision,output_generation:state.output_generation});let _=active.events.send(VoiceProductEvent::State{state:state.clone()});}
            if let Ok(journal)=self.journal().await{
                if journal.append(state.voice_session_id.clone(),state.activation_epoch,format!("revoke:{message}:{revision}"),"speech_delivery_revoked".into(),Some(message.into()),serde_json::json!({"revision":revision,"output_generation":state.output_generation,"consumed_facts_preserved":true})).await.is_err(){let _=active.input.try_control(VoiceControl::Close{reason:VoiceCloseReason::JournalUnavailable});continue;}
            }
            if let Some(control)=&control{let _=active.endpoint.control(control.clone(),Instant::now()+Duration::from_secs(1)).await;}
            let source=VoiceSpeechSourceRef{message_id:message.into(),revision,through_seq:revision};
            let revoked=active.input.control(VoiceControl::RevokeSpeech{source,output_generation:state.output_generation},Instant::now()+Duration::from_secs(3)).await;
            if revoked.is_err(){let _=active.input.try_control(VoiceControl::Close{reason:VoiceCloseReason::ProviderFailed});continue;}
            if let Some(control)=control{let _=active.input.control(control,Instant::now()+Duration::from_secs(3)).await;}
        }
    }
    pub async fn shutdown(&self) {
        self.shutdown.cancel();let sessions=self.sessions.lock().await.values().cloned().collect::<Vec<_>>();
        for active in sessions{self.close_active(active,VoiceCloseReason::AppShutdown).await;}
        let _=self.work.shutdown_pending_inputs().await;
    }
}
impl Drop for VoiceSessionService{
    fn drop(&mut self){self.shutdown.cancel();for active in self.sessions.get_mut().values(){let _=active.input.try_control(VoiceControl::Close{reason:VoiceCloseReason::AppShutdown});}}
}
fn id_for(active:&ActiveVoice)->String{active.state().voice_session_id}
async fn note_context_target(active:&ActiveVoice,target:Option<nomifun_voice_contracts::WorkTarget>)->Result<(),VoiceError>{
    let mut bridge=active.bridge.lock().await;
    let before=bridge.required_source_context();bridge.note_context_target(target)?;
    let requirement=bridge.required_source_context();
    if before!=requirement{
        let _=active.events.send(VoiceProductEvent::SourceContextChanged{requirement});
    }
    Ok(())
}
fn receipt_fact(receipt:&VoiceWorkReceipt)->VerifiedVoiceFact{VerifiedVoiceFact{correlation_id:receipt.operation_key.clone(),upstream_trigger_id:None,canonical_receipt_id:receipt.receipt_id.clone(),
    content:serde_json::to_string(receipt).unwrap_or_default(),speak:true,output_generation:None,work_context:None,speech_source:receipt.speech.as_ref().map(speech_source)}}
fn speech_source(speech:&VoiceSpeechProjection)->VoiceSpeechSourceRef{VoiceSpeechSourceRef{message_id:speech.message_id.clone(),revision:speech.revision,through_seq:speech.through_seq}}
fn work_bridge_context_port(service:&std::sync::Weak<VoiceSessionService>)->Option<Arc<dyn VoiceWorkPort>>{service.upgrade().map(|service|service.work.clone())}
async fn admitted_receipt_fact(active:&ActiveVoice,work:Option<Arc<dyn VoiceWorkPort>>,journal:&VoiceJournal,receipt:&VoiceWorkReceipt)->Result<VerifiedVoiceFact,VoiceError>{
    let state=active.state();let mut admitted=receipt.clone();
    if let Some(speech)=&receipt.speech{
        let source=speech_source(speech);
        let registered=active.speech_sources.lock().unwrap_or_else(|p|p.into_inner()).register(source.clone(),state.output_generation)?;
        let valid=if registered{match work{Some(work)=>work.validate_speech(&active.owner,&state.agent_session_id,speech).await.unwrap_or(false),None=>false}}else{false};
        let current=active.speech_sources.lock().unwrap_or_else(|p|p.into_inner()).allows(&source,state.output_generation);
        if valid&&current{
            journal.append(state.voice_session_id.clone(),state.activation_epoch,format!("delivery:{}:{}:{}",speech.message_id,speech.revision,state.output_generation),"speech_delivery_generated".into(),Some(receipt.receipt_id.clone()),serde_json::json!({"source":source,"output_generation":state.output_generation,"projection":speech})).await?;
        }else{admitted.speech=None;}
    }
    let mut fact=receipt_fact(&admitted);fact.output_generation=Some(state.output_generation);Ok(fact)
}
async fn work_bridge_context(service:&std::sync::Weak<VoiceSessionService>,active:&ActiveVoice)->Result<crate::VoiceWorkContext,VoiceError>{
    let service=service.upgrade().ok_or_else(||VoiceError::new(VoiceErrorKind::Closed,"voice owner closed"))?;let state=active.state();
    service.work.context(&active.owner,&state.agent_session_id,state.binding_version).await
}
fn system_now_ms()->i64{std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis().min(i64::MAX as u128) as i64}
async fn bounded_join(mut worker:AbortOnDropHandle<()>,timeout:Duration){if tokio::time::timeout(timeout,&mut worker).await.is_err(){worker.abort();let _=worker.await;}}
#[cfg(test)]
#[path="service/tests.rs"]
mod tests;
