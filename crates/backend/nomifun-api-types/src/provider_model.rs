//! Wire DTOs for provider models and their task-scoped invocation
//! capabilities.
//!
//! A provider model stores only identity and display metadata. Every usable
//! modality is represented by exactly one capability keyed by
//! `(provider_id, model, task)`. Transport, connection, provider parameters,
//! and health therefore have one owner and cannot drift between model-level
//! columns and nested compatibility JSON.

use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::model_task::{ModelTask, ModelTrait};
use crate::provider::{HealthStatus, ProviderHealthCheckErrorKind};

fn empty_object() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

fn default_false() -> bool {
    false
}

pub(crate) fn default_true() -> bool {
    true
}

/// Validate the one canonical wire/runtime invariant for capability traits.
///
/// Persisted rows and incoming DTOs deliberately share this rule so saving a
/// model cannot produce a capability that the runtime later rejects.
pub fn validate_model_traits_unique(traits: &[ModelTrait]) -> Result<(), &'static str> {
    let mut unique = HashSet::with_capacity(traits.len());
    if traits.iter().copied().all(|model_trait| unique.insert(model_trait)) {
        Ok(())
    } else {
        Err("capability traits must not contain duplicates")
    }
}

fn deserialize_unique_traits<'de, D>(deserializer: D) -> Result<Vec<ModelTrait>, D::Error>
where
    D: Deserializer<'de>,
{
    let traits = Vec::<ModelTrait>::deserialize(deserializer)?;
    validate_model_traits_unique(&traits).map_err(D::Error::custom)?;
    Ok(traits)
}

fn deserialize_non_empty_capabilities<'de, D>(
    deserializer: D,
) -> Result<Vec<ProviderModelCapabilityInput>, D::Error>
where
    D: Deserializer<'de>,
{
    let capabilities = Vec::<ProviderModelCapabilityInput>::deserialize(deserializer)?;
    if capabilities.is_empty() {
        return Err(D::Error::custom(
            "provider model must declare at least one capability",
        ));
    }

    let mut tasks = HashSet::with_capacity(capabilities.len());
    for capability in &capabilities {
        if !tasks.insert(capability.task) {
            return Err(D::Error::custom(format!(
                "provider model capability task '{}' is duplicated",
                serde_json::to_value(capability.task)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "unknown".to_owned())
            )));
        }
    }
    Ok(capabilities)
}

/// Complete user-authored configuration for one model modality.
///
/// `protocol` and `connection_role` are intentionally required and nonblank.
/// Even the default provider connection is represented explicitly as
/// `connection_role = "default"`; there is no implicit or legacy fallback.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct ProviderModelCapabilityInput {
    pub task: ModelTask,
    #[serde(default, deserialize_with = "deserialize_unique_traits")]
    #[ts(optional = nullable)]
    pub traits: Vec<ModelTrait>,
    #[serde(deserialize_with = "crate::serde_util::deserialize_non_empty_string")]
    pub protocol: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_non_empty_string")]
    pub connection_role: String,
    #[serde(
        default,
        deserialize_with = "crate::serde_util::deserialize_optional_non_empty_string"
    )]
    #[ts(optional)]
    pub base_url_override: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::serde_util::deserialize_optional_non_empty_string"
    )]
    #[ts(optional)]
    pub endpoint: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::serde_util::deserialize_optional_non_empty_string"
    )]
    #[ts(optional)]
    pub poll_endpoint: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::serde_util::deserialize_optional_non_empty_string"
    )]
    #[ts(optional)]
    pub content_endpoint: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::serde_util::deserialize_optional_non_empty_string"
    )]
    #[ts(optional)]
    pub realtime_endpoint: Option<String>,
    #[serde(default = "default_false")]
    #[ts(optional = nullable)]
    pub allow_cross_origin_credentials: bool,
    #[serde(default = "empty_object")]
    #[ts(optional = nullable, type = "unknown")]
    pub provider_params: serde_json::Value,
    #[serde(default)]
    #[ts(optional, type = "number")]
    pub context_limit: Option<i64>,
    #[serde(default)]
    #[ts(optional, type = "number")]
    pub output_limit: Option<i64>,
}

/// Latest health observation for one task-scoped capability.
///
/// The diagnostic fields are all optional so a row written by an older build
/// (`status`/`latency`/`error` only) still deserializes under
/// `deny_unknown_fields`; no migration is needed because the column is opaque
/// JSON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct CapabilityHealth {
    pub status: HealthStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub latency: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    /// Why it failed, as a machine-readable category. Persisting only `error`
    /// meant a 404 and a 401 were indistinguishable once the check was over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_kind: Option<ProviderHealthCheckErrorKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub http_status: Option<u16>,
    /// The URL that was requested, with query material redacted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub attempted_url: Option<String>,
}

/// Persisted task-scoped capability returned with its owning model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct ProviderModelCapabilityResponse {
    pub task: ModelTask,
    pub traits: Vec<ModelTrait>,
    pub protocol: String,
    pub connection_role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub base_url_override: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub poll_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub content_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub realtime_endpoint: Option<String>,
    pub allow_cross_origin_credentials: bool,
    #[ts(type = "unknown")]
    pub provider_params: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub context_limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub output_limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub health: Option<CapabilityHealth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub health_checked_at: Option<i64>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

/// One provider model with all of its usable task capabilities nested in
/// deterministic task order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct ProviderModelResponse {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    pub model: String,
    pub enabled: bool,
    #[ts(type = "number")]
    pub sort_order: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub description: Option<String>,
    pub capabilities: Vec<ProviderModelCapabilityResponse>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

/// Complete provider-model input reused by aggregate provider creation and
/// the standalone full-save endpoint. It deliberately has no provider id.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct ProviderModelInput {
    #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
    pub model: String,
    #[serde(default = "default_true")]
    #[ts(optional = nullable)]
    pub enabled: bool,
    #[serde(default)]
    #[ts(optional)]
    pub description: Option<String>,
    #[serde(default)]
    #[ts(optional, type = "number")]
    pub sort_order: Option<i64>,
    #[serde(deserialize_with = "deserialize_non_empty_capabilities")]
    pub capabilities: Vec<ProviderModelCapabilityInput>,
}

/// Upsert a provider model and replace its complete capability set atomically.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct SaveProviderModelRequest {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    pub model: ProviderModelInput,
}

/// Body identifying one model by its composite natural key.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct ProviderModelKeyRequest {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
    pub model: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PROVIDER_ID: &str = "018f1234-5678-7abc-8def-012345678990";

    fn capability() -> serde_json::Value {
        json!({
            "task": "speech_synthesis",
            "traits": ["audio_output", "streaming"],
            "protocol": "stepfun.audio_speech",
            "connection_role": "default",
            "endpoint": "/audio/speech",
            "provider_params": {"voice": "cixingnansheng"}
        })
    }

    #[test]
    fn create_requires_one_unique_capability_with_explicit_transport() {
        for capabilities in [json!([]), json!([capability(), capability()])] {
            let value = json!({
                "model": "step-tts-mini",
                "capabilities": capabilities,
            });
            assert!(serde_json::from_value::<ProviderModelInput>(value).is_err());
        }

        for invalid in [
            json!({"task":"chat","protocol":"","connection_role":"default"}),
            json!({"task":"chat","protocol":"openai.chat_text","connection_role":" "}),
        ] {
            let value = json!({
                "model": "custom-model",
                "capabilities": [invalid],
            });
            assert!(serde_json::from_value::<ProviderModelInput>(value).is_err());
        }

        let request: ProviderModelInput = serde_json::from_value(json!({
            "model": "step-tts-mini",
            "capabilities": [capability()],
        }))
        .unwrap();
        assert!(request.enabled);
        assert_eq!(request.capabilities.len(), 1);
        assert_eq!(request.capabilities[0].connection_role, "default");
        assert_eq!(
            request.capabilities[0].provider_params,
            json!({"voice":"cixingnansheng"})
        );
    }

    #[test]
    fn capability_traits_are_unique_at_the_wire_boundary() {
        let duplicate = json!({
            "model": "duplicate-traits",
            "capabilities": [{
                "task": "chat",
                "traits": ["streaming", "streaming"],
                "protocol": "openai.chat_text",
                "connection_role": "default"
            }]
        });
        let error = serde_json::from_value::<ProviderModelInput>(duplicate).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("capability traits must not contain duplicates")
        );

        let valid: ProviderModelInput = serde_json::from_value(json!({
            "model": "unique-traits",
            "capabilities": [{
                "task": "chat",
                "traits": ["streaming", "function_calling"],
                "protocol": "openai.chat_text",
                "connection_role": "default"
            }]
        }))
        .unwrap();
        assert_eq!(
            valid.capabilities[0].traits,
            vec![ModelTrait::Streaming, ModelTrait::FunctionCalling]
        );
    }

    #[test]
    fn save_is_full_capability_replacement() {
        assert!(
            serde_json::from_value::<SaveProviderModelRequest>(json!({
                "provider_id": PROVIDER_ID,
                "model": {"model": "model-without-capabilities"}
            }))
            .is_err()
        );

        let request: SaveProviderModelRequest = serde_json::from_value(json!({
            "provider_id": PROVIDER_ID,
            "model": {
                "model": "step-tts-mini",
                "description": null,
                "capabilities": [capability()]
            }
        }))
        .unwrap();
        assert_eq!(request.model.description, None);
        assert_eq!(request.model.capabilities.len(), 1);
    }

    #[test]
    fn provider_params_default_to_an_object_and_unknown_fields_fail() {
        let request: ProviderModelInput = serde_json::from_value(json!({
            "model": "custom-model",
            "capabilities": [{
                "task": "chat",
                "protocol": "openai.chat_text",
                "connection_role": "default"
            }]
        }))
        .unwrap();
        assert_eq!(request.capabilities[0].provider_params, json!({}));
        assert!(
            serde_json::from_value::<ProviderModelInput>(json!({
                "model": "custom-model",
                "capabilities": [{
                    "task": "chat",
                    "protocol": "openai.chat_text",
                    "connection_role": "default",
                    "params": {}
                }]
            }))
            .is_err()
        );
    }

    #[test]
    fn model_response_requires_its_capability_collection() {
        assert!(
            serde_json::from_value::<ProviderModelResponse>(json!({
                "provider_id": PROVIDER_ID,
                "model": "custom-model",
                "enabled": true,
                "sort_order": 0,
                "created_at": 1,
                "updated_at": 1
            }))
            .is_err()
        );
    }
}
