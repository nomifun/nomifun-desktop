//! Versioned interaction, media and delivery facts. No wire, device or authority owner lives here.
use std::collections::{BTreeMap, BTreeSet};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::{DigestHex, digest_payload};

pub const VOICE_CONTRACT_VERSION: u32 = 1;
pub const VOICE_ROUTE_SCHEMA: &str = "nomifun.voice-route-record.v1";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentInteraction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<AgentVoiceInteraction>,
}
impl AgentInteraction { pub fn is_empty(&self) -> bool { self.voice.is_none() } }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentVoiceInteraction {
    pub enabled: bool,
    pub mode: VoiceInteractionMode,
    pub route_key: String,
    pub work_policy: VoiceWorkPolicy,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VoiceInteractionMode { FullDuplex }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VoiceWorkPolicy { ExplicitIntent }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VoiceTransportPreference { #[default] Relay, NativeWebrtc }

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VoiceFeature {
    ContinuousInput, UnderstandInputDuringOutput, SemanticTurnTaking, FactInjection,
    TypedTools, Delegation, InputTranscript, OutputTranscript, InterruptGeneration,
    ExactOutputTruncation, ControlledOutputBoundary, NativeMedia,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySupport { Supported, Unsupported, Unknown }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceCapabilityEvidence {
    pub support: CapabilitySupport,
    pub source: String,
}
pub type VoiceCapabilities = BTreeMap<VoiceFeature, VoiceCapabilityEvidence>;
pub fn native_duplex_requirements() -> BTreeSet<VoiceFeature> {
    [VoiceFeature::ContinuousInput, VoiceFeature::UnderstandInputDuringOutput,
     VoiceFeature::SemanticTurnTaking, VoiceFeature::FactInjection].into_iter().collect()
}
pub fn validate_native_duplex(capabilities: &VoiceCapabilities) -> Result<(), String> {
    let supported = |feature| capabilities.get(&feature).is_some_and(|v| v.support == CapabilitySupport::Supported);
    for feature in native_duplex_requirements() {
        if !supported(feature) { return Err(format!("required voice capability {feature:?} is unsupported or unverified")); }
    }
    if !supported(VoiceFeature::TypedTools) && !supported(VoiceFeature::Delegation) {
        return Err("native duplex work requires typed tools or delegation".into());
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceRouteRecord {
    pub schema: String,
    pub route_id: String,
    pub revision: u64,
    pub provider_id: String,
    pub model: String,
    pub model_revision: u64,
    pub connection_config_ref: String,
    pub credential_ref: String,
    pub adapter_id: String,
    pub adapter_contract_version: u32,
    /// Validated adapter-private configuration, never interpreted by the core.
    pub adapter_config: Value,
    pub adapter_config_digest: DigestHex,
    pub connection_config_digest: DigestHex,
    pub required_features: BTreeSet<VoiceFeature>,
    pub transport: VoiceTransportPreference,
}
impl VoiceRouteRecord {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != VOICE_ROUTE_SCHEMA || self.revision == 0
            || self.adapter_contract_version != VOICE_CONTRACT_VERSION { return Err("invalid voice route version".into()); }
        for value in [&self.route_id, &self.provider_id, &self.model, &self.connection_config_ref,
            &self.credential_ref, &self.adapter_id] {
            if value.trim().is_empty() || value.len() > 1024 { return Err("invalid voice route identity".into()); }
        }
        if !self.adapter_config.is_object() { return Err("adapter config must be a validated object".into()); }
        if digest_payload(&self.adapter_config).map_err(|e| e.to_string())? != self.adapter_config_digest {
            return Err("voice adapter config digest mismatch".into());
        }
        if !self.required_features.is_superset(&native_duplex_requirements())
            || (!self.required_features.contains(&VoiceFeature::TypedTools) && !self.required_features.contains(&VoiceFeature::Delegation)) {
            return Err("voice route does not require native duplex work capabilities".into());
        }
        for digest in [&self.adapter_config_digest, &self.connection_config_digest] {
            if digest.as_ref().len() != 64 || !digest.as_ref().bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
                return Err("invalid voice route digest".into());
            }
        }
        Ok(())
    }
    pub fn identity(&self) -> Result<VoiceRouteIdentity, String> {
        self.validate()?;
        Ok(VoiceRouteIdentity { route_id: self.route_id.clone(), revision: self.revision,
            record_digest: digest_payload(self).map_err(|e| e.to_string())?,
            adapter_contract_ref: format!("{}@{}", self.adapter_id, self.adapter_contract_version) })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceRouteIdentity {
    pub route_id: String,
    pub revision: u64,
    pub record_digest: DigestHex,
    pub adapter_contract_ref: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedVoicePlan {
    pub interaction: AgentVoiceInteraction,
    pub identity: VoiceRouteIdentity,
}
impl ResolvedVoicePlan {
    pub fn validate(&self) -> Result<(), String> {
        if !self.interaction.enabled || self.interaction.route_key.trim().is_empty()
            || self.identity.route_id.trim().is_empty() || self.identity.revision == 0
            || self.identity.record_digest.as_ref().len() != 64 || self.identity.adapter_contract_ref.trim().is_empty() {
            return Err("invalid frozen voice plan".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AudioEncoding { Pcm, Opus, Wav, Mp3, Aac, Flac, G711MuLaw, G711ALaw }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AudioSampleFormat { Signed16Le, Float32Le, Encoded }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioFormat {
    pub encoding: AudioEncoding,
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: AudioSampleFormat,
}
impl AudioFormat {
    pub fn pcm16(sample_rate: u32, channels: u16) -> Self {
        Self { encoding: AudioEncoding::Pcm, sample_rate, channels, sample_format: AudioSampleFormat::Signed16Le }
    }
    pub fn validate(&self) -> Result<(), String> {
        if !(8_000..=192_000).contains(&self.sample_rate) || self.channels == 0 || self.channels > 8 {
            return Err("unsupported sample rate or channel count".into());
        }
        if (self.encoding == AudioEncoding::Pcm) == (self.sample_format == AudioSampleFormat::Encoded) {
            return Err("audio encoding and sample format disagree".into());
        }
        Ok(())
    }
    pub fn pcm_frame_bytes(&self) -> Option<usize> {
        if self.encoding != AudioEncoding::Pcm { return None; }
        Some(usize::from(self.channels) * match self.sample_format {
            AudioSampleFormat::Signed16Le => 2, AudioSampleFormat::Float32Le => 4, AudioSampleFormat::Encoded => return None,
        })
    }
    pub fn duration_us(&self, bytes: usize) -> Result<u64, String> {
        self.validate()?;
        let frame_bytes = self.pcm_frame_bytes().ok_or("encoded duration requires codec metadata")?;
        if !bytes.is_multiple_of(frame_bytes) { return Err("partial PCM sample frame".into()); }
        Ok((bytes / frame_bytes) as u64 * 1_000_000 / u64::from(self.sample_rate))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaSpec {
    pub format: AudioFormat,
    /// Timestamp units per second; duration is always explicitly in microseconds.
    pub timebase: u32,
    pub min_frame_duration_us: u32,
    pub max_frame_duration_us: u32,
    pub max_frame_bytes: u32,
    pub max_buffer_duration_us: u32,
    pub max_frame_age_us: u32,
}
impl MediaSpec {
    pub fn validate(&self) -> Result<(), String> {
        self.format.validate()?;
        if self.timebase == 0 || self.min_frame_duration_us == 0
            || self.max_frame_duration_us < self.min_frame_duration_us || self.max_frame_bytes == 0
            || self.max_buffer_duration_us < self.max_frame_duration_us || self.max_frame_age_us == 0 {
            return Err("invalid media cadence or latency budget".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioFrame {
    pub activation_epoch: u64,
    pub output_generation: u64,
    pub sequence: u64,
    pub timestamp: u64,
    pub duration_us: u32,
    pub format: AudioFormat,
    pub payload: Vec<u8>,
}
impl AudioFrame {
    pub fn validate_for(&self, spec: &MediaSpec) -> Result<(), String> {
        spec.validate()?;
        if self.format != spec.format || self.payload.len() > spec.max_frame_bytes as usize
            || self.duration_us < spec.min_frame_duration_us || self.duration_us > spec.max_frame_duration_us {
            return Err("audio frame violates negotiated media spec".into());
        }
        if self.format.encoding == AudioEncoding::Pcm {
            let actual = self.format.duration_us(self.payload.len())?;
            if actual.abs_diff(u64::from(self.duration_us)) > 1 { return Err("PCM duration does not match payload".into()); }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VoiceSpeaker { User, Assistant }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptCommit { Tentative, Committed }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaRange { pub start_us: u64, pub end_us: u64 }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TranscriptFragment {
    pub speaker: VoiceSpeaker,
    pub fragment_id: String,
    pub revision: u64,
    pub commit: TranscriptCommit,
    pub text: String,
    pub media_range: Option<MediaRange>,
}
/// Canonical reset/binding boundary referenced by voice-local restoration.
/// This is not a second Agent checkpoint or an execution permission.
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceReplayScope{pub agent_session_id:crate::AgentSessionId,pub binding_version:u64,pub context_floor:u64}
impl VoiceReplayScope{
    pub fn validate(&self)->Result<(),String>{
        if self.agent_session_id.as_ref().trim().is_empty()||self.agent_session_id.as_ref().len()>1024||self.binding_version==0{return Err("invalid voice replay scope".into());}Ok(())
    }
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceLocalFactRef{pub voice_session_id:String,pub activation_epoch:u64,pub journal_sequence:u64}
impl VoiceLocalFactRef{
    pub fn validate(&self)->Result<(),String>{if self.voice_session_id.trim().is_empty()||self.voice_session_id.len()>1024||self.activation_epoch==0||self.journal_sequence==0{return Err("invalid voice-local fact reference".into());}Ok(())}
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize,JsonSchema)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
pub enum VoiceLocalReplayEntry{
    UserCommitted{origin:VoiceLocalFactRef,fragment:TranscriptFragment},
    Playback{origin:VoiceLocalFactRef,receipt:PlaybackReceipt},
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceLocalReplay{pub scope:VoiceReplayScope,pub entries:Vec<VoiceLocalReplayEntry>}
impl VoiceLocalReplay{
    pub fn validate_for(&self,current:&VoiceReplayScope)->Result<(),String>{
        self.scope.validate()?;current.validate()?;
        if &self.scope!=current{return Err("voice replay crosses the current Session binding or context floor".into());}
        if self.entries.len()>64||serde_json::to_vec(self).map_err(|_|"invalid replay encoding")?.len()>64*1024{return Err("voice replay exceeds bounded context budget".into());}
        let mut users=BTreeSet::new();let mut origins=BTreeSet::new();
        for entry in &self.entries{match entry{
            VoiceLocalReplayEntry::UserCommitted{origin,fragment}=>{
                origin.validate()?;
                if fragment.speaker!=VoiceSpeaker::User||fragment.commit!=TranscriptCommit::Committed||fragment.fragment_id.trim().is_empty()||fragment.fragment_id.len()>1024||fragment.revision==0||fragment.text.trim().is_empty()
                    ||fragment.media_range.as_ref().is_some_and(|range|range.end_us<range.start_us){return Err("voice replay requires a complete committed user fragment".into());}
                if !users.insert((origin.voice_session_id.clone(),origin.activation_epoch,fragment.fragment_id.clone())){return Err("voice replay contains multiple revisions of one user fragment".into());}
                if !origins.insert((origin.voice_session_id.clone(),origin.activation_epoch,origin.journal_sequence)){return Err("voice-local fact reference was repeated".into());}
            },
            VoiceLocalReplayEntry::Playback{origin,receipt}=>{
                origin.validate()?;
                if receipt.activation_epoch!=origin.activation_epoch||receipt.segment_id.trim().is_empty()||receipt.segment_id.len()>1024||receipt.revision==0||receipt.output_generation==0||receipt.consumed_us==0
                    ||!matches!(receipt.state,DeliveryState::Played|DeliveryState::Interrupted){return Err("voice replay requires reported consumed media timing, not generated or queued output".into());}
                if !origins.insert((origin.voice_session_id.clone(),origin.activation_epoch,origin.journal_sequence)){return Err("voice-local fact reference was repeated".into());}
            },
        }}Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceToolDefinition { pub name: String, pub description: String, pub parameters: Value }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkTrigger {
    TypedToolCall { upstream_trigger_id: String, name: String, arguments: Value, transcript_window_ref: Option<String> },
    DelegationTrigger { upstream_trigger_id: String, target: String, offset_ms: u64, context_window_ref: Option<String> },
}
impl WorkTrigger {
    pub fn upstream_trigger_id(&self) -> &str { match self {
        Self::TypedToolCall { upstream_trigger_id, .. } | Self::DelegationTrigger { upstream_trigger_id, .. } => upstream_trigger_id,
    } }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", content = "value", rename_all = "snake_case")]
pub enum VoicePatch<T> { Keep, Clear, Set(T) }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceConfigurationPatch { pub instructions: VoicePatch<String>, pub tools: VoicePatch<Vec<VoiceToolDefinition>> }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState { Generated, Queued, Played, Interrupted, Revoked }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackPrecision { Exact, Estimated, Unknown }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputSegment {
    pub response_id: String,
    pub segment_id: String,
    pub revision: u64,
    pub output_generation: u64,
    pub text: Option<String>,
    pub correlation_id: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlaybackReceipt {
    pub activation_epoch: u64,
    pub segment_id: String,
    pub revision: u64,
    pub output_generation: u64,
    pub state: DeliveryState,
    pub consumed_us: u64,
    pub uncertain_tail_us: u64,
    pub precision: PlaybackPrecision,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerifiedVoiceFact {
    pub correlation_id: String,
    pub upstream_trigger_id: Option<String>,
    pub canonical_receipt_id: String,
    pub content: String,
    pub speak: bool,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub output_generation:Option<u64>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub work_context:Option<VoiceWorkContextFact>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub speech_source:Option<VoiceSpeechSourceRef>,
}
/// Canonical source identity; it is independent of a model's audio segment ID.
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceSpeechSourceRef {
    pub message_id:String,
    pub revision:u64,
    pub through_seq:u64,
}
impl VoiceSpeechSourceRef {
    pub fn validate(&self)->Result<(),String>{
        if self.message_id.trim().is_empty()||self.message_id.len()>1024||self.revision==0||self.through_seq<self.revision{
            return Err("invalid canonical speech source".into());
        }
        Ok(())
    }
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceWorkContextFact { pub target:Option<crate::WorkTarget> }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VoiceControl {
    MuteInput { muted: bool },
    /// Application-owned, correlated provider/local-input acceptance. Client
    /// requests stay MuteInput and cannot select this correlation identity.
    SetInputMuted { muted:bool,request_id:String },
    /// Data-only committed local input identity, never a canonical receipt or
    /// permission grant. A typed work tool must cite this explicit reference.
    PresentInputSource {fragment:TranscriptFragment},
    InterruptOutput { output_generation: u64, played: Option<PlaybackReceipt> },
    UpdateConfiguration { patch: VoiceConfigurationPatch },
    InjectFact { fact: VerifiedVoiceFact },
    /// Application-owned revocation; must precede any rebuild that could replay facts.
    RevokeSpeech { source:VoiceSpeechSourceRef,output_generation:u64 },
    RejectWorkTrigger { upstream_trigger_id:String,reason:String },
    Playback { receipt: PlaybackReceipt },
    Close { reason: VoiceCloseReason },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VoiceCloseReason { UserEnded, LeaseRevoked, BindingChanged, DeviceUnavailable, PermissionRevoked, NetworkLost, Backlog, JournalUnavailable, AppShutdown, ModelLimit, ProviderFailed }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VoiceConnectionState { Opening, Ready, Recovering, Closing, Closed, Failed }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VoiceCaptureState { Idle, Capturing, Paused, Unavailable }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VoicePlaybackState { Idle, Playing, Interrupted }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceState {
    pub voice_session_id: String,
    pub agent_session_id: String,
    pub binding_version: u64,
    pub activation_epoch: u64,
    pub output_generation: u64,
    pub connection: VoiceConnectionState,
    pub capture: VoiceCaptureState,
    pub playback: VoicePlaybackState,
    pub work_running: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceTermination { pub reason: VoiceCloseReason, pub finalization_confirmed: bool, pub message: Option<String> }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VoiceErrorKind { Configuration, Unsupported, Authentication, Quota, Network, Deadline, Backlog, StaleEpoch, StaleBinding, SourceContextRequired, JournalUnavailable, Closed, Provider }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[error("{message}")]
#[serde(deny_unknown_fields)]
pub struct VoiceError { pub kind: VoiceErrorKind, pub message: String, pub retryable: bool }
impl VoiceError { pub fn new(kind: VoiceErrorKind, message: impl Into<String>) -> Self { Self { kind, message: message.into(), retryable: false } } }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceAdapterDescriptor {
    pub adapter_id: String,
    pub contract_version: u32,
    pub api_version: String,
    pub config_schema_version: u32,
    pub label: String,
    pub capabilities: VoiceCapabilities,
    pub transports: Vec<VoiceTransportPreference>,
    pub input_specs: Vec<MediaSpec>,
    pub output_specs: Vec<MediaSpec>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub native_requirements:Option<VoiceNativeRequirements>,
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceNativeRequirements{pub data_channel_label:Option<String>}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceNegotiation {
    pub capabilities: VoiceCapabilities,
    pub input_spec: Option<MediaSpec>,
    pub output_spec: Option<MediaSpec>,
    pub native_attachment: Option<VoiceNativeAttachment>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceNativeAttachment { pub attachment_id: String, pub answer_sdp: String, pub safe_output_boundary: bool }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VoiceWorkStatus { Accepted, Applied, PendingBoundary, Rejected, Deferred, Queued, Terminal }
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceSpeechProjection {pub message_id:String,pub revision:u64,pub through_seq:u64,pub text:String}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceWorkReceipt {
    pub receipt_id: String,
    pub operation_key: String,
    pub target: Option<crate::WorkTarget>,
    pub pending_input_id: Option<String>,
    pub status: VoiceWorkStatus,
    /// A filtered canonical presentation, never a raw tool result or reasoning.
    pub summary: String,
    pub duplicate: bool,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub speech: Option<VoiceSpeechProjection>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub pending_input_revision:Option<u64>,
    /// The first same-operation native claim, read from canonical typed facts.
    /// A current generation alone cannot prove a safe opening-zero upgrade.
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub first_claim_generation:Option<u64>,
}
impl VoiceWorkReceipt {
    pub fn canonical_reference(&self)->Option<&str>{
        (self.target.is_some()&&!self.receipt_id.is_empty()&&!self.receipt_id.starts_with("voice-input:")&&self.status!=VoiceWorkStatus::Queued).then_some(self.receipt_id.as_str())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum VoiceWorkRequest {
    Start { text: String },
    Steer { target: crate::WorkTarget, text: String },
    Observe { target: crate::WorkTarget },
    Cancel { target: crate::WorkTarget },
    CancelQueued { pending_input_id: String },
    ReviseQueued { pending_input_id:String,text:String,expected_revision:u64 },
    AnswerApproval { answer: crate::VoiceApprovalAnswer },
}
/// The application's originating request classification. This telemetry is
/// neither supplier tool arguments nor a work/permission/completion proof.
#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize,Deserialize,JsonSchema)]
#[serde(rename_all="snake_case")]
pub enum VoiceWorkRequestKind {Start,Steer,Observe,Cancel,CancelQueued,ReviseQueued,AnswerApproval}
impl VoiceWorkRequest {
    pub fn kind(&self)->VoiceWorkRequestKind{match self{
        Self::Start{..}=>VoiceWorkRequestKind::Start,Self::Steer{..}=>VoiceWorkRequestKind::Steer,
        Self::Observe{..}=>VoiceWorkRequestKind::Observe,Self::Cancel{..}=>VoiceWorkRequestKind::Cancel,
        Self::CancelQueued{..}=>VoiceWorkRequestKind::CancelQueued,Self::ReviseQueued{..}=>VoiceWorkRequestKind::ReviseQueued,
        Self::AnswerApproval{..}=>VoiceWorkRequestKind::AnswerApproval,
    }}
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VoiceProductEvent {
    /// Media admission credit, never a canonical work receipt or proof of hearing.
    InputAdmitted {activation_epoch:u64,sequence:u64,duration_us:u32},
    /// Returns bounded media credit for a valid frame dropped while input is
    /// paused. It proves neither model admission nor hearing.
    InputReleased {activation_epoch:u64,sequence:u64,duration_us:u32},
    EndpointControl { activation_epoch: u64, control: VoiceControl },
    State { state: VoiceState },
    Model { event: VoiceModelEvent },
    WorkReceipt { receipt: VoiceWorkReceipt, #[serde(default,skip_serializing_if="Option::is_none")] upstream_trigger_id:Option<String>,
        #[serde(default,skip_serializing_if="Option::is_none")] request_kind:Option<VoiceWorkRequestKind> },
    ApprovalPresented { presentation: crate::VoiceApprovalPresentation },
    SourceContextChanged {requirement:Option<crate::VoiceSourceContextRequirement>},
    SpeechRevoked { source_message_id:String,source_revision:u64,output_generation:u64 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VoiceModelEvent {
    Ready { negotiation: VoiceNegotiation },
    ContextOpened { context_window_ref:String, work_context:Option<VoiceWorkContextFact> },
    WorkContextCheckpoint { context_window_ref:String, media_range:MediaRange,
        target:Option<crate::WorkTarget>, canonical_receipt_id:String },
    Transcript { fragment: TranscriptFragment },
    OutputStarted { segment: OutputSegment },
    Audio { segment_id: String, frame: AudioFrame },
    OutputFinished { response_id: String },
    OutputInterrupted { response_id: Option<String> },
    WorkTrigger { trigger: WorkTrigger },
    ConfigurationApplied,
    InputMuteApplied {muted:bool,request_id:String},
    RelayRecovering {output_generation:u64},
    RelayRecovered {output_generation:u64},
    /// Acknowledging a control alone is not proof of a safe native media boundary.
    OutputBoundary { output_generation: u64, attachment_id: Option<String> },
    NativeAttachmentRequired { output_generation: u64 },
    ControlRejected { error: VoiceError },
    Error { error: VoiceError },
    Closed { termination: VoiceTermination },
}

#[cfg(test)]
mod tests {
    use super::*;
    fn local_replay_fixture()->VoiceLocalReplay{
        VoiceLocalReplay{scope:VoiceReplayScope{agent_session_id:"agent".into(),binding_version:2,context_floor:9},entries:vec![VoiceLocalReplayEntry::UserCommitted{origin:VoiceLocalFactRef{voice_session_id:"voice".into(),activation_epoch:3,journal_sequence:4},fragment:TranscriptFragment{speaker:VoiceSpeaker::User,fragment_id:"fragment".into(),revision:2,commit:TranscriptCommit::Committed,text:"already submitted".into(),media_range:None}}]}
    }
    #[test]
    fn local_replay_rejects_binding_floor_and_uncommitted_or_assistant_history(){
        let replay=local_replay_fixture();
        replay.validate_for(&replay.scope).unwrap();
        let mut scope=replay.scope.clone();scope.context_floor+=1;
        assert!(replay.validate_for(&scope).is_err());
        scope=replay.scope.clone();scope.binding_version+=1;
        assert!(replay.validate_for(&scope).is_err());
        scope=replay.scope.clone();scope.agent_session_id="other-agent".into();
        assert!(replay.validate_for(&scope).is_err());
        for assistant in [false,true]{
            let mut invalid=replay.clone();
            let VoiceLocalReplayEntry::UserCommitted{fragment,..}=&mut invalid.entries[0] else{unreachable!()};
            if assistant{fragment.speaker=VoiceSpeaker::Assistant;}else{fragment.commit=TranscriptCommit::Tentative;}
            assert!(invalid.validate_for(&invalid.scope).is_err());
        }
        let mut duplicate=replay.clone();duplicate.entries.push(duplicate.entries[0].clone());
        assert!(duplicate.validate_for(&duplicate.scope).is_err());
        let mut oversized=replay.clone();let VoiceLocalReplayEntry::UserCommitted{fragment,..}=&mut oversized.entries[0] else{unreachable!()};fragment.text="x".repeat(64*1024);
        assert!(oversized.validate_for(&oversized.scope).is_err());
    }
    #[test]
    fn local_replay_playback_requires_actual_consumption_without_word_or_work_claims(){
        let mut replay=local_replay_fixture();
        let origin=VoiceLocalFactRef{voice_session_id:"voice".into(),activation_epoch:3,journal_sequence:5};
        let receipt=PlaybackReceipt{activation_epoch:3,output_generation:1,segment_id:"segment".into(),revision:1,state:DeliveryState::Interrupted,consumed_us:20_000,uncertain_tail_us:10_000,precision:PlaybackPrecision::Unknown};
        replay.entries=vec![VoiceLocalReplayEntry::Playback{origin:origin.clone(),receipt:receipt.clone()}];
        replay.validate_for(&replay.scope).unwrap();
        for state in [DeliveryState::Generated,DeliveryState::Queued,DeliveryState::Revoked]{
            let mut invalid=receipt.clone();invalid.state=state;
            replay.entries=vec![VoiceLocalReplayEntry::Playback{origin:origin.clone(),receipt:invalid}];
            assert!(replay.validate_for(&replay.scope).is_err());
        }
        let mut invalid=receipt.clone();invalid.consumed_us=0;
        replay.entries=vec![VoiceLocalReplayEntry::Playback{origin:origin.clone(),receipt:invalid}];
        assert!(replay.validate_for(&replay.scope).is_err());
        let mut invalid=receipt;invalid.activation_epoch+=1;
        replay.entries=vec![VoiceLocalReplayEntry::Playback{origin,receipt:invalid}];
        assert!(replay.validate_for(&replay.scope).is_err());
        assert!(serde_json::from_value::<VoiceLocalReplay>(serde_json::json!({"scope":{"agent_session_id":"agent","binding_version":2,"context_floor":9},"entries":[],"canonical_receipt_id":"invented"})).is_err());
    }
    #[test] fn pcm_format_has_explicit_duration_and_channels() {
        assert_eq!(AudioFormat::pcm16(16_000, 1).duration_us(640).unwrap(), 20_000);
        assert_eq!(AudioFormat::pcm16(48_000, 2).duration_us(3840).unwrap(), 20_000);
        assert!(AudioFormat::pcm16(48_000, 2).duration_us(3).is_err());
    }
    #[test] fn unknown_does_not_admit_native_duplex() {
        let caps = native_duplex_requirements().into_iter().chain([VoiceFeature::TypedTools]).map(|f|
            (f, VoiceCapabilityEvidence { support: CapabilitySupport::Unknown, source: "unverified".into() })).collect();
        assert!(validate_native_duplex(&caps).is_err());
    }
    #[test] fn metadata_delegation_cannot_deserialize_as_tool() {
        let value = serde_json::json!({"kind":"delegation_trigger","upstream_trigger_id":"d1","target":"work","offset_ms":12,"context_window_ref":null});
        assert!(matches!(serde_json::from_value::<WorkTrigger>(value).unwrap(), WorkTrigger::DelegationTrigger { .. }));
    }
    #[test] fn keep_clear_set_are_distinct() {
        assert_eq!(serde_json::to_value(VoicePatch::<String>::Clear).unwrap(), serde_json::json!({"op":"clear"}));
        assert_ne!(serde_json::to_value(VoicePatch::<String>::Keep).unwrap(), serde_json::to_value(VoicePatch::<String>::Clear).unwrap());
    }
}
