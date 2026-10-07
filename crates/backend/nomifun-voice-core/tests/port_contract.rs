//! A consumer crate uses only the published API; no provider/network implementation is present.
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use std::time::Duration;
use async_trait::async_trait;
use nomifun_voice_contracts::DigestHex;
use nomifun_voice_core::*;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

fn spec() -> MediaSpec { MediaSpec { format: AudioFormat::pcm16(16_000, 1), timebase: 1_000_000,
    min_frame_duration_us: 10_000, max_frame_duration_us: 40_000, max_frame_bytes: 1280,
    max_buffer_duration_us: 60_000, max_frame_age_us: 100_000 } }
fn capabilities() -> VoiceCapabilities {
    native_duplex_requirements().into_iter().chain([VoiceFeature::TypedTools, VoiceFeature::Delegation])
        .map(|f| (f, VoiceCapabilityEvidence { support: CapabilitySupport::Supported, source: "local contract harness".into() })).collect()
}
fn descriptor() -> VoiceAdapterDescriptor { VoiceAdapterDescriptor { adapter_id: "external.local".into(), contract_version: 1,
    api_version: "local-v1".into(), config_schema_version: 1, label: "Local".into(), capabilities: capabilities(),
    transports: vec![VoiceTransportPreference::Relay], input_specs: vec![spec()], output_specs: vec![spec()],native_requirements:None } }
fn request() -> VoiceOpenRequest { VoiceOpenRequest { voice_session_id: "voice1".into(), activation_epoch: 4, output_generation: 1,
    route_identity: VoiceRouteIdentity { route_id: "local".into(), revision: 1, record_digest: DigestHex("a".repeat(64)), adapter_contract_ref: "external.local@1".into() },
    model: "offline".into(), instructions: String::new(),initial_facts:vec![],replay_scope:VoiceReplayScope{agent_session_id:"agent1".into(),binding_version:2,context_floor:0},initial_local_replay:None,
    tools: vec![], transport: VoiceTransportPreference::Relay, native_offer: None,work_context:None } }
fn frame(sequence: u64) -> AudioFrame { AudioFrame { activation_epoch: 4, output_generation: 1, sequence,
    timestamp: sequence * 20_000, duration_us: 20_000, format: spec().format, payload: vec![0; 640] } }

struct LocalPort { delegate: bool, live: Arc<AtomicUsize> }
#[async_trait]
impl VoiceModelPort for LocalPort {
    fn describe(&self) -> VoiceAdapterDescriptor { descriptor() }
    async fn open(&self, _request: VoiceOpenRequest, cancel: CancellationToken, deadline: Instant) -> Result<VoiceModelSession, VoiceError> {
        if Instant::now() >= deadline { return Err(VoiceError::new(VoiceErrorKind::Deadline, "open deadline")); }
        let negotiation = VoiceNegotiation { capabilities: capabilities(), input_spec: Some(spec()), output_spec: Some(spec()), native_attachment: None };
        let (io, mut worker_io) = VoiceModelSession::channels(Some(spec()), VoiceSessionLimits { close_timeout: Duration::from_millis(50), ..Default::default() }, cancel)?;
        let delegate = self.delegate; let live = self.live.clone(); live.fetch_add(1, Ordering::SeqCst);
        let worker = tokio::spawn(async move {
            struct Guard(Arc<AtomicUsize>); impl Drop for Guard { fn drop(&mut self) { self.0.fetch_sub(1, Ordering::SeqCst); } }
            let _guard = Guard(live);
            loop {
                tokio::select! { biased;
                    _ = worker_io.cancel.cancelled() => break,
                    Some(_control)=worker_io.urgent_rx.recv()=>{worker_io.media_rx.clear();},
                    Some(control) = worker_io.control_rx.recv() => {
                        if matches!(control, VoiceControl::InterruptOutput { .. }) { worker_io.media_rx.clear(); }
                    },
                    Some(_frame) = worker_io.media_rx.recv() => {
                        let trigger = if delegate { WorkTrigger::DelegationTrigger { upstream_trigger_id: "d1".into(), target: "work".into(), offset_ms: 20, context_window_ref: Some("window1".into()) } }
                            else { WorkTrigger::TypedToolCall { upstream_trigger_id: "t1".into(), name: "observe_work".into(), arguments: serde_json::json!({"task":"existing"}), transcript_window_ref: None } };
                        let event = VoiceModelEvent::WorkTrigger { trigger };
                        tokio::select! { biased; _ = worker_io.cancel.cancelled() => break, result = worker_io.event_tx.send(event) => { if result.is_err() { break; } } }
                    },
                }
            }
            worker_io.terminate(VoiceTermination { reason: worker_io.requested_close_reason(), finalization_confirmed: true, message: None });
        });
        Ok(VoiceModelSession::from_parts(negotiation, io, worker))
    }
}
#[tokio::test] async fn two_distinct_work_semantics_run_without_network_through_public_seam() {
    let live = Arc::new(AtomicUsize::new(0));
    for delegate in [false, true] {
        let adapter = LocalPort { delegate, live: live.clone() };
        let mut session = adapter.open(request(), CancellationToken::new(), Instant::now() + Duration::from_secs(1)).await.unwrap();
        let mut core = VoiceSessionCore::new("voice1".into(), "agent1".into(), 2, 4);
        core.negotiate(&session.negotiation).unwrap(); core.capture(true); core.work_running(true);
        core.admit_input(&frame(1)).unwrap(); session.input.try_audio(frame(1)).unwrap();
        let event = tokio::time::timeout(Duration::from_secs(1), session.events.recv()).await.unwrap().unwrap();
        assert!(core.accept_event(&event).unwrap());
        assert!(matches!(event, VoiceModelEvent::WorkTrigger { trigger: WorkTrigger::DelegationTrigger { .. } }) == delegate);
        assert!(core.state().work_running); assert_eq!(core.state().capture, VoiceCaptureState::Capturing);
        assert!(session.shutdown(VoiceCloseReason::UserEnded).await.finalization_confirmed);
    }
    assert_eq!(live.load(Ordering::SeqCst), 0);
}
#[tokio::test] async fn close_bypasses_full_media_and_control_and_abort_is_joined() {
    let cancel = CancellationToken::new();
    let (io, _worker_io) = VoiceModelSession::channels(Some(spec()), VoiceSessionLimits { media_slots: 3, control_slots: 1, close_timeout: Duration::from_millis(10), ..Default::default() }, cancel.clone()).unwrap();
    for sequence in 0..3 { io.input.try_audio(frame(sequence)).unwrap(); }
    assert_eq!(io.input.try_audio(frame(4)).unwrap_err().kind, VoiceErrorKind::Backlog);
    io.input.try_control(VoiceControl::UpdateConfiguration { patch:VoiceConfigurationPatch{instructions:VoicePatch::Keep,tools:VoicePatch::Keep} }).unwrap();
    let live = Arc::new(AtomicUsize::new(1)); let worker_live = live.clone();
    let worker = tokio::spawn(async move {
        struct Guard(Arc<AtomicUsize>); impl Drop for Guard { fn drop(&mut self) { self.0.store(0, Ordering::SeqCst); } }
        let _guard = Guard(worker_live); std::future::pending::<()>().await;
    });
    let session = VoiceModelSession::from_parts(VoiceNegotiation { capabilities: capabilities(), input_spec: Some(spec()), output_spec: Some(spec()), native_attachment: None }, io, worker);
    tokio::task::yield_now().await;
    let result = session.shutdown(VoiceCloseReason::AppShutdown).await;
    assert!(!result.finalization_confirmed); assert!(cancel.is_cancelled()); assert_eq!(live.load(Ordering::SeqCst), 0);
}
#[tokio::test]async fn output_media_budget_and_urgent_controls_are_not_slot_count_guesses(){
    let cancel=CancellationToken::new();
    let (io,mut worker)=VoiceModelSession::channels_with_specs(Some(spec()),Some(spec()),VoiceSessionLimits{control_slots:1,..Default::default()},cancel.clone()).unwrap();
    io.input.try_control(VoiceControl::UpdateConfiguration{patch:VoiceConfigurationPatch{instructions:VoicePatch::Keep,tools:VoicePatch::Keep}}).unwrap();
    io.input.try_control(VoiceControl::InterruptOutput{output_generation:2,played:None}).unwrap();
    assert!(matches!(worker.urgent_rx.recv().await,Some(VoiceControl::InterruptOutput{output_generation:2,..})));
    for sequence in 0..3{worker.event_tx.try_send(VoiceModelEvent::Audio{segment_id:"s".into(),frame:frame(sequence)}).unwrap();}
    assert_eq!(worker.event_tx.try_send(VoiceModelEvent::Audio{segment_id:"s".into(),frame:frame(4)}).unwrap_err().kind,VoiceErrorKind::Backlog);
    assert!(cancel.is_cancelled());assert_eq!(worker.requested_close_reason(),VoiceCloseReason::Backlog);
}
#[test] fn late_output_does_not_cross_generation_and_native_waits_for_safe_boundary() {
    let mut core = VoiceSessionCore::new("v".into(), "a".into(), 1, 4);
    let control = core.interrupt_output(true);
    assert!(matches!(control, VoiceControl::InterruptOutput { output_generation: 2, .. }));
    let old = VoiceModelEvent::OutputStarted { segment: OutputSegment { response_id: "r".into(), segment_id: "s".into(), revision: 1, output_generation: 1, text: None, correlation_id: None } };
    assert!(!core.accept_event(&old).unwrap());
    let new = VoiceModelEvent::OutputStarted { segment: OutputSegment { response_id: "r2".into(), segment_id: "s2".into(), revision: 1, output_generation: 2, text: None, correlation_id: None } };
    assert!(!core.accept_event(&new).unwrap());
    core.accept_event(&VoiceModelEvent::ConfigurationApplied).unwrap();
    assert!(!core.accept_event(&new).unwrap());
    assert!(!core.accept_event(&VoiceModelEvent::OutputBoundary{output_generation:3,attachment_id:Some("future".into())}).unwrap());
    assert!(!core.accept_event(&new).unwrap());
    core.accept_event(&VoiceModelEvent::OutputBoundary { output_generation: 2, attachment_id: Some("rebuilt".into()) }).unwrap();
    assert!(core.accept_event(&new).unwrap());
}

#[test] fn late_played_revision_preserves_cursor_without_changing_current_playback_and_transcripts_keep_order(){
    let mut core=VoiceSessionCore::new("v".into(),"a".into(),1,4);
    for(id,text)in[("z","first"),("a","second")]{
        core.accept_event(&VoiceModelEvent::Transcript{fragment:TranscriptFragment{fragment_id:id.into(),revision:1,speaker:VoiceSpeaker::User,text:text.into(),commit:TranscriptCommit::Committed,media_range:None}}).unwrap();
    }
    assert_eq!(core.transcripts().iter().map(|f|f.text.as_str()).collect::<Vec<_>>(),vec!["first","second"]);
    let segment=OutputSegment{response_id:"r".into(),segment_id:"s".into(),revision:1,output_generation:1,text:None,correlation_id:None};
    core.accept_event(&VoiceModelEvent::OutputStarted{segment:segment.clone()}).unwrap();
    let receipt=PlaybackReceipt{activation_epoch:4,segment_id:"s".into(),revision:1,output_generation:1,state:DeliveryState::Played,consumed_us:1000,uncertain_tail_us:0,precision:PlaybackPrecision::Exact};
    core.receipt(receipt.clone()).unwrap();core.interrupt_output(false);
    core.accept_event(&VoiceModelEvent::OutputStarted{segment:OutputSegment{revision:2,output_generation:2,..segment}}).unwrap();
    core.receipt(PlaybackReceipt{consumed_us:2000,..receipt.clone()}).unwrap();
    assert_eq!(core.state().playback,VoicePlaybackState::Interrupted);
    assert!(core.receipt(receipt).is_err(),"old revision cursor remains monotonic after replacement");
    assert!(matches!(core.interrupt_output(false),VoiceControl::InterruptOutput{played:None,..}),"old generation consumption is not a current response cursor");
}
