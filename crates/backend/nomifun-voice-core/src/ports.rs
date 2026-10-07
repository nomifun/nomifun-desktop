use std::sync::{Arc, Mutex};
use std::time::Duration;
use async_trait::async_trait;
use tokio::sync::{mpsc, watch, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use nomifun_voice_contracts::voice::*;

#[derive(Clone, Debug)]
pub struct VoiceOpenRequest {
    pub voice_session_id: String,
    pub activation_epoch: u64,
    pub output_generation: u64,
    pub route_identity: VoiceRouteIdentity,
    pub model: String,
    pub instructions: String,
    /// Source-tagged canonical data, separate from immutable system instructions.
    pub initial_facts:Vec<VerifiedVoiceFact>,
    /// Voice-local provenance is deliberately separate from canonical facts.
    pub replay_scope:VoiceReplayScope,
    pub initial_local_replay:Option<VoiceLocalReplay>,
    pub tools: Vec<VoiceToolDefinition>,
    pub transport: VoiceTransportPreference,
    pub native_offer: Option<String>,
    pub work_context:Option<VoiceWorkContextFact>,
}

#[async_trait]
pub trait VoiceModelPort: Send + Sync {
    fn describe(&self) -> VoiceAdapterDescriptor;
    async fn open(&self, request: VoiceOpenRequest, cancel: CancellationToken, deadline: Instant)
        -> Result<VoiceModelSession, VoiceError>;
}

#[derive(Clone, Debug)]
pub struct VoiceEndpointLease {
    pub endpoint_id: String,
    pub voice_session_id: String,
    pub activation_epoch: u64,
    pub input_spec: Option<MediaSpec>,
    pub output_spec: Option<MediaSpec>,
    pub native_attachment: Option<VoiceNativeAttachment>,
}
#[async_trait]
pub trait VoiceEndpointPort: Send + Sync {
    async fn attach(&self, lease: VoiceEndpointLease, cancel: CancellationToken, deadline: Instant) -> Result<(), VoiceError>;
    async fn deliver(&self, segment: OutputSegment, frame: AudioFrame, deadline: Instant) -> Result<(), VoiceError>;
    async fn control(&self, control: VoiceControl, deadline: Instant) -> Result<Option<PlaybackReceipt>, VoiceError>;
    async fn shutdown(&self, deadline: Instant) -> Result<(), VoiceError>;
}

#[derive(Clone, Copy, Debug)]
pub struct VoiceSessionLimits {
    pub media_slots: usize,
    pub control_slots: usize,
    pub event_slots: usize,
    pub max_control_bytes: usize,
    pub write_timeout: Duration,
    pub close_timeout: Duration,
}
impl Default for VoiceSessionLimits {
    fn default() -> Self { Self { media_slots: 64, control_slots: 32, event_slots: 64,
        max_control_bytes: 256 * 1024, write_timeout: Duration::from_secs(3), close_timeout: Duration::from_secs(5) } }
}
impl VoiceSessionLimits {
    pub fn validate(&self) -> Result<(), VoiceError> {
        if self.media_slots == 0 || self.control_slots == 0 || self.event_slots == 0 || self.max_control_bytes == 0
            || self.write_timeout.is_zero() || self.close_timeout.is_zero() {
            return Err(VoiceError::new(VoiceErrorKind::Configuration, "invalid voice session limits"));
        }
        Ok(())
    }
}

struct QueuedAudio { frame: AudioFrame, admitted_at: Instant, _duration: OwnedSemaphorePermit }
pub struct VoiceMediaReceiver {
    rx: mpsc::Receiver<QueuedAudio>,
    max_age: Duration,
    cancel: CancellationToken,
    termination_tx: watch::Sender<Option<VoiceTermination>>,
    close_reason:Arc<Mutex<VoiceCloseReason>>,
}
impl VoiceMediaReceiver {
    pub async fn recv(&mut self) -> Option<AudioFrame> {
        let queued = tokio::select! { biased;
            _ = self.cancel.cancelled() => return None,
            queued = self.rx.recv() => queued?,
        };
        if queued.admitted_at.elapsed() > self.max_age {
            *self.close_reason.lock().unwrap_or_else(|p|p.into_inner())=VoiceCloseReason::Backlog;
            self.termination_tx.send_replace(Some(VoiceTermination { reason: VoiceCloseReason::Backlog,
                finalization_confirmed: false, message: Some("input media exceeded age budget".into()) }));
            self.cancel.cancel();
            return None;
        }
        Some(queued.frame)
    }
    pub fn clear(&mut self) { while self.rx.try_recv().is_ok() {} }
}

struct QueuedVoiceEvent{event:VoiceModelEvent,admitted_at:Instant,_duration:Option<OwnedSemaphorePermit>}
#[derive(Clone)]
pub struct VoiceEventSender{
    tx:mpsc::Sender<QueuedVoiceEvent>,output_spec:Option<MediaSpec>,duration:Arc<Semaphore>,cancel:CancellationToken,
    termination_tx:watch::Sender<Option<VoiceTermination>>,close_reason:Arc<Mutex<VoiceCloseReason>>,
}
impl VoiceEventSender{
    fn prepare(&self,event:VoiceModelEvent)->Result<QueuedVoiceEvent,VoiceError>{
        let duration=if let VoiceModelEvent::Audio{frame,..}=&event{
            let spec=self.output_spec.as_ref().ok_or_else(||VoiceError::new(VoiceErrorKind::Unsupported,"native attachment cannot emit relay audio"))?;
            frame.validate_for(spec).map_err(|e|VoiceError::new(VoiceErrorKind::Configuration,e))?;
            match self.duration.clone().try_acquire_many_owned(frame.duration_us.div_ceil(1000)){
                Ok(permit)=>Some(permit),Err(_)=>{self.fail_backlog();return Err(VoiceError::new(VoiceErrorKind::Backlog,"model audio event duration budget exceeded"));}
            }
        }else{None};
        Ok(QueuedVoiceEvent{event,admitted_at:Instant::now(),_duration:duration})
    }
    fn fail_backlog(&self){
        *self.close_reason.lock().unwrap_or_else(|p|p.into_inner())=VoiceCloseReason::Backlog;
        self.termination_tx.send_replace(Some(VoiceTermination{reason:VoiceCloseReason::Backlog,finalization_confirmed:false,message:Some("model event media backlog exceeded".into())}));self.cancel.cancel();
    }
    pub fn try_send(&self,event:VoiceModelEvent)->Result<(),VoiceError>{
        if self.cancel.is_cancelled(){return Err(VoiceError::new(VoiceErrorKind::Closed,"voice model closed"));}
        let event=self.prepare(event)?;
        self.tx.try_send(event).map_err(|error|{let full=matches!(error,mpsc::error::TrySendError::Full(_));if full{self.fail_backlog();}
            VoiceError::new(if full{VoiceErrorKind::Backlog}else{VoiceErrorKind::Closed},"model event queue unavailable")})
    }
    pub async fn send(&self,event:VoiceModelEvent)->Result<(),VoiceError>{
        let event=self.prepare(event)?;
        tokio::select!{biased;_=self.cancel.cancelled()=>Err(VoiceError::new(VoiceErrorKind::Closed,"voice model closed")),
            result=self.tx.send(event)=>result.map_err(|_|VoiceError::new(VoiceErrorKind::Closed,"model event channel closed"))}
    }
}
pub struct VoiceEventReceiver{rx:mpsc::Receiver<QueuedVoiceEvent>,max_age:Duration,cancel:CancellationToken,termination_tx:watch::Sender<Option<VoiceTermination>>,close_reason:Arc<Mutex<VoiceCloseReason>>}
impl VoiceEventReceiver{
    pub async fn recv(&mut self)->Option<VoiceModelEvent>{
        let event=tokio::select!{biased;_=self.cancel.cancelled()=>return None,event=self.rx.recv()=>event?};
        if matches!(&event.event,VoiceModelEvent::Audio{..})&&event.admitted_at.elapsed()>self.max_age{
            *self.close_reason.lock().unwrap_or_else(|p|p.into_inner())=VoiceCloseReason::Backlog;
            self.termination_tx.send_replace(Some(VoiceTermination{reason:VoiceCloseReason::Backlog,finalization_confirmed:false,message:Some("model audio event exceeded age budget".into())}));self.cancel.cancel();return None;
        }
        Some(event.event)
    }
}

#[derive(Clone)]
pub struct VoiceInputHandle {
    media_tx: mpsc::Sender<QueuedAudio>,
    control_tx: mpsc::Sender<VoiceControl>,
    urgent_tx:mpsc::Sender<VoiceControl>,
    duration: Arc<Semaphore>,
    input_spec: Option<MediaSpec>,
    cancel: CancellationToken,
    close_reason: Arc<Mutex<VoiceCloseReason>>,
    limits: VoiceSessionLimits,
}
impl VoiceInputHandle {
    /// Admission never waits behind audio; an exceeded duration budget fails closed.
    pub fn try_audio(&self, frame: AudioFrame) -> Result<(), VoiceError> {
        if self.cancel.is_cancelled() { return Err(VoiceError::new(VoiceErrorKind::Closed, "voice session closed")); }
        let spec = self.input_spec.as_ref().ok_or_else(|| VoiceError::new(VoiceErrorKind::Unsupported, "native attachment does not accept relay audio"))?;
        frame.validate_for(spec).map_err(|e| VoiceError::new(VoiceErrorKind::Configuration, e))?;
        let permit = self.duration.clone().try_acquire_many_owned(frame.duration_us.div_ceil(1000))
            .map_err(|_| VoiceError::new(VoiceErrorKind::Backlog, "input media exceeded duration budget"))?;
        self.media_tx.try_send(QueuedAudio { frame, admitted_at: Instant::now(), _duration: permit })
            .map_err(|e| VoiceError::new(if matches!(e, mpsc::error::TrySendError::Full(_)) { VoiceErrorKind::Backlog } else { VoiceErrorKind::Closed }, "input media queue unavailable"))
    }
    pub fn try_control(&self, control: VoiceControl) -> Result<(), VoiceError> {
        if let VoiceControl::Close { reason } = &control {
            *self.close_reason.lock().unwrap_or_else(|p| p.into_inner()) = *reason;
            self.cancel.cancel();
            // The cancellation lane cannot be blocked by either bounded queue.
            let _ = self.control_tx.try_send(control);
            return Ok(());
        }
        if self.cancel.is_cancelled() { return Err(VoiceError::new(VoiceErrorKind::Closed, "voice session closed")); }
        self.validate_control(&control)?;
        let sender=if matches!(&control,VoiceControl::InterruptOutput{..}|VoiceControl::MuteInput{..}|VoiceControl::SetInputMuted{..}|VoiceControl::RevokeSpeech{..}){&self.urgent_tx}else{&self.control_tx};
        sender.try_send(control).map_err(|e| VoiceError::new(
            if matches!(e, mpsc::error::TrySendError::Full(_)) { VoiceErrorKind::Backlog } else { VoiceErrorKind::Closed }, "voice control queue unavailable"))
    }
    pub async fn control(&self, control: VoiceControl, deadline: Instant) -> Result<(), VoiceError> {
        if matches!(control, VoiceControl::Close { .. }) { return self.try_control(control); }
        self.validate_control(&control)?;
        let sender=if matches!(&control,VoiceControl::InterruptOutput{..}|VoiceControl::MuteInput{..}|VoiceControl::SetInputMuted{..}|VoiceControl::RevokeSpeech{..}){&self.urgent_tx}else{&self.control_tx};
        tokio::select! { biased;
            _ = self.cancel.cancelled() => Err(VoiceError::new(VoiceErrorKind::Closed, "voice session closed")),
            result = tokio::time::timeout_at(deadline, sender.send(control)) => result
                .map_err(|_| VoiceError::new(VoiceErrorKind::Deadline, "voice control deadline"))?
                .map_err(|_| VoiceError::new(VoiceErrorKind::Closed, "voice session closed")),
        }
    }
    fn validate_control(&self, control: &VoiceControl) -> Result<(), VoiceError> {
        let bytes = match control {
            VoiceControl::SetInputMuted{request_id,..}=>{
                if request_id.trim().is_empty()||request_id.len()>256{return Err(VoiceError::new(VoiceErrorKind::Configuration,"invalid input control identity"));}request_id.len()
            },
            VoiceControl::PresentInputSource{fragment}=>{
                if fragment.speaker!=VoiceSpeaker::User||fragment.commit!=TranscriptCommit::Committed||fragment.revision==0||fragment.fragment_id.trim().is_empty()||fragment.fragment_id.len()>1024||fragment.text.len()>32*1024||fragment.media_range.as_ref().is_some_and(|range|range.end_us<range.start_us){return Err(VoiceError::new(VoiceErrorKind::Configuration,"invalid committed input source"));}
                fragment.text.len()+fragment.fragment_id.len()+64
            },
            VoiceControl::InjectFact { fact } => fact.content.len() + fact.correlation_id.len() + fact.canonical_receipt_id.len(),
            VoiceControl::RejectWorkTrigger{upstream_trigger_id,reason}=>upstream_trigger_id.len()+reason.len(),
            VoiceControl::UpdateConfiguration { patch } => (match &patch.instructions { VoicePatch::Set(v) => v.len(), _ => 0 })
                + match &patch.tools { VoicePatch::Set(v) => v.iter().map(|t| t.name.len() + t.description.len() + t.parameters.to_string().len()).sum(), _ => 0 },
            _ => 0,
        };
        if bytes > self.limits.max_control_bytes { return Err(VoiceError::new(VoiceErrorKind::Configuration, "voice control exceeds size limit")); }
        Ok(())
    }
    pub fn cancellation(&self) -> CancellationToken { self.cancel.clone() }
    pub fn requested_close_reason(&self) -> VoiceCloseReason { *self.close_reason.lock().unwrap_or_else(|p|p.into_inner()) }
}

pub struct VoiceSessionIo {
    pub input: VoiceInputHandle,
    pub event_rx: VoiceEventReceiver,
    pub termination_rx: watch::Receiver<Option<VoiceTermination>>,
    limits: VoiceSessionLimits,
}
pub struct VoiceWorkerIo {
    pub media_rx: VoiceMediaReceiver,
    pub control_rx: mpsc::Receiver<VoiceControl>,
    pub urgent_rx:mpsc::Receiver<VoiceControl>,
    pub event_tx: VoiceEventSender,
    pub termination_tx: watch::Sender<Option<VoiceTermination>>,
    pub cancel: CancellationToken,
    pub close_reason: Arc<Mutex<VoiceCloseReason>>,
}
impl VoiceWorkerIo {
    pub fn requested_close_reason(&self) -> VoiceCloseReason { *self.close_reason.lock().unwrap_or_else(|p| p.into_inner()) }
    pub fn terminate(&self, termination: VoiceTermination) { self.termination_tx.send_replace(Some(termination)); }
}

pub struct VoiceModelSession {
    pub negotiation: VoiceNegotiation,
    pub input: VoiceInputHandle,
    pub events: VoiceEventReceiver,
    pub termination: watch::Receiver<Option<VoiceTermination>>,
    lifecycle: VoiceLifecycle,
}
impl VoiceModelSession {
    /// Public construction seam used equally by built-in, local and external adapters.
    pub fn channels(input_spec: Option<MediaSpec>, limits: VoiceSessionLimits, cancel: CancellationToken)
        -> Result<(VoiceSessionIo, VoiceWorkerIo), VoiceError> {
        Self::channels_with_specs(input_spec.clone(),input_spec,limits,cancel)
    }
    pub fn channels_with_specs(input_spec:Option<MediaSpec>,output_spec:Option<MediaSpec>,limits:VoiceSessionLimits,cancel:CancellationToken)
        ->Result<(VoiceSessionIo,VoiceWorkerIo),VoiceError>{
        limits.validate()?;
        if let Some(spec) = &input_spec { spec.validate().map_err(|e| VoiceError::new(VoiceErrorKind::Configuration, e))?; }
        let (media_tx, media_rx) = mpsc::channel(limits.media_slots);
        let (control_tx, control_rx) = mpsc::channel(limits.control_slots);
        let (urgent_tx,urgent_rx)=mpsc::channel(8);
        let (event_tx, event_rx) = mpsc::channel(limits.event_slots);
        let (termination_tx, termination_rx) = watch::channel(None);
        let budget_ms = input_spec.as_ref().map_or(1, |s| s.max_buffer_duration_us.div_ceil(1000));
        let max_age = Duration::from_micros(input_spec.as_ref().map_or(1, |s| u64::from(s.max_frame_age_us)));
        let close_reason = Arc::new(Mutex::new(VoiceCloseReason::UserEnded));
        let input = VoiceInputHandle { media_tx, control_tx,urgent_tx, duration: Arc::new(Semaphore::new(budget_ms as usize)),
            input_spec, cancel: cancel.clone(), close_reason: close_reason.clone(), limits };
        if let Some(spec)=&output_spec{spec.validate().map_err(|e|VoiceError::new(VoiceErrorKind::Configuration,e))?;}
        let output_budget=output_spec.as_ref().map_or(1,|s|s.max_buffer_duration_us.div_ceil(1000));
        let output_age=Duration::from_micros(output_spec.as_ref().map_or(1,|s|u64::from(s.max_frame_age_us)));
        let event_tx=VoiceEventSender{tx:event_tx,output_spec,duration:Arc::new(Semaphore::new(output_budget as usize)),cancel:cancel.clone(),termination_tx:termination_tx.clone(),close_reason:close_reason.clone()};
        let event_rx=VoiceEventReceiver{rx:event_rx,max_age:output_age,cancel:cancel.clone(),termination_tx:termination_tx.clone(),close_reason:close_reason.clone()};
        Ok((VoiceSessionIo { input, event_rx, termination_rx, limits },
            VoiceWorkerIo { media_rx: VoiceMediaReceiver { rx: media_rx, max_age, cancel: cancel.clone(), termination_tx: termination_tx.clone(),close_reason:close_reason.clone() },
                control_rx,urgent_rx,event_tx, termination_tx, cancel, close_reason }))
    }
    pub fn from_parts(negotiation: VoiceNegotiation, io: VoiceSessionIo, worker: JoinHandle<()>) -> Self {
        let lifecycle = VoiceLifecycle { cancel: io.input.cancel.clone(), worker: Some(worker), close_timeout: io.limits.close_timeout };
        Self { negotiation, input: io.input, events: io.event_rx, termination: io.termination_rx, lifecycle }
    }
    pub async fn shutdown(mut self, reason: VoiceCloseReason) -> VoiceTermination {
        let _ = self.input.try_control(VoiceControl::Close { reason });
        let confirmed_join = self.lifecycle.join().await;
        self.termination.borrow().clone().unwrap_or(VoiceTermination { reason,
            finalization_confirmed: false, message: Some(if confirmed_join { "provider finalization unconfirmed" } else { "worker aborted after close deadline" }.into()) })
    }
}
struct VoiceLifecycle { cancel: CancellationToken, worker: Option<JoinHandle<()>>, close_timeout: Duration }
impl VoiceLifecycle {
    async fn join(&mut self) -> bool {
        let Some(mut worker) = self.worker.take() else { return true; };
        match tokio::time::timeout(self.close_timeout, &mut worker).await {
            Ok(result) => result.is_ok(),
            Err(_) => { worker.abort(); let _ = worker.await; false }
        }
    }
}
impl Drop for VoiceLifecycle {
    fn drop(&mut self) {
        self.cancel.cancel();
        let Some(mut worker) = self.worker.take() else { return; };
        let timeout = self.close_timeout;
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if tokio::time::timeout(timeout, &mut worker).await.is_err() { worker.abort(); let _ = worker.await; }
            });
        } else { worker.abort(); }
    }
}
