use serde::{Deserialize,Serialize};
use schemars::JsonSchema;
use crate::voice::*;

#[derive(Clone,Debug,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceActivationRequest {
    pub agent_session_id:String,
    pub binding_version:u64,
    pub profile_id:String,
    pub profile_revision:u64,
    pub endpoint_id:String,
    pub transport:VoiceTransportPreference,
    #[serde(default)]pub native_offer:Option<String>,
    #[serde(default)]pub takeover:bool,
}
#[derive(Clone,Debug,Serialize,Deserialize,JsonSchema)]
pub struct VoiceActivationResponse {
    pub voice_session_id:String,
    pub activation_epoch:u64,
    /// Short-lived, single-use product media capability. Never a vendor key.
    pub attachment_token:String,
    pub state:VoiceState,
    pub negotiation:VoiceNegotiation,
}
#[derive(Clone,Debug,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceWorkCommand {
    pub activation_epoch:u64,
    pub operation_key:String,
    pub request:VoiceWorkRequest,
}
/// Identity of an actual committed local utterance, separate from a model call.
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceInputSourceRef {pub fragment_id:String,pub revision:u64}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceContextPresentation {pub context_key:String,pub target:Option<crate::WorkTarget>}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceSourceContextRequirement {pub source_ref:VoiceInputSourceRef,pub context:VoiceContextPresentation}
/// Only the authenticated human-facing route may supply this confirmation.
#[derive(Clone,Debug,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceSourceContextConfirmation {pub activation_epoch:u64,pub source_ref:VoiceInputSourceRef,pub context_key:String}
#[derive(Clone,Debug,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceProfileProbeRequest{
    pub agent_session_id:String,pub binding_version:u64,pub profile_revision:u64,
    #[serde(default)]pub native_offer:Option<String>,
}
#[derive(Clone,Debug,Serialize,Deserialize,JsonSchema)]
pub struct VoiceProfileProbeResult{
    pub profile_id:String,pub profile_revision:u64,pub transport:VoiceTransportPreference,
    pub negotiation:VoiceNegotiation,pub termination:VoiceTermination,
    /// True only for actual connection/open/ACK and local shutdown. It is not
    /// a device, playback, latency, duplex semantics, or account-wide claim.
    pub connection_verified:bool,
}
