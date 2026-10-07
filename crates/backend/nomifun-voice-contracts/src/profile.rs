use serde::{Deserialize,Serialize};
use schemars::JsonSchema;
use serde_json::Value;
use crate::{AgentSessionId,VoiceRouteRecord,VoiceTransportPreference,WorkSteeringPolicy};

/// Voice selection is a separate overlay; it never edits the work Snapshot.
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceProfile {
    pub profile_id:String,
    pub revision:u64,
    pub agent_session_id:AgentSessionId,
    pub binding_version:u64,
    pub enabled:bool,
    pub label:String,
    pub route:VoiceRouteRecord,
    #[serde(default)]pub work_steering_policy:WorkSteeringPolicy,
}
#[derive(Clone,Debug,Serialize,Deserialize,JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoiceProfileUpdate {
    pub expected_revision:u64,
    pub agent_session_id:AgentSessionId,
    pub binding_version:u64,
    pub enabled:bool,
    pub provider_id:String,
    pub model:String,
    #[serde(default)]pub adapter_id:Option<String>,
    #[serde(default="default_role")]pub connection_role:String,
    pub transport:VoiceTransportPreference,
    pub adapter_config:Value,
    #[serde(default)]pub work_steering_policy:WorkSteeringPolicy,
}
fn default_role()->String{"default".into()}
