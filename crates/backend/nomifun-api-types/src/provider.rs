use serde::{Deserialize, Serialize};

use crate::{ModelTask, ModelTrait};

/// Health status values for a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Unknown,
    Healthy,
    Unhealthy,
}

/// Coarse failure category for provider/model health checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealthCheckErrorKind {
    Timeout,
    InvalidAuthorizationHeader,
    Unauthorized,
    Forbidden,
    NotFound,
    InsufficientQuota,
    AwsCredentials,
    InvalidRequest,
    RateLimited,
    ConnectionError,
    ApiError,
    Unknown,
}

/// Request body for `POST /api/agents/provider-health-check`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct ProviderHealthCheckRequest {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
    pub model: String,
    /// Exact persisted capability to probe. There is no primary-task or chat
    /// fallback.
    pub task: ModelTask,
}

/// Response body for `POST /api/agents/provider-health-check`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ProviderHealthCheckResponse {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    pub platform: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
    pub model: String,
    pub task: ModelTask,
    pub status: HealthStatus,
    #[ts(type = "number")]
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error_kind: Option<ProviderHealthCheckErrorKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub http_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub timeout_stage: Option<String>,
}

/// AWS Bedrock authentication method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BedrockAuthMethod {
    #[serde(rename = "accessKey")]
    AccessKey,
    Profile,
    #[serde(rename = "defaultChain")]
    DefaultChain,
}

/// AWS Bedrock-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BedrockConfig {
    pub auth_method: BedrockAuthMethod,
    pub region: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

/// Provider response for `GET /api/providers` and single-provider endpoints.
///
/// Credentials are write-only and encrypted at rest. Responses expose only
/// whether configured credential material exists.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProviderResponse {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    pub platform: String,
    pub name: String,
    pub base_url: String,
    /// Explicit authentication transport for the provider's default connection.
    #[serde(deserialize_with = "crate::serde_util::deserialize_non_empty_string")]
    pub auth_scheme: String,
    pub has_credentials: bool,
    /// Authoritative model rows with their complete task capabilities.
    pub models: Vec<crate::provider_model::ProviderModelResponse>,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bedrock_config: Option<BedrockConfig>,
    pub sort_order: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Request body for `POST /api/providers`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateProviderRequest {
    /// Optional caller-supplied business ID. When `None`, the server generates one.
    /// This is a normal v3 create contract, not a historical-data migration hook.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::serde_util::deserialize_optional_provider_id"
    )]
    pub provider_id: Option<String>,
    pub platform: String,
    pub name: String,
    pub base_url: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_non_empty_string")]
    pub auth_scheme: String,
    /// Write-only typed credential material selected by `auth_scheme`.
    pub credentials: serde_json::Value,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bedrock_config: Option<BedrockConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<i64>,
    /// A provider is never created in an unusable half-state.
    pub initial_model: crate::provider_model::ProviderModelInput,
    /// Named connections required by the initial capability graph.
    #[serde(default)]
    pub connections: Vec<crate::provider_connection::ProviderConnectionInput>,
}

fn default_true() -> bool {
    true
}

/// Request body for `PUT /api/providers/:id`.
///
/// All fields are optional and use partial-update semantics.
///
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct UpdateProviderRequest {
    pub name: Option<String>,
    pub base_url: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::serde_util::deserialize_optional_non_empty_string"
    )]
    pub auth_scheme: Option<String>,
    /// Omitted keeps the encrypted credential payload unchanged.
    pub credentials: Option<serde_json::Value>,
    pub enabled: Option<bool>,
    pub bedrock_config: Option<BedrockConfig>,
    pub sort_order: Option<i64>,
}

/// Request body for `POST /api/providers/:id/clone`.
///
/// The body is optional on the wire: a missing/empty body clones with the
/// default `"{source name} copy"` name.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct CloneProviderRequest {
    /// Optional display name for the clone. A trimmed non-empty value wins;
    /// missing/blank falls back to `"{source name} copy"`.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub name: Option<String>,
}

/// Request body for `POST /api/providers/:id/models`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FetchModelsRequest {
    #[serde(default)]
    pub try_fix: bool,
}

/// Request body for `POST /api/providers/fetch-models` (anonymous, pre-create).
///
/// Used by the frontend's Add-Platform form to preview a provider's model
/// list before the provider row is persisted; credentials are passed in
/// the request body instead of looked up by id.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FetchModelsAnonymousRequest {
    pub platform: String,
    pub base_url: String,
    /// Explicit authentication transport for the proposed default connection.
    #[serde(deserialize_with = "crate::serde_util::deserialize_non_empty_string")]
    pub auth_scheme: String,
    /// Typed credential material for the proposed default connection.
    pub credentials: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bedrock_config: Option<BedrockConfig>,
    #[serde(default)]
    pub try_fix: bool,
}

/// A fetched model entry with one fixed wire shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<ModelTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub traits: Vec<ModelTrait>,
}

/// Response for `POST /api/providers/:id/models`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FetchModelsResponse {
    pub models: Vec<ModelInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_base_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PROVIDER_ID: &str = "018f1234-5678-7abc-8def-012345678990";

    fn initial_model() -> serde_json::Value {
        json!({
            "model":"gpt-5",
            "capabilities":[{
                "task":"chat",
                "protocol":"openai.chat_text",
                "connection_role":"default"
            }]
        })
    }

    #[test]
    fn provider_create_is_one_aggregate_shape() {
        let request: CreateProviderRequest = serde_json::from_value(json!({
            "provider_id": PROVIDER_ID,
            "platform":"openai",
            "name":"OpenAI",
            "base_url":"https://api.openai.com/v1",
            "auth_scheme":"bearer",
            "credentials":{"api_keys":["secret"]},
            "initial_model": initial_model()
        }))
        .unwrap();
        assert_eq!(request.initial_model.model, "gpt-5");
        assert!(
            serde_json::from_value::<CreateProviderRequest>(json!({
                "platform":"openai",
                "name":"OpenAI",
                "base_url":"https://api.openai.com/v1",
                "auth_scheme":"bearer",
                "initial_model": initial_model()
            }))
            .is_err(),
            "credential payload is explicit even when an auth scheme permits an empty object"
        );
        assert!(
            serde_json::from_value::<CreateProviderRequest>(json!({
                "platform":"openai",
                "name":"OpenAI",
                "base_url":"https://api.openai.com/v1",
                "auth_scheme":"bearer",
                "credentials":{"api_keys":["secret"]},
                "models":["gpt-5"],
                "initial_model": initial_model()
            }))
            .is_err()
        );
        for auth_scheme in [None, Some("   ")] {
            let mut value = json!({
                "platform":"openai",
                "name":"OpenAI",
                "base_url":"https://api.openai.com/v1",
                "credentials":{"api_keys":["secret"]},
                "initial_model": initial_model()
            });
            if let Some(auth_scheme) = auth_scheme {
                value["auth_scheme"] = json!(auth_scheme);
            }
            assert!(serde_json::from_value::<CreateProviderRequest>(value).is_err());
        }
    }

    #[test]
    fn fetched_model_suggestions_have_one_inline_task_trait_shape() {
        let response = FetchModelsResponse {
            models: vec![ModelInfo {
                id: "gpt-5".into(),
                name: None,
                tasks: vec![ModelTask::Chat],
                traits: vec![ModelTrait::VisionInput],
            }],
            fixed_base_url: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(
            value,
            json!({
                "models": [{
                    "id": "gpt-5",
                    "name": null,
                    "tasks": ["chat"],
                    "traits": ["vision_input"]
                }]
            })
        );
    }

    #[test]
    fn anonymous_fetch_requires_explicit_nonblank_auth_scheme() {
        assert!(
            serde_json::from_value::<FetchModelsAnonymousRequest>(json!({
                "platform":"openai",
                "base_url":"https://api.openai.com/v1",
                "credentials":{"api_keys":["secret"]}
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<FetchModelsAnonymousRequest>(json!({
                "platform":"openai",
                "base_url":"https://api.openai.com/v1",
                "auth_scheme":" ",
                "credentials":{"api_keys":["secret"]}
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<FetchModelsAnonymousRequest>(json!({
                "platform":"openai",
                "base_url":"https://api.openai.com/v1",
                "auth_scheme":"bearer"
            }))
            .is_err()
        );
        let request: FetchModelsAnonymousRequest = serde_json::from_value(json!({
            "platform":"gemini",
            "base_url":"https://generativelanguage.googleapis.com",
            "auth_scheme":" header_key:x-goog-api-key ",
            "credentials":{"api_keys":["secret"]}
        }))
        .unwrap();
        assert_eq!(request.auth_scheme, "header_key:x-goog-api-key");
    }

    #[test]
    fn provider_response_rejects_noncanonical_id() {
        let value = json!({
            "provider_id":"openai",
            "platform":"openai","name":"OpenAI","base_url":"https://api.openai.com/v1",
            "auth_scheme":"bearer","has_credentials":true,"models":[],"enabled":true,
            "sort_order":0,"created_at":1,"updated_at":2
        });
        assert!(serde_json::from_value::<ProviderResponse>(value).is_err());
    }

    #[test]
    fn provider_response_requires_the_authoritative_models_field() {
        let value = json!({
            "provider_id": PROVIDER_ID,
            "platform":"openai","name":"OpenAI","base_url":"https://api.openai.com/v1",
            "auth_scheme":"bearer","has_credentials":true,"enabled":true,
            "sort_order":0,"created_at":1,"updated_at":2
        });
        assert!(serde_json::from_value::<ProviderResponse>(value).is_err());

        let mut old_parallel_shape = json!({
            "provider_id": PROVIDER_ID,
            "platform":"openai","name":"OpenAI","base_url":"https://api.openai.com/v1",
            "auth_scheme":"bearer","has_credentials":true,"models":[],"enabled":true,
            "sort_order":0,"created_at":1,"updated_at":2
        });
        old_parallel_shape["model_names"] = json!(["gpt-5"]);
        assert!(serde_json::from_value::<ProviderResponse>(old_parallel_shape).is_err());
    }
}
