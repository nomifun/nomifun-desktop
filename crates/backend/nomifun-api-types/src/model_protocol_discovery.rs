//! Setup-time protocol discovery. Chat verification may be billable; other
//! tasks use documentation/catalog evidence without submitting generation jobs.
use serde::{Deserialize, Serialize};
use crate::{BedrockConfig, ProviderConnectionInput, ProviderHealthCheckErrorKind, ProviderModelCapabilityInput};

fn default_apply_recommended_defaults() -> bool {true}

#[derive(Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct DiscoverModelProtocolsRequest {
    #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
    pub model: String,
    pub capability: ProviderModelCapabilityInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub preferred_protocol: Option<String>,
    /// Fill missing documented provider parameters for an untouched draft.
    /// False preserves deliberate user/persisted omissions; protocol conversion
    /// and connection validation still apply.
    #[serde(default = "default_apply_recommended_defaults")]
    #[ts(optional = nullable)]
    pub apply_recommended_defaults: bool,
}

#[derive(Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct DiscoverModelProtocolsAnonymousRequest {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub preferred_protocol: Option<String>,
    #[serde(default = "default_apply_recommended_defaults")]
    #[ts(optional = nullable)]
    pub apply_recommended_defaults: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "unknown")]
    pub bedrock_config: Option<BedrockConfig>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum ModelProtocolCandidateStatus { Available, Incompatible, Inconclusive, Blocked }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum ModelProtocolCandidateEvidence { VerifiedStream, OfficialModel, CatalogTask, RegisteredOnly }

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ModelProtocolCandidate {
    pub protocol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub capability: Option<ProviderModelCapabilityInput>,
    pub status: ModelProtocolCandidateStatus,
    pub evidence: ModelProtocolCandidateEvidence,
    pub recommended: bool,
    /// Request output budget used for this small setup check; the returned
    /// capability retains the user's configured model limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub probe_output_limit: Option<u32>,
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
pub struct DiscoverModelProtocolsResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub selected_protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub selected_capability: Option<ProviderModelCapabilityInput>,
    pub candidates: Vec<ModelProtocolCandidate>,
    #[ts(type = "number")]
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub config_revision: Option<i64>,
}
