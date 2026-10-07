//! In-process production-owner integration. No provider credentials, network or device is simulated as accepted.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicUsize, Ordering}};
use std::time::Duration;
use async_trait::async_trait;
use nomifun_voice_contracts::{DigestHex, WorkTarget, voice::*};
use nomifun_voice_core::{VoiceEventSender, VoiceModelPort, VoiceModelSession, VoiceOpenRequest, VoiceSessionLimits};
use tokio::sync::Notify;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use crate::*;

const OWNER: &str = "owner";
const SESSION: &str = "agent-session";
#[tokio::test]
async fn canonical_source_watcher_retracts_cached_initial_text_and_observer_projection_without_stopping_work(){
    let h=Harness::new();let speech=VoiceSpeechProjection{message_id:"canonical-source".into(),revision:7,through_seq:7,text:"old source words".into()};
    *h.work.speech_source.lock().unwrap()=Some((speech,true));let a=h.activate("source-watch").await;
    let mut media=h.service.attach_media(OWNER,&a.voice_session_id,&a.attachment_token).await.unwrap();
    h.adapter.emit(0,VoiceModelEvent::OutputStarted{segment:OutputSegment{response_id:"source-reply".into(),segment_id:"source-audio".into(),revision:1,output_generation:1,text:Some("generated old words".into()),correlation_id:None}}).await;
    h.adapter.emit(0,VoiceModelEvent::Transcript{fragment:TranscriptFragment{speaker:VoiceSpeaker::Assistant,fragment_id:"caption-old".into(),revision:1,commit:TranscriptCommit::Committed,text:"generated old words".into(),media_range:None}}).await;
    h.adapter.emit(0,VoiceModelEvent::Audio{segment_id:"source-audio".into(),frame:frame(a.activation_epoch,1)}).await;
    let held=tokio::time::timeout(Duration::from_secs(1),media.frames.recv()).await.unwrap().unwrap();
    h.service.control(OWNER,&a.voice_session_id,a.activation_epoch,VoiceControl::Playback{receipt:PlaybackReceipt{activation_epoch:a.activation_epoch,segment_id:"source-audio".into(),revision:1,output_generation:1,state:DeliveryState::Played,consumed_us:9000,uncertain_tail_us:1000,precision:PlaybackPrecision::Estimated}}).await.unwrap();
    h.work.speech_source.lock().unwrap().as_mut().unwrap().1=false;
    wait_until(||h.adapter.controls.lock().unwrap().iter().any(|control|matches!(control,VoiceControl::RevokeSpeech{source,..} if source.message_id=="canonical-source"&&source.revision==7))).await;
    assert!(!held.is_current());let projection=h.service.projection(OWNER,&a.voice_session_id,a.activation_epoch).await.unwrap();
    assert!(projection.transcripts.iter().all(|fragment|fragment.speaker==VoiceSpeaker::User));assert_eq!(projection.playback_receipts[0].consumed_us,9000);
    assert_eq!(projection.state.connection,VoiceConnectionState::Ready);assert!(h.work.requests.lock().unwrap().is_empty());
    assert!(h.journal_rows("speech_delivery_revoked").iter().any(|row|row["revision"]==7));h.service.shutdown().await;
}
#[tokio::test]
async fn profile_probe_uses_registered_port_and_joins_without_media_or_task_admission(){
    let h=Harness::new();let request=nomifun_voice_contracts::VoiceProfileProbeRequest{agent_session_id:SESSION.into(),binding_version:1,profile_revision:1,native_offer:None};
    let result=h.service.probe_profile(OWNER,"profile",request,Arc::new(Access(AtomicBool::new(true)))).await.unwrap();
    assert!(result.connection_verified);assert!(result.termination.finalization_confirmed);assert_eq!(h.adapter.live_workers.load(Ordering::SeqCst),0);assert!(h.work.requests.lock().unwrap().is_empty());
    assert!(!h.dir.path().join("voice/voice.sqlite3").exists());assert!(h.adapter.opens.lock().unwrap()[0].initial_facts.is_empty());assert!(h.adapter.opens.lock().unwrap()[0].tools.is_empty());
}

#[tokio::test]
async fn foreground_heartbeat_is_independent_of_microphone_and_expiry_never_cancels_work() {
    let h=Harness::new();let a=h.activate("mobile-foreground").await;
    let _media=h.service.attach_media(OWNER,&a.voice_session_id,&a.attachment_token).await.unwrap();
    h.service.control(OWNER,&a.voice_session_id,a.activation_epoch,VoiceControl::MuteInput{muted:true}).await.unwrap();
    tokio::time::pause();
    for sequence in 1..=5 {
        tokio::time::advance(Duration::from_secs(1)).await;
        h.service.foreground(OWNER,&a.voice_session_id,a.activation_epoch,sequence).await.unwrap();
    }
    assert_eq!(h.state(&a).await.capture,VoiceCaptureState::Paused);
    assert_eq!(h.service.foreground(OWNER,&a.voice_session_id,a.activation_epoch,5).await.unwrap_err().kind,VoiceErrorKind::StaleEpoch);
    tokio::time::advance(Duration::from_secs(4)).await;
    assert_eq!(h.service.foreground(OWNER,&a.voice_session_id,a.activation_epoch,6).await.unwrap_err().kind,VoiceErrorKind::Closed);
    assert_eq!(h.service.audio(OWNER,&a.voice_session_id,frame(a.activation_epoch,1)).await.unwrap_err().kind,VoiceErrorKind::Closed);
    tokio::time::resume();wait_closed(&h,&a).await;
    assert!(h.work.requests.lock().unwrap().is_empty());
    assert_eq!(h.adapter.live_workers.load(Ordering::SeqCst),0);h.service.shutdown().await;
}

struct Access(AtomicBool);
impl VoiceAccessLease for Access{fn is_valid(&self)->bool{self.0.load(Ordering::SeqCst)}}
#[tokio::test]
async fn revoked_authorization_cannot_consume_attachment_or_admit_ui_work() {
    let h=Harness::new();let access=Arc::new(Access(AtomicBool::new(true)));
    let a=h.service.activate_with_access(OWNER,request("mobile-auth",false),access.clone()).await.unwrap();
    access.0.store(false,Ordering::SeqCst);
    assert_eq!(h.service.capability_owner(&a.voice_session_id,&a.attachment_token).await.unwrap_err().kind,VoiceErrorKind::Authentication);
    assert_eq!(h.service.attach_media(OWNER,&a.voice_session_id,&a.attachment_token).await.err().unwrap().kind,VoiceErrorKind::Authentication);
    assert!(h.work.requests.lock().unwrap().is_empty());h.service.shutdown().await;
}

#[tokio::test]
async fn provider_transcripts_before_attachment_or_after_mute_never_authorize_later_work(){
    let h=Harness::new();let a=h.activate("mobile-before-input").await;
    let transcript=VoiceModelEvent::Transcript{fragment:TranscriptFragment{speaker:VoiceSpeaker::User,fragment_id:"pre-attach".into(),revision:1,commit:TranscriptCommit::Committed,text:"Prepare a report".into(),media_range:None}};
    h.adapter.emit(0,transcript.clone()).await;h.adapter.emit(0,start_trigger("barrier-pre-attach",None)).await;
    wait_until(||h.adapter.controls.lock().unwrap().iter().any(|control|matches!(control,VoiceControl::RejectWorkTrigger{upstream_trigger_id,..} if upstream_trigger_id=="barrier-pre-attach"))).await;
    let _media=h.service.attach_media(OWNER,&a.voice_session_id,&a.attachment_token).await.unwrap();
    h.adapter.emit(0,start_trigger("late-from-pre-attach",None)).await;
    wait_until(||h.adapter.controls.lock().unwrap().iter().any(|control|matches!(control,VoiceControl::RejectWorkTrigger{upstream_trigger_id,..} if upstream_trigger_id=="late-from-pre-attach"))).await;
    h.service.control(OWNER,&a.voice_session_id,a.activation_epoch,VoiceControl::MuteInput{muted:true}).await.unwrap();
    h.adapter.emit(0,transcript).await;h.adapter.emit(0,start_trigger("muted-transcript",None)).await;
    wait_until(||h.adapter.controls.lock().unwrap().iter().any(|control|matches!(control,VoiceControl::RejectWorkTrigger{upstream_trigger_id,..} if upstream_trigger_id=="muted-transcript"))).await;
    assert!(h.service.projection(OWNER,&a.voice_session_id,a.activation_epoch).await.unwrap().transcripts.is_empty());
    assert!(h.work.requests.lock().unwrap().is_empty());h.service.shutdown().await;
}

#[tokio::test]
async fn ui_retry_keys_are_immutable_and_profile_cas_is_voice_local() {
    let h=Harness::new();let journal=h.service.journal().await.unwrap();
    let profile=nomifun_voice_contracts::VoiceProfile{profile_id:"profile".into(),revision:1,agent_session_id:SESSION.into(),binding_version:1,enabled:true,label:"Mobile voice".into(),route:h.authority.plan.record.clone(),work_steering_policy:Default::default()};
    journal.save_profile(OWNER.into(),profile.clone(),0).await.unwrap();
    assert!(journal.save_profile("other-owner".into(),profile.clone(),0).await.is_err());
    let mut overflow=profile.clone();overflow.revision=0;
    assert!(journal.save_profile(OWNER.into(),overflow,u64::MAX).await.is_err());
    assert!(journal.save_profile(OWNER.into(),profile.clone(),0).await.is_err());
    let a=h.activate("mobile-ui").await;
    let _media=h.service.attach_media(OWNER,&a.voice_session_id,&a.attachment_token).await.unwrap();
    let command=nomifun_voice_contracts::VoiceWorkCommand{activation_epoch:a.activation_epoch,operation_key:"tap-1".into(),request:VoiceWorkRequest::Cancel{target:target()}};
    let first=h.service.work_command(OWNER,&a.voice_session_id,command.clone()).await.unwrap();
    let replay=h.service.work_command(OWNER,&a.voice_session_id,command.clone()).await.unwrap();
    assert_eq!(first.receipt_id,replay.receipt_id);assert_eq!(h.work.requests.lock().unwrap().len(),1);
    let mut changed=command;changed.request=VoiceWorkRequest::Start{text:"different command".into()};
    assert_eq!(h.service.work_command(OWNER,&a.voice_session_id,changed).await.unwrap_err().kind,VoiceErrorKind::Configuration);
    h.service.delete_session(SESSION).await.unwrap();
    assert!(journal.profile(OWNER.into(),"profile".into()).await.unwrap().is_none());
    assert!(!h.dir.path().join("nomifun-backend.db").exists());h.service.shutdown().await;
}
fn spec() -> MediaSpec {
    MediaSpec { format: AudioFormat::pcm16(16_000, 1), timebase: 16_000,
        min_frame_duration_us: 10_000, max_frame_duration_us: 20_000, max_frame_bytes: 640,
        max_buffer_duration_us: 100_000, max_frame_age_us: 1_000_000 }
}
fn capabilities() -> VoiceCapabilities {
    native_duplex_requirements().into_iter().chain([VoiceFeature::TypedTools])
        .map(|feature| (feature, VoiceCapabilityEvidence { support: CapabilitySupport::Supported,
            source: "local lifecycle contract harness; no acoustic/vendor acceptance claim".into() })).collect()
}
fn descriptor() -> VoiceAdapterDescriptor {
    VoiceAdapterDescriptor { adapter_id: "test.lifecycle".into(), contract_version: 1, api_version: "local-1".into(),
        config_schema_version: 1, label: "In-process lifecycle port".into(), capabilities: capabilities(),
        transports: vec![VoiceTransportPreference::Relay], input_specs: vec![spec()], output_specs: vec![spec()],
        native_requirements: None }
}
fn target() -> WorkTarget {
    serde_json::from_value(serde_json::json!({"agent_session_id":SESSION,"binding_version":1,
        "turn_operation_id":"original-work","execution_generation":1})).unwrap()
}
fn receipt(key: &str) -> VoiceWorkReceipt {
    VoiceWorkReceipt { receipt_id: format!("canonical:{key}"), operation_key: key.into(), target: Some(target()),
        pending_input_id: None, status: VoiceWorkStatus::Applied, summary: "Canonical work remains active".into(),
        duplicate: false, speech: None, pending_input_revision: None,first_claim_generation:None }
}
fn request(endpoint: &str, takeover: bool) -> VoiceActivationRequest {
    VoiceActivationRequest { agent_session_id: SESSION.into(), binding_version: 1,profile_id:"profile".into(),profile_revision:1, endpoint_id: endpoint.into(),
        transport: VoiceTransportPreference::Relay, native_offer: None, takeover }
}
fn frame(epoch: u64, sequence: u64) -> AudioFrame {
    AudioFrame { activation_epoch: epoch, output_generation: 1, sequence, timestamp: sequence * 320,
        duration_us: 20_000, format: spec().format, payload: vec![0; 640] }
}
fn start_trigger(id: &str, window: Option<&str>) -> VoiceModelEvent {
    VoiceModelEvent::WorkTrigger { trigger: WorkTrigger::TypedToolCall { upstream_trigger_id: id.into(),
        name: "nomi_work".into(), arguments: serde_json::json!({"action":"start","text":"Prepare the requested report"}),
        transcript_window_ref: window.map(str::to_owned) } }
}

#[derive(Default)]
struct LocalAdapter {
    opens: Mutex<Vec<VoiceOpenRequest>>,
    senders: Mutex<Vec<VoiceEventSender>>,
    controls: Arc<Mutex<Vec<VoiceControl>>>,
    terminations: Arc<Mutex<Vec<VoiceTermination>>>,
    live_workers: Arc<AtomicUsize>,
    input_frames: Arc<AtomicUsize>,
}
impl LocalAdapter {
    async fn emit(&self, index: usize, event: VoiceModelEvent) {
        let sender = self.senders.lock().unwrap()[index].clone();
        sender.send(event).await.unwrap();
    }
}
#[async_trait]
impl VoiceModelPort for LocalAdapter {
    fn describe(&self) -> VoiceAdapterDescriptor { descriptor() }
    async fn open(&self, request: VoiceOpenRequest, cancel: CancellationToken, deadline: Instant) -> Result<VoiceModelSession, VoiceError> {
        if Instant::now() >= deadline { return Err(VoiceError::new(VoiceErrorKind::Deadline, "test port open deadline")); }
        let negotiation = VoiceNegotiation { capabilities: capabilities(), input_spec: Some(spec()), output_spec: Some(spec()), native_attachment: None };
        let (io, mut worker_io) = VoiceModelSession::channels_with_specs(Some(spec()), Some(spec()),
            VoiceSessionLimits { close_timeout: Duration::from_millis(100), ..Default::default() }, cancel)?;
        self.opens.lock().unwrap().push(request);
        self.senders.lock().unwrap().push(worker_io.event_tx.clone());
        let live = self.live_workers.clone(); let controls = self.controls.clone();
        let inputs = self.input_frames.clone(); let terminations = self.terminations.clone();
        live.fetch_add(1, Ordering::SeqCst);
        let worker = tokio::spawn(async move {
            struct Joined(Arc<AtomicUsize>);
            impl Drop for Joined { fn drop(&mut self) { self.0.fetch_sub(1, Ordering::SeqCst); } }
            let _joined = Joined(live);
            loop { tokio::select! { biased;
                _ = worker_io.cancel.cancelled() => break,
                Some(control) = worker_io.urgent_rx.recv() => {
                    if let VoiceControl::SetInputMuted{muted,request_id}=&control{worker_io.event_tx.send(VoiceModelEvent::InputMuteApplied{muted:*muted,request_id:request_id.clone()}).await.unwrap();}
                    if matches!(control, VoiceControl::MuteInput { muted: true } | VoiceControl::InterruptOutput { .. }) { worker_io.media_rx.clear(); }
                    controls.lock().unwrap().push(control);
                },
                Some(control) = worker_io.control_rx.recv() => { controls.lock().unwrap().push(control); },
                Some(_) = worker_io.media_rx.recv() => { inputs.fetch_add(1, Ordering::SeqCst); },
            } }
            let termination = VoiceTermination { reason: worker_io.requested_close_reason(), finalization_confirmed: true, message: None };
            terminations.lock().unwrap().push(termination.clone()); worker_io.terminate(termination);
        });
        Ok(VoiceModelSession::from_parts(negotiation, io, worker))
    }
}

struct Authority { plan: VoiceBindingPlan, binding_valid: AtomicBool, credential_valid: AtomicBool }
#[async_trait]
impl VoiceAuthorityPort for Authority {
    async fn resolve_plan(&self, owner: &str, session: &str, binding: u64,_profile:&str,_revision:u64) -> Result<VoiceBindingPlan, VoiceError> {
        self.validate_binding(owner, session, binding).await?; Ok(self.plan.clone())
    }
    async fn validate_binding(&self, owner: &str, session: &str, binding: u64) -> Result<(), VoiceError> {
        if owner != OWNER { return Err(VoiceError::new(VoiceErrorKind::Authentication, "not the canonical owner")); }
        if session != SESSION || binding != 1 || !self.binding_valid.load(Ordering::SeqCst) {
            return Err(VoiceError::new(VoiceErrorKind::StaleBinding, "canonical binding changed"));
        }
        Ok(())
    }
    async fn validate_lease(&self, owner: &str, session: &str, binding: u64, record: &VoiceRouteRecord, revision: &str) -> Result<(), VoiceError> {
        self.validate_binding(owner, session, binding).await?;
        if !self.credential_valid.load(Ordering::SeqCst) || record != &self.plan.record || revision != self.plan.lease_revision {
            return Err(VoiceError::new(VoiceErrorKind::Authentication, "resolved credential lease revoked"));
        }
        Ok(())
    }
}
#[derive(Default)]
struct Work {
    recoveries:Mutex<Vec<(String,String)>>,
    requests: Mutex<Vec<(String, VoiceWorkRequest)>>,
    receipts: Mutex<BTreeMap<String, VoiceWorkReceipt>>,
    admission_gate: Mutex<Option<(Arc<Notify>, Arc<Notify>)>>,
    speech_source:Mutex<Option<(VoiceSpeechProjection,bool)>>,
    context_target:Mutex<Option<WorkTarget>>,
}
#[async_trait]
impl VoiceWorkPort for Work {
    async fn recover_pending_inputs(&self,owner:&str,session:&str)->Result<(),VoiceError>{self.recoveries.lock().unwrap().push((owner.into(),session.into()));Ok(())}
    async fn validate_source_ref(&self,_owner:&str,_session:&str,source:&VoiceSpeechSourceRef)->Result<bool,VoiceError>{Ok(self.speech_source.lock().unwrap().as_ref().is_some_and(|(speech,valid)|*valid&&speech.message_id==source.message_id&&speech.revision==source.revision&&speech.through_seq==source.through_seq))}
    async fn validate_initial_facts(&self,_owner:&str,_session:&str,_facts:&[VerifiedVoiceFact])->Result<bool,VoiceError>{Ok(true)}
    async fn answer_approval(&self, _: &str, _: &str, _: nomifun_voice_contracts::VoiceApprovalAnswer) -> Result<VoiceWorkReceipt, VoiceError> {
        Err(VoiceError::new(VoiceErrorKind::Unsupported, "no pending approval in this fixture"))
    }
    async fn context(&self, owner: &str, session: &str, binding: u64) -> Result<VoiceWorkContext, VoiceError> {
        assert_eq!((owner, session, binding), (OWNER, SESSION, 1));
        Ok(VoiceWorkContext { context_floor:0,context_reference:"canonical-context".into(),instructions: "Use the canonical report workspace".into(),
            initial_facts: vec![VerifiedVoiceFact{correlation_id:"prior".into(),upstream_trigger_id:None,canonical_receipt_id:"canonical-prior".into(),content:"The prior canonical conversation".into(),speak:false,output_generation:None,work_context:None,speech_source:self.speech_source.lock().unwrap().as_ref().map(|(speech,_)|VoiceSpeechSourceRef{message_id:speech.message_id.clone(),revision:speech.revision,through_seq:speech.through_seq})}],
            observed_target: Some(self.context_target.lock().unwrap().clone().unwrap_or_else(target)), facts: vec![receipt("original")], approvals: vec![] })
    }
    async fn interact(&self, owner: &str, session: &str, binding: u64, key: &str, request: VoiceWorkRequest) -> Result<VoiceWorkReceipt, VoiceError> {
        assert_eq!((owner, session, binding), (OWNER, SESSION, 1));
        self.requests.lock().unwrap().push((key.into(), request));
        let gate = self.admission_gate.lock().unwrap().clone();
        if let Some((entered, release)) = gate { entered.notify_one(); release.notified().await; }
        let accepted = receipt(key); self.receipts.lock().unwrap().insert(key.into(), accepted.clone()); Ok(accepted)
    }
    async fn lookup_operation(&self, _: &str, _: &str, key: &str) -> Result<Option<VoiceWorkReceipt>, VoiceError> {
        Ok(self.receipts.lock().unwrap().get(key).cloned())
    }
    async fn observe(&self, owner: &str, observed: &WorkTarget) -> Result<VoiceWorkReceipt, VoiceError> {
        assert_eq!(owner, OWNER);assert_eq!(observed.agent_session_id.as_ref(),SESSION);assert_eq!(observed.binding_version,1);
        let mut fact=receipt("original");fact.target=Some(observed.clone());Ok(fact)
    }
}
struct Harness { service: Arc<VoiceSessionService>, adapter: Arc<LocalAdapter>, authority: Arc<Authority>, work: Arc<Work>, dir: tempfile::TempDir }
impl Harness {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap(); let adapter = Arc::new(LocalAdapter::default());
        let factory_adapter = adapter.clone();
        let registry = Arc::new(VoiceAdapterRegistry::new(vec![VoiceAdapterRegistration { adapter_id: "test.lifecycle".into(),
            contract_version: 1, config_schema: serde_json::json!({"type":"object"}), catalog_projection: serde_json::json!({}),
            describe: Arc::new(|_| descriptor()), validate: Arc::new(|_| Ok(())), factory: Arc::new(move |_| {
                let port: Arc<dyn VoiceModelPort> = factory_adapter.clone(); Box::pin(async move { Ok(port) })
            }) }]).unwrap());
        let config = serde_json::json!({});
        let record = VoiceRouteRecord { schema: VOICE_ROUTE_SCHEMA.into(), route_id: "independent-voice-model".into(), revision: 1,
            provider_id: "local".into(), model: "offline".into(), model_revision: 1, connection_config_ref: "connection".into(),
            credential_ref: "credential-reference".into(), adapter_id: "test.lifecycle".into(), adapter_contract_version: 1,
            adapter_config_digest: nomifun_agent_contracts::digest_payload(&config).unwrap(), adapter_config: config,
            connection_config_digest: DigestHex("a".repeat(64)), required_features: native_duplex_requirements().into_iter().chain([VoiceFeature::TypedTools]).collect(),
            transport: VoiceTransportPreference::Relay };
        let plan = ResolvedVoicePlan { identity: record.identity().unwrap(), interaction: AgentVoiceInteraction {
            enabled: true, mode: VoiceInteractionMode::FullDuplex, route_key: "voice.primary".into(), work_policy: VoiceWorkPolicy::ExplicitIntent } };
        let authority = Arc::new(Authority { plan: VoiceBindingPlan { record, plan,profile_id:"profile".into(),profile_revision:1,work_steering_policy:Default::default(), lease_revision: "credential-v1".into() },
            binding_valid: AtomicBool::new(true), credential_valid: AtomicBool::new(true) });
        let work = Arc::new(Work::default());
        let service = Arc::new(VoiceSessionService::new(registry, work.clone(), authority.clone(), dir.path().into(), CancellationToken::new()));
        Self { service, adapter, authority, work, dir }
    }
    async fn activate(&self, endpoint: &str) -> VoiceActivationResponse { self.service.activate(OWNER, request(endpoint, false)).await.unwrap() }
    async fn state(&self, activation: &VoiceActivationResponse) -> VoiceState {
        self.service.state(OWNER, &activation.voice_session_id, activation.activation_epoch).await.unwrap()
    }
    fn journal_rows(&self, kind: &str) -> Vec<serde_json::Value> {
        let db = rusqlite::Connection::open_with_flags(self.dir.path().join("voice/voice.sqlite3"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let mut query = db.prepare("SELECT payload FROM voice_facts WHERE kind=?1 ORDER BY sequence").unwrap();
        query.query_map([kind], |row| row.get::<_, String>(0)).unwrap().map(|row| serde_json::from_str(&row.unwrap()).unwrap()).collect()
    }
}
async fn wait_until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !condition() { tokio::time::sleep(Duration::from_millis(5)).await; }
    }).await.expect("production owner did not reach the required lifecycle boundary");
}
async fn wait_closed(harness: &Harness, activation: &VoiceActivationResponse) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while harness.state(activation).await.connection != VoiceConnectionState::Closed { tokio::time::sleep(Duration::from_millis(5)).await; }
    }).await.unwrap();
}

#[tokio::test]
async fn explicit_authenticated_activation_calls_work_recovery_after_lazy_journal_open_but_off_and_probe_do_not(){
    let h=Harness::new();assert!(h.work.recoveries.lock().unwrap().is_empty());assert!(!h.dir.path().join("voice/voice.sqlite3").exists());
    let invalid=Arc::new(Access(AtomicBool::new(false)));assert!(h.service.activate_with_access(OWNER,request("denied",false),invalid).await.is_err());assert!(h.work.recoveries.lock().unwrap().is_empty());
    h.service.probe_profile(OWNER,"profile",nomifun_voice_contracts::VoiceProfileProbeRequest {agent_session_id:SESSION.into(),binding_version:1,profile_revision:1,native_offer:None},Arc::new(Access(AtomicBool::new(true)))).await.unwrap();
    assert!(h.work.recoveries.lock().unwrap().is_empty());assert!(!h.dir.path().join("voice/voice.sqlite3").exists());
    let _activation=h.service.activate_with_access(OWNER,request("recover-voice",false),Arc::new(Access(AtomicBool::new(true)))).await.unwrap();
    assert!(h.dir.path().join("voice/voice.sqlite3").exists());assert_eq!(*h.work.recoveries.lock().unwrap(),vec![(OWNER.into(),SESSION.into())]);assert!(h.work.requests.lock().unwrap().is_empty());h.service.shutdown().await;
}

#[tokio::test]
async fn activation_requires_one_shot_owner_attachment_before_work_or_media_admission() {
    let h = Harness::new(); let a = h.activate("desktop-a").await;
    assert_eq!(a.state.capture, VoiceCaptureState::Idle); assert!(a.state.work_running);
    assert_eq!(h.service.audio(OWNER, &a.voice_session_id, frame(a.activation_epoch, 1)).await.unwrap_err().kind, VoiceErrorKind::Closed);
    for muted in [true, false] {
        assert_eq!(h.service.control(OWNER, &a.voice_session_id, a.activation_epoch, VoiceControl::MuteInput { muted }).await.unwrap_err().kind,
            VoiceErrorKind::Closed, "mute/unmute must not fabricate an attached input lease");
    }
    assert_eq!(h.state(&a).await.capture, VoiceCaptureState::Idle);
    h.adapter.emit(0, start_trigger("unattached", None)).await;
    wait_until(|| h.adapter.controls.lock().unwrap().iter().any(|c| matches!(c, VoiceControl::RejectWorkTrigger { upstream_trigger_id, .. } if upstream_trigger_id == "unattached"))).await;
    assert!(h.work.requests.lock().unwrap().is_empty());
    assert_eq!(h.service.attach_media("other-owner", &a.voice_session_id, &a.attachment_token).await.err().unwrap().kind, VoiceErrorKind::Authentication);
    assert_eq!(h.service.attach_media(OWNER, &a.voice_session_id, "wrong-capability").await.err().unwrap().kind, VoiceErrorKind::Authentication);
    assert_eq!(h.service.state("other-owner", &a.voice_session_id, a.activation_epoch).await.unwrap_err().kind, VoiceErrorKind::Authentication);
    let _media = h.service.attach_media(OWNER, &a.voice_session_id, &a.attachment_token).await.unwrap();
    assert_eq!(h.service.attach_media(OWNER, &a.voice_session_id, &a.attachment_token).await.err().unwrap().kind, VoiceErrorKind::StaleEpoch);
    h.service.audio(OWNER, &a.voice_session_id, frame(a.activation_epoch, 2)).await.unwrap();
    wait_until(|| h.adapter.input_frames.load(Ordering::SeqCst) == 1).await;
    h.service.shutdown().await; assert_eq!(h.adapter.live_workers.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn explicit_takeover_revokes_old_capture_and_media_without_cancelling_canonical_work() {
    let h = Harness::new(); let first = h.activate("desktop-a").await;
    let media = h.service.attach_media(OWNER, &first.voice_session_id, &first.attachment_token).await.unwrap();
    assert_eq!(h.service.activate(OWNER, request("desktop-b", false)).await.unwrap_err().kind, VoiceErrorKind::StaleBinding);
    assert_eq!(h.adapter.opens.lock().unwrap().len(), 1);
    let second = h.service.activate(OWNER, request("desktop-b", true)).await.unwrap();
    assert!(second.activation_epoch > first.activation_epoch);
    assert_eq!(h.state(&first).await.connection, VoiceConnectionState::Closed);
    assert_ne!(h.state(&first).await.capture, VoiceCaptureState::Capturing);
    assert!(media.endpoint.control_snapshot().iter().any(|event| matches!(event,
        VoiceProductEvent::EndpointControl { control: VoiceControl::Close { .. }, .. })));
    assert_eq!(h.adapter.live_workers.load(Ordering::SeqCst), 1);
    assert!(h.adapter.terminations.lock().unwrap().iter().any(|t| t.reason == VoiceCloseReason::LeaseRevoked && t.finalization_confirmed));
    h.service.control(OWNER, &second.voice_session_id, second.activation_epoch, VoiceControl::Close { reason: VoiceCloseReason::UserEnded }).await.unwrap();
    assert!(h.work.requests.lock().unwrap().is_empty()); assert_eq!(h.adapter.live_workers.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn typed_work_context_receipts_are_durable_and_duplicate_triggers_do_not_resubmit() {
    let h = Harness::new(); let a = h.activate("desktop-a").await;
    let _media = h.service.attach_media(OWNER, &a.voice_session_id, &a.attachment_token).await.unwrap();
    {
        let opens = h.adapter.opens.lock().unwrap(); let open = &opens[0];
        assert!(!open.instructions.contains("The prior canonical conversation")); assert!(!open.instructions.contains("Canonical work remains active"));
        assert!(open.initial_facts.iter().any(|fact|fact.content.contains("The prior canonical conversation")));
        assert!(open.initial_facts.iter().any(|fact|fact.content.contains("Canonical work remains active")));
        assert_eq!(open.work_context.as_ref().unwrap().target, Some(target())); assert!(open.tools.iter().any(|tool| tool.name == "nomi_work"));
    }
    h.adapter.emit(0, VoiceModelEvent::Transcript { fragment: TranscriptFragment { speaker: VoiceSpeaker::User,
        fragment_id: "intent-1".into(), revision: 1, commit: TranscriptCommit::Committed,
        text: "Prepare the requested report".into(), media_range: Some(MediaRange { start_us: 0, end_us: 20_000 }) } }).await;
    h.adapter.emit(0, start_trigger("stable-intent", Some("intent-1"))).await;
    wait_until(|| h.adapter.controls.lock().unwrap().iter().any(|c| matches!(c, VoiceControl::InjectFact { fact } if fact.upstream_trigger_id.as_deref() == Some("stable-intent")))).await;
    h.adapter.emit(0, start_trigger("stable-intent", Some("intent-1"))).await;
    wait_until(|| h.adapter.controls.lock().unwrap().iter().filter(|c| matches!(c, VoiceControl::InjectFact { fact } if fact.upstream_trigger_id.as_deref() == Some("stable-intent"))).count() == 2).await;
    assert_eq!(h.work.requests.lock().unwrap().len(), 1);
    let projection = h.service.projection(OWNER, &a.voice_session_id, a.activation_epoch).await.unwrap();
    assert_eq!(projection.transcripts.len(), 1); assert!(projection.last_work_receipt.as_ref().unwrap().duplicate);
    h.service.shutdown().await;
    assert_eq!(h.journal_rows("work_receipt").len(), 1);
    assert!(h.journal_rows("voice_event").iter().any(|value| value["fragment"]["commit"] == "committed"));
    assert_eq!(h.adapter.live_workers.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn mute_and_interrupt_preserve_precise_played_facts_and_never_cancel_work() {
    let h = Harness::new(); let a = h.activate("desktop-a").await;
    let mut media = h.service.attach_media(OWNER, &a.voice_session_id, &a.attachment_token).await.unwrap();
    h.adapter.emit(0, VoiceModelEvent::OutputStarted { segment: OutputSegment { response_id: "answer".into(), segment_id: "spoken".into(),
        revision: 1, output_generation: 1, text: Some("Canonical work progress".into()), correlation_id: Some("original".into()) } }).await;
    h.adapter.emit(0, VoiceModelEvent::Audio { segment_id: "spoken".into(), frame: frame(a.activation_epoch, 1) }).await;
    let held = tokio::time::timeout(Duration::from_secs(1), media.frames.recv()).await.unwrap().unwrap(); assert!(held.is_current());
    let played = PlaybackReceipt { activation_epoch: a.activation_epoch, segment_id: "spoken".into(), revision: 1,
        output_generation: 1, state: DeliveryState::Played, consumed_us: 10_000, uncertain_tail_us: 0, precision: PlaybackPrecision::Exact };
    let (muted, interrupted) = tokio::join!(
        h.service.control(OWNER, &a.voice_session_id, a.activation_epoch, VoiceControl::MuteInput { muted: true }),
        h.service.control(OWNER, &a.voice_session_id, a.activation_epoch, VoiceControl::InterruptOutput { output_generation: 2, played: Some(played.clone()) }));
    muted.unwrap(); interrupted.unwrap(); assert!(!held.is_current());
    let state = h.state(&a).await; assert_eq!(state.capture, VoiceCaptureState::Paused); assert_eq!(state.output_generation, 2); assert!(state.work_running);
    assert_eq!(media.endpoint.latest_receipt(), Some(played.clone()));
    assert_eq!(media.endpoint.control_snapshot().len(), 2);
    assert_eq!(h.journal_rows("playback"), vec![serde_json::to_value(played).unwrap()]);
    h.service.audio(OWNER, &a.voice_session_id, frame(a.activation_epoch, 2)).await.unwrap(); assert_eq!(h.adapter.input_frames.load(Ordering::SeqCst), 0);
    let mut credit=h.service.get(OWNER,&a.voice_session_id,a.activation_epoch).await.unwrap().events.subscribe();
    h.service.audio(OWNER,&a.voice_session_id,frame(a.activation_epoch,3)).await.unwrap();
    assert!(matches!(credit.recv().await.unwrap(),VoiceProductEvent::InputReleased{sequence:3,duration_us:20_000,..}));
    assert!(h.work.requests.lock().unwrap().is_empty()); h.service.shutdown().await;
    assert_eq!(h.adapter.live_workers.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn canonical_binding_and_resolved_credential_changes_close_the_lease_and_join_workers() {
    for credential_changed in [false, true] {
        let h = Harness::new(); let a = h.activate("desktop-a").await;
        let mut media = h.service.attach_media(OWNER, &a.voice_session_id, &a.attachment_token).await.unwrap();
        if credential_changed { h.authority.credential_valid.store(false, Ordering::SeqCst); }
        else { h.authority.binding_valid.store(false, Ordering::SeqCst); }
        wait_closed(&h, &a).await;
        assert_ne!(h.state(&a).await.capture, VoiceCaptureState::Capturing);
        assert!(tokio::time::timeout(Duration::from_secs(1), media.frames.recv()).await.unwrap().is_none());
        assert!(h.work.requests.lock().unwrap().is_empty()); assert_eq!(h.adapter.live_workers.load(Ordering::SeqCst), 0);
        let expected = if credential_changed { VoiceCloseReason::PermissionRevoked } else { VoiceCloseReason::BindingChanged };
        assert!(h.adapter.terminations.lock().unwrap().iter().any(|t| t.reason == expected && t.finalization_confirmed));
        h.service.shutdown().await;
    }
}

#[tokio::test]
async fn late_measured_playback_after_close_is_durable_without_reviving_the_input_lease() {
    let h = Harness::new(); let a = h.activate("desktop-a").await;
    let mut media = h.service.attach_media(OWNER, &a.voice_session_id, &a.attachment_token).await.unwrap();
    h.adapter.emit(0, VoiceModelEvent::OutputStarted { segment: OutputSegment { response_id: "answer".into(), segment_id: "tail".into(),
        revision: 1, output_generation: 1, text: None, correlation_id: None } }).await;
    h.adapter.emit(0, VoiceModelEvent::Audio { segment_id: "tail".into(), frame: frame(a.activation_epoch, 1) }).await;
    let held = tokio::time::timeout(Duration::from_secs(1), media.frames.recv()).await.unwrap().unwrap();
    h.service.control(OWNER, &a.voice_session_id, a.activation_epoch, VoiceControl::Close { reason: VoiceCloseReason::UserEnded }).await.unwrap();
    assert!(!held.is_current()); assert_eq!(h.adapter.live_workers.load(Ordering::SeqCst), 0);
    let consumed = PlaybackReceipt { activation_epoch: a.activation_epoch, segment_id: "tail".into(), revision: 1,
        output_generation: 1, state: DeliveryState::Played, consumed_us: 9000, uncertain_tail_us: 1000, precision: PlaybackPrecision::Estimated };
    assert_eq!(h.service.control("other-owner", &a.voice_session_id, a.activation_epoch, VoiceControl::Playback { receipt: consumed.clone() }).await.unwrap_err().kind,
        VoiceErrorKind::Authentication);
    let state = h.service.control(OWNER, &a.voice_session_id, a.activation_epoch, VoiceControl::Playback { receipt: consumed.clone() }).await.unwrap();
    assert_eq!(state.connection, VoiceConnectionState::Closed); assert_ne!(state.capture, VoiceCaptureState::Capturing);
    assert_eq!(media.endpoint.latest_receipt(), Some(consumed.clone())); assert_eq!(h.journal_rows("playback"), vec![serde_json::to_value(consumed.clone()).unwrap()]);
    let mut fabricated = consumed.clone(); fabricated.consumed_us = 30_000;
    assert_eq!(h.service.control(OWNER, &a.voice_session_id, a.activation_epoch, VoiceControl::Playback { receipt: fabricated }).await.unwrap_err().kind,
        VoiceErrorKind::Configuration);
    let mut requeued = consumed; requeued.state = DeliveryState::Queued;
    assert_eq!(h.service.control(OWNER, &a.voice_session_id, a.activation_epoch, VoiceControl::Playback { receipt: requeued }).await.unwrap_err().kind,
        VoiceErrorKind::StaleEpoch);
    assert_eq!(h.service.control(OWNER, &a.voice_session_id, a.activation_epoch, VoiceControl::MuteInput { muted: false }).await.unwrap_err().kind, VoiceErrorKind::Closed);
    assert_eq!(h.service.audio(OWNER, &a.voice_session_id, frame(a.activation_epoch, 2)).await.unwrap_err().kind, VoiceErrorKind::Closed);
    assert!(h.work.requests.lock().unwrap().is_empty()); h.service.shutdown().await;
}

#[tokio::test]
async fn typed_media_intents_use_only_the_endpoint_and_cannot_mint_canonical_work() {
    let h = Harness::new(); let a = h.activate("desktop-a").await;
    let media = h.service.attach_media(OWNER, &a.voice_session_id, &a.attachment_token).await.unwrap();
    for (id, arguments) in [("spoken-pause", serde_json::json!({"action":"pause_speech"})),
        ("spoken-mute", serde_json::json!({"action":"mute_input","muted":true}))] {
        h.adapter.emit(0, VoiceModelEvent::WorkTrigger { trigger: WorkTrigger::TypedToolCall { upstream_trigger_id: id.into(),
            name: "nomi_work".into(), arguments, transcript_window_ref: None } }).await;
        wait_until(|| h.adapter.controls.lock().unwrap().iter().any(|control| matches!(control,
            VoiceControl::RejectWorkTrigger { upstream_trigger_id, reason } if upstream_trigger_id == id && reason.contains("No work submitted")))).await;
    }
    let state = h.state(&a).await; assert_eq!(state.output_generation, 2); assert_eq!(state.capture, VoiceCaptureState::Paused); assert!(state.work_running);
    assert_eq!(media.endpoint.control_snapshot().len(), 2); assert!(h.work.requests.lock().unwrap().is_empty());
    assert!(h.journal_rows("work_receipt").is_empty()); assert_eq!(h.journal_rows("media_control_requested").len(), 2);
    h.service.shutdown().await; assert_eq!(h.adapter.live_workers.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn spoken_stop_speaking_bypasses_a_canonical_admission_that_is_still_waiting() {
    let h = Harness::new(); let a = h.activate("desktop-a").await;
    let media = h.service.attach_media(OWNER, &a.voice_session_id, &a.attachment_token).await.unwrap();
    let entered = Arc::new(Notify::new()); let release = Arc::new(Notify::new());
    *h.work.admission_gate.lock().unwrap() = Some((entered.clone(), release.clone()));
    h.adapter.emit(0,VoiceModelEvent::Transcript{fragment:TranscriptFragment{speaker:VoiceSpeaker::User,fragment_id:"slow-input".into(),revision:1,commit:TranscriptCommit::Committed,text:"Prepare the requested report".into(),media_range:None}}).await;
    h.adapter.emit(0, start_trigger("slow-canonical-admission", Some("slow-input"))).await;
    tokio::time::timeout(Duration::from_secs(1), entered.notified()).await.unwrap();
    h.adapter.emit(0, VoiceModelEvent::WorkTrigger { trigger: WorkTrigger::TypedToolCall {
        upstream_trigger_id: "urgent-pause".into(), name: "nomi_work".into(),
        arguments: serde_json::json!({"action":"pause_speech"}), transcript_window_ref: None,
    } }).await;
    tokio::time::timeout(Duration::from_millis(100), async {
        loop {
            if media.endpoint.control_snapshot().iter().any(|event| matches!(event,
                VoiceProductEvent::EndpointControl { control: VoiceControl::InterruptOutput { output_generation: 2, .. }, .. })) { break; }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }).await.expect("spoken stop-speech waited behind canonical work admission");
    assert_eq!(h.state(&a).await.output_generation, 2); assert!(h.state(&a).await.work_running);
    assert_eq!(h.work.requests.lock().unwrap().len(), 1); assert!(h.work.receipts.lock().unwrap().is_empty());
    release.notify_one();
    wait_until(|| h.adapter.controls.lock().unwrap().iter().any(|control| matches!(control,
        VoiceControl::InjectFact { fact } if fact.upstream_trigger_id.as_deref() == Some("slow-canonical-admission")))).await;
    assert_eq!(h.work.requests.lock().unwrap().len(), 1); assert_eq!(h.work.receipts.lock().unwrap().len(), 1);
    h.service.shutdown().await; assert_eq!(h.adapter.live_workers.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn expired_unattached_activation_closes_its_model_and_never_admits_work() {
    let h = Harness::new(); let a = h.activate("lost-http-endpoint").await;
    assert_eq!(a.state.capture, VoiceCaptureState::Idle); assert_eq!(h.adapter.live_workers.load(Ordering::SeqCst), 1);
    // Journal initialization runs on the blocking pool; pause only after it has
    // completed so virtual deadlines cannot replace real storage completion.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    for _ in 0..10 { tokio::task::yield_now().await; }
    tokio::time::resume();
    wait_closed(&h, &a).await;
    assert_eq!(h.service.attach_media(OWNER, &a.voice_session_id, &a.attachment_token).await.err().unwrap().kind, VoiceErrorKind::Authentication);
    assert_eq!(h.adapter.live_workers.load(Ordering::SeqCst), 0); assert!(h.work.requests.lock().unwrap().is_empty());
    assert!(h.adapter.terminations.lock().unwrap().iter().any(|termination| termination.reason == VoiceCloseReason::LeaseRevoked && termination.finalization_confirmed));
    h.service.shutdown().await;
}

async fn require_source_context(h:&Harness,a:&VoiceActivationResponse,next_target:WorkTarget)->nomifun_voice_contracts::VoiceSourceContextRequirement{
    *h.work.context_target.lock().unwrap()=Some(next_target.clone());
    let active=h.service.get(OWNER,&a.voice_session_id,a.activation_epoch).await.unwrap();
    super::note_context_target(&active,Some(next_target.clone())).await.unwrap();
    h.adapter.emit(0,VoiceModelEvent::Transcript{fragment:TranscriptFragment{speaker:VoiceSpeaker::User,fragment_id:"uncertain-input".into(),revision:1,commit:TranscriptCommit::Committed,text:"取消当前正在运行的任务".into(),media_range:None}}).await;
    h.adapter.emit(0,VoiceModelEvent::WorkTrigger{trigger:WorkTrigger::TypedToolCall{upstream_trigger_id:"original-blocked-call".into(),name:"nomi_work".into(),arguments:serde_json::json!({"action":"cancel","target":next_target,"source_ref":{"fragment_id":"uncertain-input","revision":1}}),transcript_window_ref:None}}).await;
    tokio::time::timeout(Duration::from_secs(1),async{loop{
        if let Some(requirement)=h.service.projection(OWNER,&a.voice_session_id,a.activation_epoch).await.unwrap().source_context_requirement{return requirement;}
        tokio::time::sleep(Duration::from_millis(5)).await;
    }}).await.expect("unknown source timing must expose a typed human confirmation requirement")
}
#[tokio::test]
async fn authenticated_source_confirmation_reprepares_only_the_original_unreserved_request(){
    let h=Harness::new();let a=h.activate("source-confirmation").await;
    let mut media=h.service.attach_media(OWNER,&a.voice_session_id,&a.attachment_token).await.unwrap();
    let mut next=target();next.turn_operation_id="successor-B".into();
    let required=require_source_context(&h,&a,next.clone()).await;
    assert!(h.work.requests.lock().unwrap().is_empty());
    let db=rusqlite::Connection::open_with_flags(h.dir.path().join("voice/voice.sqlite3"),rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let reserved:i64=db.query_row("SELECT COUNT(*) FROM voice_work_links",[],|row|row.get(0)).unwrap();assert_eq!(reserved,0,"a blocked request must have no canonical admission reservation to replay");drop(db);
    let confirmation=nomifun_voice_contracts::VoiceSourceContextConfirmation{activation_epoch:a.activation_epoch,source_ref:required.source_ref.clone(),context_key:required.context.context_key.clone()};
    assert_eq!(h.service.confirm_source_context("different-owner",&a.voice_session_id,confirmation.clone()).await.unwrap_err().kind,VoiceErrorKind::Authentication);
    let mut stale=confirmation.clone();stale.context_key.push_str("changed");assert_eq!(h.service.confirm_source_context(OWNER,&a.voice_session_id,stale).await.unwrap_err().kind,VoiceErrorKind::StaleBinding);
    h.service.confirm_source_context(OWNER,&a.voice_session_id,confirmation.clone()).await.unwrap();
    wait_until(||h.work.requests.lock().unwrap().len()==1).await;
    assert!(matches!(&h.work.requests.lock().unwrap()[0].1,VoiceWorkRequest::Cancel{target} if target==&next));
    assert!(h.service.projection(OWNER,&a.voice_session_id,a.activation_epoch).await.unwrap().source_context_requirement.is_none());
    assert!(h.service.confirm_source_context(OWNER,&a.voice_session_id,confirmation).await.is_err());assert_eq!(h.work.requests.lock().unwrap().len(),1);
    let mut cleared=false;while let Ok(event)=media.events.try_recv(){cleared|=matches!(event,VoiceProductEvent::SourceContextChanged{requirement:None});}assert!(cleared);
    h.service.shutdown().await;
}
#[tokio::test]
async fn old_human_presentation_cannot_confirm_a_different_task_or_a_closed_voice(){
    let h=Harness::new();let a=h.activate("source-expiry").await;let _media=h.service.attach_media(OWNER,&a.voice_session_id,&a.attachment_token).await.unwrap();
    let mut next=target();next.turn_operation_id="successor-B".into();let required=require_source_context(&h,&a,next).await;
    let confirmation=nomifun_voice_contracts::VoiceSourceContextConfirmation{activation_epoch:a.activation_epoch,source_ref:required.source_ref,context_key:required.context.context_key};
    let mut later=target();later.turn_operation_id="successor-C".into();*h.work.context_target.lock().unwrap()=Some(later);
    assert_eq!(h.service.confirm_source_context(OWNER,&a.voice_session_id,confirmation.clone()).await.unwrap_err().kind,VoiceErrorKind::StaleBinding);
    assert!(h.work.requests.lock().unwrap().is_empty());
    h.service.control(OWNER,&a.voice_session_id,a.activation_epoch,VoiceControl::Close{reason:VoiceCloseReason::UserEnded}).await.unwrap();
    assert_eq!(h.service.confirm_source_context(OWNER,&a.voice_session_id,confirmation).await.unwrap_err().kind,VoiceErrorKind::Closed);
    assert!(h.work.requests.lock().unwrap().is_empty());h.service.shutdown().await;
}

#[tokio::test]
async fn metadata_delegation_receipt_exposes_the_actual_application_request_kind_without_vendor_arguments(){
    let h=Harness::new();let a=h.activate("neutral-metrics").await;let mut media=h.service.attach_media(OWNER,&a.voice_session_id,&a.attachment_token).await.unwrap();
    h.adapter.emit(0,VoiceModelEvent::ContextOpened{context_window_ref:"timeline".into(),work_context:Some(VoiceWorkContextFact{target:Some(target())})}).await;
    h.adapter.emit(0,VoiceModelEvent::Transcript{fragment:TranscriptFragment{speaker:VoiceSpeaker::User,fragment_id:"timeline:user:change".into(),revision:1,commit:TranscriptCommit::Committed,text:"改成周四".into(),media_range:Some(MediaRange{start_us:0,end_us:1_000_000})}}).await;
    h.adapter.emit(0,VoiceModelEvent::WorkTrigger{trigger:WorkTrigger::DelegationTrigger{upstream_trigger_id:"native-delegation".into(),target:"client".into(),offset_ms:1000,context_window_ref:Some("timeline".into())}}).await;
    let(receipt,request_kind)=tokio::time::timeout(Duration::from_secs(1),async{let mut received_trigger=false;loop{
        match media.events.recv().await.unwrap(){
            VoiceProductEvent::Model{event:VoiceModelEvent::WorkTrigger{trigger}} if trigger.upstream_trigger_id()=="native-delegation"=>received_trigger=true,
            VoiceProductEvent::WorkReceipt{receipt,request_kind,upstream_trigger_id:Some(id)} if id=="native-delegation"=>{assert!(received_trigger,"even a fast application receipt must follow its actual trigger in the product stream");break(receipt,request_kind);},
            _=>{},
        }
    }}).await.unwrap();
    assert_eq!(request_kind,Some(VoiceWorkRequestKind::Steer));assert_eq!(receipt.status,VoiceWorkStatus::Applied);
    assert!(matches!(&h.work.requests.lock().unwrap()[0].1,VoiceWorkRequest::Steer{target:actual,text} if actual==&target()&&text=="改成周四"));
    let active=h.service.get(OWNER,&a.voice_session_id,a.activation_epoch).await.unwrap();assert_eq!(active.request_kind(&receipt,None),Some(VoiceWorkRequestKind::Steer),"later exact-operation receipt updates retain only this submitted classification");
    let mut other=receipt;other.operation_key="unrelated-observation".into();assert_eq!(active.request_kind(&other,None),None,"no classification is guessed from another task or its summary");
    h.service.shutdown().await;
}
