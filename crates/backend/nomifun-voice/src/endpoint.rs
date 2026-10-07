//! Product endpoint lease, bounded output and consumption facts. No work or wire owner.
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, Weak, atomic::{AtomicBool, AtomicU64, Ordering}};
use std::time::Duration;
use async_trait::async_trait;
use nomifun_voice_contracts::voice::*;
use nomifun_voice_core::{VoiceEndpointLease, VoiceEndpointPort};
use tokio::sync::{broadcast, mpsc, watch, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

type SegmentKey = (String, u64, u64);
type FrameBudget = Mutex<Option<OwnedSemaphorePermit>>;
struct EndpointFence { epoch: u64, generation: AtomicU64, closed: AtomicBool, cancel: CancellationToken }
pub struct VoiceOutputFrame {
    pub segment_id: String,
    pub frame: AudioFrame,
    admitted_at: Instant,
    max_age: Duration,
    _duration: Arc<FrameBudget>,
    fence: Arc<EndpointFence>,
}
impl VoiceOutputFrame {
    pub fn age(&self) -> Duration { self.admitted_at.elapsed() }
    /// A dequeued packet still needs this fence immediately before socket delivery.
    pub fn is_current(&self) -> bool {
        !self.fence.closed.load(Ordering::Acquire) && !self.fence.cancel.is_cancelled()
            && self.frame.activation_epoch == self.fence.epoch
            && self.frame.output_generation >= self.fence.generation.load(Ordering::Acquire)
            && self.age() <= self.max_age
    }
}
struct SegmentProjection { segment: OutputSegment, generated_us: u64, receipt: Option<PlaybackReceipt> }
struct QueuedBudget { generation: u64, admitted_at: Instant, duration: Weak<FrameBudget> }
struct EndpointProjection {
    segments: BTreeMap<SegmentKey, SegmentProjection>,
    order: VecDeque<SegmentKey>,
    queued: Vec<QueuedBudget>,
}
#[derive(Default)]
struct EndpointControls { interrupt:Option<VoiceControl>, mute:Option<VoiceControl>, close:Option<VoiceControl> }
pub struct ProductVoiceEndpoint {
    lease: VoiceEndpointLease,
    frames: Mutex<Option<mpsc::Sender<VoiceOutputFrame>>>,
    duration: Arc<Semaphore>,
    events: broadcast::Sender<VoiceProductEvent>,
    urgent: watch::Sender<Option<VoiceProductEvent>>,
    fence: Arc<EndpointFence>,
    projection: Mutex<EndpointProjection>,
    weak_self: Weak<ProductVoiceEndpoint>,
    watcher: Mutex<Option<JoinHandle<()>>>,
    controls: Mutex<EndpointControls>,
}
impl ProductVoiceEndpoint {
    pub fn new(lease: VoiceEndpointLease, events: broadcast::Sender<VoiceProductEvent>, cancel: CancellationToken)
        -> Result<(Arc<Self>, mpsc::Receiver<VoiceOutputFrame>), VoiceError> {
        validate_lease(&lease)?;
        let budget = lease.output_spec.as_ref().map_or(1, |spec| spec.max_buffer_duration_us);
        let slots = lease.output_spec.as_ref().map_or(1, |spec|
            spec.max_buffer_duration_us.div_ceil(spec.min_frame_duration_us).clamp(1, 256)) as usize;
        let (frames, receiver) = mpsc::channel(slots);
        let (urgent, _) = watch::channel(None);
        let fence = Arc::new(EndpointFence { epoch: lease.activation_epoch, generation: AtomicU64::new(1),
            closed: AtomicBool::new(false), cancel: cancel.child_token() });
        Ok((Arc::new_cyclic(|weak| Self { lease, frames:Mutex::new(Some(frames)), duration: Arc::new(Semaphore::new(budget as usize)), events, urgent, fence,
            projection: Mutex::new(EndpointProjection { segments: BTreeMap::new(), order: VecDeque::new(), queued: Vec::new() }),
            weak_self:weak.clone(),watcher:Mutex::new(None),controls:Mutex::new(EndpointControls::default()) }), receiver))
    }
    /// Dedicated coalescing control lane; media writers select it before backlog.
    pub fn subscribe_controls(&self) -> watch::Receiver<Option<VoiceProductEvent>> { self.urgent.subscribe() }
    pub fn control_snapshot(&self)->Vec<VoiceProductEvent>{
        let controls=self.controls.lock().unwrap_or_else(|poison|poison.into_inner());
        let current:Vec<VoiceControl>=if let Some(close)=&controls.close{vec![close.clone()]}else{[controls.interrupt.clone(),controls.mute.clone()].into_iter().flatten().collect()};
        current.into_iter().map(|control|VoiceProductEvent::EndpointControl{activation_epoch:self.lease.activation_epoch,control}).collect()
    }
    pub fn lease(&self) -> &VoiceEndpointLease { &self.lease }
    pub fn register_segment(&self, segment: OutputSegment) -> Result<(), VoiceError> {
        self.require_live()?;
        if segment.segment_id.trim().is_empty() || segment.segment_id.len() > 1024 || segment.revision == 0
            || segment.output_generation != self.fence.generation.load(Ordering::Acquire) {
            return Err(VoiceError::new(VoiceErrorKind::StaleEpoch, "invalid or stale endpoint output segment"));
        }
        let key = segment_key(&segment);
        let mut projection = self.projection.lock().unwrap_or_else(|poison| poison.into_inner());
        if let Some(existing) = projection.segments.get(&key) {
            if existing.segment != segment { return Err(VoiceError::new(VoiceErrorKind::Configuration, "endpoint segment identity was reused for different facts")); }
            return Ok(());
        }
        projection.order.push_back(key.clone());
        projection.segments.insert(key, SegmentProjection { segment, generated_us: 0, receipt: None });
        while projection.order.len() > 128 {
            if let Some(old) = projection.order.pop_front() { projection.segments.remove(&old); }
        }
        Ok(())
    }
    pub fn record_receipt(&self, receipt: PlaybackReceipt) -> Result<Option<PlaybackReceipt>, VoiceError> {
        if self.fence.closed.load(Ordering::Acquire)&&receipt.state==DeliveryState::Queued{
            return Err(VoiceError::new(VoiceErrorKind::StaleEpoch,"closed endpoint cannot queue output"));
        }
        if receipt.activation_epoch != self.lease.activation_epoch {
            return Err(VoiceError::new(VoiceErrorKind::StaleEpoch, "playback receipt belongs to another activation"));
        }
        let key = (receipt.segment_id.clone(), receipt.revision, receipt.output_generation);
        let mut projection = self.projection.lock().unwrap_or_else(|poison| poison.into_inner());
        let segment = projection.segments.get_mut(&key).ok_or_else(|| VoiceError::new(VoiceErrorKind::Configuration, "playback has no exact endpoint segment"))?;
        if self.lease.native_attachment.is_some() && receipt.precision == PlaybackPrecision::Exact {
            return Err(VoiceError::new(VoiceErrorKind::Unsupported, "native attachment has no exact per-segment consumption clock"));
        }
        if self.lease.native_attachment.is_none() && receipt.consumed_us > segment.generated_us.saturating_add(1) {
            return Err(VoiceError::new(VoiceErrorKind::Configuration, "playback consumption exceeds generated audio"));
        }
        if receipt.output_generation < self.fence.generation.load(Ordering::Acquire) && receipt.state == DeliveryState::Queued {
            return Err(VoiceError::new(VoiceErrorKind::StaleEpoch, "revoked output cannot be queued again"));
        }
        if segment.receipt.as_ref().is_some_and(|previous| receipt.consumed_us < previous.consumed_us) {
            return Err(VoiceError::new(VoiceErrorKind::Configuration, "endpoint consumption cursor cannot regress"));
        }
        let previous = segment.receipt.replace(receipt);
        Ok(previous)
    }
    pub fn latest_receipt(&self) -> Option<PlaybackReceipt> {
        let projection = self.projection.lock().unwrap_or_else(|poison| poison.into_inner());
        projection.order.iter().rev().find_map(|key| projection.segments.get(key).and_then(|segment| segment.receipt.clone()))
    }
    fn require_live(&self) -> Result<(), VoiceError> {
        if self.fence.closed.load(Ordering::Acquire) || self.fence.cancel.is_cancelled() {
            Err(VoiceError::new(VoiceErrorKind::Closed, "product endpoint lease is closed"))
        } else { Ok(()) }
    }
    fn emit_control(&self, control: VoiceControl) {
        {
            let mut controls=self.controls.lock().unwrap_or_else(|poison|poison.into_inner());
            match &control {
                VoiceControl::InterruptOutput{..}=>controls.interrupt=Some(control.clone()),
                VoiceControl::MuteInput{..}=>controls.mute=Some(control.clone()),
                VoiceControl::Close{..}=>controls.close=Some(control.clone()),
                _=>{}
            }
        }
        let event = VoiceProductEvent::EndpointControl { activation_epoch: self.lease.activation_epoch, control };
        self.urgent.send_replace(Some(event.clone()));
        let _ = self.events.send(event);
    }
    fn release_backlog(&self, generation: Option<u64>) {
        let mut projection = self.projection.lock().unwrap_or_else(|poison| poison.into_inner());
        projection.queued.retain(|queued| {
            let Some(budget) = queued.duration.upgrade() else { return false; };
            if generation.is_none_or(|current| queued.generation < current) {
                budget.lock().unwrap_or_else(|poison| poison.into_inner()).take();
                false
            } else { true }
        });
    }
    fn close(&self, reason: VoiceCloseReason) {
        if self.fence.closed.swap(true, Ordering::AcqRel) { return; }
        self.fence.cancel.cancel();
        self.frames.lock().unwrap_or_else(|poison|poison.into_inner()).take();
        self.release_backlog(None);
        self.emit_control(VoiceControl::Close { reason });
    }
    fn backlog(&self, message: &'static str) -> VoiceError {
        self.close(VoiceCloseReason::Backlog);
        VoiceError::new(VoiceErrorKind::Backlog, message)
    }
}
impl Drop for ProductVoiceEndpoint {
    fn drop(&mut self) {
        self.close(VoiceCloseReason::AppShutdown);
        if let Some(mut watcher)=self.watcher.lock().unwrap_or_else(|poison|poison.into_inner()).take(){
            if let Ok(runtime)=tokio::runtime::Handle::try_current(){runtime.spawn(async move{
                if tokio::time::timeout(Duration::from_secs(1),&mut watcher).await.is_err(){watcher.abort();let _=watcher.await;}
            });}else{watcher.abort();}
        }
    }
}

#[async_trait]
impl VoiceEndpointPort for ProductVoiceEndpoint {
    async fn attach(&self, lease: VoiceEndpointLease, cancel: CancellationToken, deadline: Instant) -> Result<(), VoiceError> {
        if deadline <= Instant::now() { return Err(VoiceError::new(VoiceErrorKind::Deadline, "endpoint attach deadline")); }
        validate_lease(&lease)?;
        if cancel.is_cancelled() { return Err(VoiceError::new(VoiceErrorKind::Closed, "endpoint activation cancelled")); }
        if lease.endpoint_id != self.lease.endpoint_id || lease.voice_session_id != self.lease.voice_session_id
            || lease.activation_epoch != self.lease.activation_epoch || lease.input_spec != self.lease.input_spec
            || lease.output_spec != self.lease.output_spec || lease.native_attachment != self.lease.native_attachment {
            return Err(VoiceError::new(VoiceErrorKind::StaleEpoch, "endpoint attach differs from the exact lease"));
        }
        self.require_live()?;
        let mut watcher=self.watcher.lock().unwrap_or_else(|poison|poison.into_inner());
        if watcher.is_none(){
            let endpoint=self.weak_self.clone();let endpoint_cancel=self.fence.cancel.clone();
            *watcher=Some(tokio::spawn(async move{
                tokio::select!{biased;_=endpoint_cancel.cancelled()=>{},_=cancel.cancelled()=>{}}
                if let Some(endpoint)=endpoint.upgrade(){endpoint.close(VoiceCloseReason::LeaseRevoked);}
            }));
        }
        Ok(())
    }
    async fn deliver(&self, segment: OutputSegment, frame: AudioFrame, deadline: Instant) -> Result<(), VoiceError> {
        self.require_live()?;
        if deadline <= Instant::now() { return Err(VoiceError::new(VoiceErrorKind::Deadline, "endpoint output deadline")); }
        let spec = self.lease.output_spec.as_ref().ok_or_else(|| VoiceError::new(VoiceErrorKind::Unsupported, "native media does not accept relay frames"))?;
        frame.validate_for(spec).map_err(|message| VoiceError::new(VoiceErrorKind::Configuration, message))?;
        if frame.activation_epoch != self.lease.activation_epoch || frame.output_generation != segment.output_generation {
            return Err(VoiceError::new(VoiceErrorKind::StaleEpoch, "endpoint frame differs from the exact segment activation"));
        }
        self.register_segment(segment.clone())?;
        let max_age = Duration::from_micros(u64::from(spec.max_frame_age_us));
        {
            let mut projection = self.projection.lock().unwrap_or_else(|poison| poison.into_inner());
            projection.queued.retain(|queued| queued.duration.strong_count() > 0);
            if projection.queued.iter().any(|queued| queued.admitted_at.elapsed() > max_age) {
                drop(projection); return Err(self.backlog("endpoint output exceeded negotiated age budget"));
            }
        }
        let permit = match self.duration.clone().try_acquire_many_owned(frame.duration_us) {
            Ok(permit) => permit, Err(_) => return Err(self.backlog("endpoint output exceeded negotiated duration budget")),
        };
        let admitted_at = Instant::now();
        let duration = Arc::new(Mutex::new(Some(permit)));
        {
            let mut projection = self.projection.lock().unwrap_or_else(|poison| poison.into_inner());
            projection.queued.push(QueuedBudget { generation: frame.output_generation, admitted_at, duration: Arc::downgrade(&duration) });
            if let Some(segment) = projection.segments.get_mut(&segment_key(&segment)) {
                segment.generated_us = segment.generated_us.saturating_add(u64::from(frame.duration_us));
            }
        }
        let packet = VoiceOutputFrame { segment_id: segment.segment_id, frame, admitted_at, max_age, _duration: duration, fence: self.fence.clone() };
        let sender=self.frames.lock().unwrap_or_else(|poison|poison.into_inner()).as_ref().cloned()
            .ok_or_else(||VoiceError::new(VoiceErrorKind::Closed,"endpoint media lease closed"))?;
        sender.try_send(packet).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => self.backlog("endpoint media queue is full"),
            mpsc::error::TrySendError::Closed(_) => { self.close(VoiceCloseReason::NetworkLost); VoiceError::new(VoiceErrorKind::Closed, "endpoint media receiver closed") },
        })
    }
    async fn control(&self, control: VoiceControl, deadline: Instant) -> Result<Option<PlaybackReceipt>, VoiceError> {
        if let VoiceControl::Close { reason } = control { self.close(reason); return Ok(self.latest_receipt()); }
        self.require_live()?;
        if deadline <= Instant::now() { return Err(VoiceError::new(VoiceErrorKind::Deadline, "endpoint control deadline")); }
        match &control {
            VoiceControl::InterruptOutput { output_generation, played } => {
                if *output_generation < self.fence.generation.load(Ordering::Acquire) {
                    return Err(VoiceError::new(VoiceErrorKind::StaleEpoch, "stale endpoint output interruption"));
                }
                if let Some(receipt) = played { self.record_receipt(receipt.clone())?; }
                self.fence.generation.fetch_max(*output_generation, Ordering::AcqRel);
                self.release_backlog(Some(*output_generation));
            }
            VoiceControl::Playback { receipt } => { self.record_receipt(receipt.clone())?; return Ok(self.latest_receipt()); }
            VoiceControl::MuteInput { .. } => {}
            _ => return Err(VoiceError::new(VoiceErrorKind::Unsupported, "control is owned by the model or canonical application")),
        }
        self.emit_control(control);
        Ok(self.latest_receipt())
    }
    async fn shutdown(&self, deadline: Instant) -> Result<(), VoiceError> {
        self.close(VoiceCloseReason::AppShutdown);
        let watcher=self.watcher.lock().unwrap_or_else(|poison|poison.into_inner()).take();
        if let Some(mut watcher)=watcher {
            if tokio::time::timeout_at(deadline,&mut watcher).await.is_err(){watcher.abort();let _=watcher.await;
                return Err(VoiceError::new(VoiceErrorKind::Deadline,"endpoint shutdown finalization deadline"));}
        }
        if deadline <= Instant::now() { Err(VoiceError::new(VoiceErrorKind::Deadline, "endpoint shutdown finalization deadline")) } else { Ok(()) }
    }
}
fn segment_key(segment: &OutputSegment) -> SegmentKey { (segment.segment_id.clone(), segment.revision, segment.output_generation) }
fn validate_lease(lease: &VoiceEndpointLease) -> Result<(), VoiceError> {
    if lease.endpoint_id.trim().is_empty() || lease.voice_session_id.trim().is_empty() || lease.activation_epoch == 0 {
        return Err(VoiceError::new(VoiceErrorKind::Configuration, "invalid product endpoint lease"));
    }
    if lease.native_attachment.is_some() && (lease.input_spec.is_some() || lease.output_spec.is_some()) {
        return Err(VoiceError::new(VoiceErrorKind::Configuration, "native endpoint cannot duplicate relay media"));
    }
    for spec in [&lease.input_spec, &lease.output_spec].into_iter().flatten() {
        spec.validate().map_err(|message| VoiceError::new(VoiceErrorKind::Configuration, message))?;
    }
    if lease.native_attachment.is_none() && (lease.input_spec.is_none() || lease.output_spec.is_none()) {
        return Err(VoiceError::new(VoiceErrorKind::Configuration, "relay endpoint needs negotiated input and output specifications"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> MediaSpec { MediaSpec { format: AudioFormat::pcm16(16_000, 1), timebase:16_000,
        min_frame_duration_us:10_000,max_frame_duration_us:20_000,max_frame_bytes:640,max_buffer_duration_us:40_000,max_frame_age_us:100_000 } }
    fn lease() -> VoiceEndpointLease { VoiceEndpointLease { endpoint_id:"desktop".into(),voice_session_id:"voice".into(),activation_epoch:7,
        input_spec:Some(spec()),output_spec:Some(spec()),native_attachment:None } }
    fn segment(generation:u64)->OutputSegment{OutputSegment{response_id:"response".into(),segment_id:"segment".into(),revision:1,output_generation:generation,text:None,correlation_id:None}}
    fn frame(generation:u64,sequence:u64)->AudioFrame{AudioFrame{activation_epoch:7,output_generation:generation,sequence,timestamp:sequence*320,duration_us:20_000,format:spec().format,payload:vec![0;640]}}
    fn make_endpoint()->(Arc<ProductVoiceEndpoint>,mpsc::Receiver<VoiceOutputFrame>){let (events,_)=broadcast::channel(8);ProductVoiceEndpoint::new(lease(),events,CancellationToken::new()).unwrap()}
    #[tokio::test]
    async fn interrupt_is_independent_of_slow_media_and_fences_even_dequeued_frames(){
        let (endpoint,mut receiver)=make_endpoint();let mut urgent=endpoint.subscribe_controls();
        let deadline=Instant::now()+Duration::from_secs(1);
        endpoint.deliver(segment(1),frame(1,1),deadline).await.unwrap();
        endpoint.deliver(segment(1),frame(1,2),deadline).await.unwrap();
        let held=receiver.recv().await.unwrap();assert!(held.is_current());
        endpoint.control(VoiceControl::InterruptOutput{output_generation:2,played:None},deadline).await.unwrap();
        urgent.changed().await.unwrap();assert!(matches!(&*urgent.borrow(),Some(VoiceProductEvent::EndpointControl{control:VoiceControl::InterruptOutput{output_generation:2,..},..})));
        assert!(!held.is_current());
        endpoint.deliver(segment(2),frame(2,3),deadline).await.unwrap();
        assert!(!receiver.recv().await.unwrap().is_current());assert!(receiver.recv().await.unwrap().is_current());
    }
    #[tokio::test]
    async fn duration_backlog_closes_only_endpoint_and_close_bypasses_full_queue(){
        let parent=CancellationToken::new();let (events,_)=broadcast::channel(8);
        let (endpoint,_receiver)=ProductVoiceEndpoint::new(lease(),events,parent.clone()).unwrap();let mut controls=endpoint.subscribe_controls();
        let deadline=Instant::now()+Duration::from_secs(1);
        endpoint.deliver(segment(1),frame(1,1),deadline).await.unwrap();endpoint.deliver(segment(1),frame(1,2),deadline).await.unwrap();
        assert_eq!(endpoint.deliver(segment(1),frame(1,3),deadline).await.unwrap_err().kind,VoiceErrorKind::Backlog);
        controls.changed().await.unwrap();assert!(matches!(&*controls.borrow(),Some(VoiceProductEvent::EndpointControl{control:VoiceControl::Close{reason:VoiceCloseReason::Backlog},..})));
        assert!(!parent.is_cancelled());endpoint.shutdown(deadline).await.unwrap();
    }
    #[tokio::test]
    async fn exact_lease_and_receipt_identity_are_checked_and_consumption_is_monotonic(){
        let (endpoint,_receiver)=make_endpoint();let deadline=Instant::now()+Duration::from_secs(1);
        let mut other=lease();other.activation_epoch=8;assert_eq!(endpoint.attach(other,CancellationToken::new(),deadline).await.unwrap_err().kind,VoiceErrorKind::StaleEpoch);
        endpoint.deliver(segment(1),frame(1,1),deadline).await.unwrap();
        let receipt=PlaybackReceipt{activation_epoch:7,segment_id:"segment".into(),revision:1,output_generation:1,state:DeliveryState::Played,consumed_us:10_000,uncertain_tail_us:1000,precision:PlaybackPrecision::Estimated};
        endpoint.record_receipt(receipt.clone()).unwrap();let mut late=receipt.clone();late.consumed_us=9000;assert!(endpoint.record_receipt(late).is_err());
        endpoint.control(VoiceControl::InterruptOutput{output_generation:2,played:Some(receipt.clone())},deadline).await.unwrap();assert_eq!(endpoint.latest_receipt(),Some(receipt));
    }
    #[tokio::test]
    async fn native_has_no_relay_path_or_claimed_exact_consumption(){
        let mut native=lease();native.input_spec=None;native.output_spec=None;native.native_attachment=Some(VoiceNativeAttachment{attachment_id:"direct".into(),answer_sdp:"opaque".into(),safe_output_boundary:true});
        let (events,_)=broadcast::channel(8);let (endpoint,_)=ProductVoiceEndpoint::new(native,events,CancellationToken::new()).unwrap();
        endpoint.register_segment(segment(1)).unwrap();assert_eq!(endpoint.deliver(segment(1),frame(1,1),Instant::now()+Duration::from_secs(1)).await.unwrap_err().kind,VoiceErrorKind::Unsupported);
        assert_eq!(endpoint.record_receipt(PlaybackReceipt{activation_epoch:7,segment_id:"segment".into(),revision:1,output_generation:1,state:DeliveryState::Played,consumed_us:1,uncertain_tail_us:0,precision:PlaybackPrecision::Exact}).unwrap_err().kind,VoiceErrorKind::Unsupported);
    }
    #[tokio::test]
    async fn coalesced_controls_keep_interruption_when_mute_arrives_before_the_writer(){
        let (endpoint,_)=make_endpoint();let deadline=Instant::now()+Duration::from_secs(1);
        endpoint.control(VoiceControl::InterruptOutput{output_generation:2,played:None},deadline).await.unwrap();
        endpoint.control(VoiceControl::MuteInput{muted:true},deadline).await.unwrap();
        let current=endpoint.control_snapshot();assert_eq!(current.len(),2);
        assert!(matches!(&current[0],VoiceProductEvent::EndpointControl{control:VoiceControl::InterruptOutput{output_generation:2,..},..}));
        endpoint.control(VoiceControl::Close{reason:VoiceCloseReason::LeaseRevoked},deadline).await.unwrap();
        assert_eq!(endpoint.control_snapshot().len(),1);
    }
    #[tokio::test]
    async fn attached_owner_cancellation_revokes_media_and_shutdown_joins_its_listener(){
        let (endpoint,mut receiver)=make_endpoint();let cancel=CancellationToken::new();let mut controls=endpoint.subscribe_controls();
        let deadline=Instant::now()+Duration::from_secs(1);
        endpoint.attach(lease(),cancel.clone(),deadline).await.unwrap();cancel.cancel();
        tokio::time::timeout(Duration::from_millis(100),controls.changed()).await.unwrap().unwrap();
        assert!(matches!(&*controls.borrow(),Some(VoiceProductEvent::EndpointControl{control:VoiceControl::Close{reason:VoiceCloseReason::LeaseRevoked},..})));
        assert!(receiver.recv().await.is_none());endpoint.shutdown(deadline).await.unwrap();
    }
    #[tokio::test]
    async fn queued_output_age_and_deadlines_are_real_admission_limits(){
        let mut aged=lease();aged.output_spec.as_mut().unwrap().max_frame_age_us=1000;
        let (events,_)=broadcast::channel(8);let (endpoint,mut receiver)=ProductVoiceEndpoint::new(aged,events,CancellationToken::new()).unwrap();
        endpoint.deliver(segment(1),frame(1,1),Instant::now()+Duration::from_secs(1)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;assert!(!receiver.recv().await.unwrap().is_current());
        let (endpoint,_)=make_endpoint();assert_eq!(endpoint.deliver(segment(1),frame(1,1),Instant::now()).await.unwrap_err().kind,VoiceErrorKind::Deadline);
    }
}
