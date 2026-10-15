//! Explicit setup-time, potentially billable Chat protocol verification.
//! This returns a tested draft and never persists a provider, model or Session.
use serde::{Deserialize, Serialize};
use crate::{ProviderConnectionInput, ProviderHealthCheckErrorKind, ProviderModelCapabilityInput};

#[derive(Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct DetectChatProtocolRequest {
    #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
    pub model: String,
    pub capability: ProviderModelCapabilityInput,
}

#[derive(Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct DetectChatProtocolAnonymousRequest {
    pub platform: String,
    pub base_url: String,
    pub auth_scheme: String,
    #[ts(type = "unknown")]
    pub credentials: serde_json::Value,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub connections: Vec<ProviderConnectionInput>,
    #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
    pub model: String,
    pub capability: ProviderModelCapabilityInput,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum ChatProtocolDetectionStatus { Verified, Incompatible, Failed }

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ChatProtocolDetectionAttempt {
    pub protocol: String,
    pub status: ChatProtocolDetectionStatus,
    /// Request output budget used for this small setup check; the returned
    /// capability retains the user's configured model limit.
    pub probe_output_limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_kind: Option<ProviderHealthCheckErrorKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub http_status: Option<u16>,
    #[ts(type = "number")]
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct DetectChatProtocolResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub selected_protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub selected_capability: Option<ProviderModelCapabilityInput>,
    pub attempts: Vec<ChatProtocolDetectionAttempt>,
    #[ts(type = "number")]
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub config_revision: Option<i64>,
}
