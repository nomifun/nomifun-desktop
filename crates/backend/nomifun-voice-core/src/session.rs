use std::collections::{BTreeMap,VecDeque};
use nomifun_voice_contracts::voice::*;

/// Orthogonal media facts, with no work execution or model selection.
pub struct VoiceSessionCore {
    state: VoiceState,
    input_spec: Option<MediaSpec>,
    output_spec: Option<MediaSpec>,
    last_input_sequence: Option<u64>,
    segments: BTreeMap<String, OutputSegment>,
    playback: BTreeMap<String, PlaybackReceipt>,
    transcripts: BTreeMap<(VoiceSpeakerKey, String), TranscriptFragment>,
    native_blocked: bool,
    transcript_order:VecDeque<(VoiceSpeakerKey,String)>,segment_order:VecDeque<String>,last_output_sequences:BTreeMap<String,u64>,
    known_segment_versions:VecDeque<(String,u64,u64)>,receipt_versions:BTreeMap<(String,u64,u64),PlaybackReceipt>,
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum VoiceSpeakerKey { User, Assistant }
impl From<VoiceSpeaker> for VoiceSpeakerKey { fn from(v: VoiceSpeaker) -> Self { match v { VoiceSpeaker::User => Self::User, VoiceSpeaker::Assistant => Self::Assistant } } }
impl VoiceSessionCore {
    pub fn new(voice_session_id: String, agent_session_id: String, binding_version: u64, activation_epoch: u64) -> Self {
        Self { state: VoiceState { voice_session_id, agent_session_id, binding_version, activation_epoch, output_generation: 1,
            connection: VoiceConnectionState::Opening, capture: VoiceCaptureState::Idle, playback: VoicePlaybackState::Idle, work_running: false },
            input_spec: None, output_spec: None, last_input_sequence: None, segments: BTreeMap::new(), playback: BTreeMap::new(), transcripts: BTreeMap::new(), native_blocked: false,
            transcript_order:VecDeque::new(),segment_order:VecDeque::new(),last_output_sequences:BTreeMap::new(),known_segment_versions:VecDeque::new(),receipt_versions:BTreeMap::new() }
    }
    pub fn state(&self) -> &VoiceState { &self.state }
    pub fn transcripts(&self)->Vec<TranscriptFragment>{self.transcript_order.iter().filter_map(|key|self.transcripts.get(key).cloned()).collect()}
    pub fn playback_receipts(&self)->Vec<PlaybackReceipt>{self.receipt_versions.values().filter(|receipt|receipt.consumed_us>0).cloned().collect()}
    fn clear_assistant_transcripts(&mut self){
        self.transcripts.retain(|(speaker,_),_|*speaker==VoiceSpeakerKey::User);
        self.transcript_order.retain(|(speaker,_)|*speaker==VoiceSpeakerKey::User);
    }
    pub fn segment(&self,id:&str)->Option<OutputSegment>{self.segments.get(id).cloned()}
    pub fn negotiate(&mut self, negotiation: &VoiceNegotiation) -> Result<(), VoiceError> {
        validate_native_duplex(&negotiation.capabilities).map_err(|e| VoiceError::new(VoiceErrorKind::Unsupported, e))?;
        for spec in [&negotiation.input_spec, &negotiation.output_spec].into_iter().flatten() {
            spec.validate().map_err(|e| VoiceError::new(VoiceErrorKind::Configuration, e))?;
        }
        if negotiation.native_attachment.is_some() && (negotiation.input_spec.is_some() || negotiation.output_spec.is_some()) {
            return Err(VoiceError::new(VoiceErrorKind::Configuration, "native media must not duplicate relay audio"));
        }
        self.input_spec = negotiation.input_spec.clone(); self.output_spec = negotiation.output_spec.clone();
        self.state.connection = VoiceConnectionState::Ready;
        Ok(())
    }
    pub fn capture(&mut self, capturing: bool) { self.state.capture = if capturing { VoiceCaptureState::Capturing } else { VoiceCaptureState::Paused }; }
    pub fn work_running(&mut self, running: bool) { self.state.work_running = running; }
    pub fn admit_input(&mut self, frame: &AudioFrame) -> Result<(), VoiceError> {
        self.check_epoch(frame.activation_epoch)?;
        if self.state.capture != VoiceCaptureState::Capturing || self.state.connection != VoiceConnectionState::Ready {
            return Err(VoiceError::new(VoiceErrorKind::Closed, "capture is not active"));
        }
        frame.validate_for(self.input_spec.as_ref().ok_or_else(|| VoiceError::new(VoiceErrorKind::Unsupported, "native media uses tracks"))?)
            .map_err(|e| VoiceError::new(VoiceErrorKind::Configuration, e))?;
        if self.last_input_sequence.is_some_and(|s| frame.sequence <= s) { return Err(VoiceError::new(VoiceErrorKind::StaleEpoch, "duplicate or late input audio")); }
        self.last_input_sequence = Some(frame.sequence); Ok(())
    }
    pub fn accept_event(&mut self, event: &VoiceModelEvent) -> Result<bool, VoiceError> {
        match event {
            VoiceModelEvent::Ready { negotiation } => { self.negotiate(negotiation)?; }
            VoiceModelEvent::Transcript { fragment } => {
                if fragment.fragment_id.trim().is_empty()||fragment.fragment_id.len()>1024||fragment.revision==0||fragment.text.len()>32*1024
                    || fragment.media_range.as_ref().is_some_and(|r|r.end_us<r.start_us){
                    return Err(VoiceError::new(VoiceErrorKind::Provider,"invalid or oversized voice transcript"));
                }
                let key = (fragment.speaker.into(), fragment.fragment_id.clone());
                if self.transcripts.get(&key).is_some_and(|f| f.revision >= fragment.revision) { return Ok(false); }
                if !self.transcripts.contains_key(&key){self.transcript_order.push_back(key.clone());}
                self.transcripts.insert(key, fragment.clone());
                while self.transcript_order.len()>512{if let Some(old)=self.transcript_order.pop_front(){self.transcripts.remove(&old);}}
            }
            VoiceModelEvent::OutputStarted { segment } => {
                if segment.output_generation != self.state.output_generation || self.native_blocked { return Ok(false); }
                let key=(segment.segment_id.clone(),segment.revision,segment.output_generation);
                if !self.known_segment_versions.contains(&key){self.known_segment_versions.push_back(key);}
                while self.known_segment_versions.len()>512{if let Some(old)=self.known_segment_versions.pop_front(){self.receipt_versions.remove(&old);}}
                if !self.segments.contains_key(&segment.segment_id){self.segment_order.push_back(segment.segment_id.clone());}
                if self.segments.get(&segment.segment_id).is_some_and(|prior|prior.revision!=segment.revision||prior.output_generation!=segment.output_generation){self.last_output_sequences.remove(&segment.segment_id);}
                self.segments.insert(segment.segment_id.clone(), segment.clone());
                while self.segment_order.len()>256{if let Some(old)=self.segment_order.pop_front(){self.segments.remove(&old);self.playback.remove(&old);self.last_output_sequences.remove(&old);}}
            }
            VoiceModelEvent::Audio { segment_id, frame } => {
                self.check_epoch(frame.activation_epoch)?;
                if frame.output_generation != self.state.output_generation || self.native_blocked { return Ok(false); }
                if !self.segments.contains_key(segment_id) { return Err(VoiceError::new(VoiceErrorKind::Provider, "audio has no output segment")); }
                if self.last_output_sequences.get(segment_id).is_some_and(|prior|frame.sequence<=*prior){return Ok(false);}
                frame.validate_for(self.output_spec.as_ref().ok_or_else(|| VoiceError::new(VoiceErrorKind::Unsupported, "native audio is delivered through tracks"))?)
                    .map_err(|e| VoiceError::new(VoiceErrorKind::Provider, e))?;
                self.last_output_sequences.insert(segment_id.clone(),frame.sequence);
            }
            VoiceModelEvent::OutputBoundary { output_generation, .. } => {
                if *output_generation!=self.state.output_generation{return Ok(false);}
                self.native_blocked = false;
            }
            VoiceModelEvent::NativeAttachmentRequired{output_generation} if *output_generation!=self.state.output_generation=>return Ok(false),
            VoiceModelEvent::OutputInterrupted { .. } => { self.state.playback = VoicePlaybackState::Interrupted; }
            VoiceModelEvent::RelayRecovering{output_generation}=>{
                if *output_generation!=self.state.output_generation||self.input_spec.is_none(){return Ok(false);}
                self.state.connection=VoiceConnectionState::Recovering;self.state.capture=VoiceCaptureState::Paused;self.clear_assistant_transcripts();
            }
            VoiceModelEvent::RelayRecovered{output_generation}=>{
                if *output_generation!=self.state.output_generation||self.input_spec.is_none()||self.state.connection!=VoiceConnectionState::Recovering{return Ok(false);}
                self.state.connection=VoiceConnectionState::Ready;self.state.capture=VoiceCaptureState::Paused;
            }
            VoiceModelEvent::Error { .. } => { self.state.connection = VoiceConnectionState::Failed; self.state.capture = VoiceCaptureState::Unavailable; }
            VoiceModelEvent::Closed { .. } => { self.state.connection = VoiceConnectionState::Closed; self.state.capture = VoiceCaptureState::Idle; self.state.playback = VoicePlaybackState::Idle; }
            _ => {}
        }
        Ok(true)
    }
    pub fn receipt(&mut self, receipt: PlaybackReceipt) -> Result<(), VoiceError> {
        self.check_epoch(receipt.activation_epoch)?;
        let key=(receipt.segment_id.clone(),receipt.revision,receipt.output_generation);
        if !self.known_segment_versions.contains(&key){return Err(VoiceError::new(VoiceErrorKind::StaleEpoch,"unknown playback segment revision"));}
        if self.receipt_versions.get(&key).is_some_and(|r|receipt.consumed_us<r.consumed_us){return Err(VoiceError::new(VoiceErrorKind::Configuration,"playback cursor cannot regress"));}
        let current=self.segments.get(&receipt.segment_id).is_some_and(|s|s.revision==receipt.revision&&s.output_generation==receipt.output_generation);
        if current&&receipt.output_generation==self.state.output_generation{
            self.state.playback = match receipt.state { DeliveryState::Queued => VoicePlaybackState::Playing, DeliveryState::Interrupted => VoicePlaybackState::Interrupted, _ => VoicePlaybackState::Idle };
        }
        if current{self.playback.insert(receipt.segment_id.clone(),receipt.clone());}
        self.receipt_versions.insert(key,receipt);Ok(())
    }
    /// Physical endpoint flush/detach must happen before forwarding this control.
    pub fn interrupt_output(&mut self, native: bool) -> VoiceControl {
        let played=self.segment_order.iter().rev().filter_map(|id|self.playback.get(id)).find(|r|r.output_generation==self.state.output_generation).cloned();
        self.state.output_generation += 1;
        self.clear_assistant_transcripts();
        self.state.playback = VoicePlaybackState::Interrupted;
        self.native_blocked = native;
        VoiceControl::InterruptOutput { output_generation: self.state.output_generation,
            played }
    }
    pub fn revoke_unplayed(&mut self, segment_id: &str, revision: u64) -> bool {
        let Some(segment) = self.segments.get(segment_id) else { return false; };
        if segment.revision > revision { return false; }
        // Preserve consumed facts even when the remainder is revoked.
        if let Some(receipt) = self.playback.get_mut(segment_id) { receipt.state = DeliveryState::Revoked; }
        true
    }
    pub fn close(&mut self, unavailable: bool) {
        self.clear_assistant_transcripts();
        self.state.connection = VoiceConnectionState::Closing;
        self.state.capture = if unavailable { VoiceCaptureState::Unavailable } else { VoiceCaptureState::Idle };
        self.state.playback = VoicePlaybackState::Idle;
        self.state.output_generation += 1;
    }
    pub fn recovering_attachment(&mut self) {
        self.state.connection=VoiceConnectionState::Recovering;
        self.state.capture=VoiceCaptureState::Paused;
        self.state.playback=VoicePlaybackState::Interrupted;
        self.native_blocked=true;
    }
    pub fn check_epoch(&self, epoch: u64) -> Result<(), VoiceError> {
        if epoch != self.state.activation_epoch { return Err(VoiceError::new(VoiceErrorKind::StaleEpoch, "voice activation epoch changed")); }
        Ok(())
    }
}
