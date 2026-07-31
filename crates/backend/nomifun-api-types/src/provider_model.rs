//! Wire DTO for a single `provider_models` row — the authoritative per-model
//! entity exposed on `ProviderResponse::models_detail` — plus the request
//! bodies for the row-level `/api/provider-models` CRUD surface.

use serde::{Deserialize, Serialize};

use crate::model_task::{ModelTask, ModelTrait, ProfileSource};
use crate::provider::ModelHealthStatus;

/// One authoritative per-model catalog entry, projected from a
/// `provider_models` row. Identity is `(provider_id, model)`.
//
// ts-rs annotations mirror the serde wire truth: `skip_serializing_if`
// optionals emit `x?: T`; i64 timestamps/counters emit `number` (this API
// serializes plain JSON numbers, not bigints); opaque JSON emits `unknown`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ProviderModelResponse {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    pub model: String,
    pub enabled: bool,
    #[ts(type = "number")]
    pub sort_order: i64,
    pub tasks: Vec<ModelTask>,
    pub traits: Vec<ModelTrait>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub connection_role: Option<String>,
    #[serde(default)]
    #[ts(type = "unknown")]
    pub params: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub context_limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub description: Option<String>,
    #[serde(default)]
    pub source: ProfileSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub health: Option<ModelHealthStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub health_checked_at: Option<i64>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

/// Body for `POST /api/provider-models` — create one catalog row.
///
/// `tasks` left empty means "no explicit profile": the service seeds the
/// heuristic profile ([`crate::derive_tasks_and_traits`]) with
/// `source = inferred`; a non-empty `tasks` is an explicit user profile
/// (`source = user`).
// Request DTO: every `#[serde(default)]` field may be omitted by the client,
// so the binding marks it `?`. Plain `#[ts(optional)]` unwraps the Option
// (null ≡ absent ≡ unset here, so the binding does not advertise `| null`);
// non-Option defaulted fields use `optional = nullable`, the no-unwrap form
// (their TS type carries no null — it only adds the `?`).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct CreateProviderModelRequest {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
    pub model: String,
    #[serde(default = "crate::provider_model::default_true")]
    #[ts(optional = nullable)]
    pub enabled: bool,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub tasks: Vec<ModelTask>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub traits: Vec<ModelTrait>,
    #[serde(default)]
    #[ts(optional)]
    pub protocol: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub connection_role: Option<String>,
    #[serde(default)]
    #[ts(optional, type = "unknown")]
    pub params: Option<serde_json::Value>,
    #[serde(default)]
    #[ts(optional, type = "number")]
    pub context_limit: Option<i64>,
    #[serde(default)]
    #[ts(optional)]
    pub description: Option<String>,
    #[serde(default)]
    #[ts(optional, type = "number")]
    pub sort_order: Option<i64>,
}

pub(crate) fn default_true() -> bool {
    true
}

/// Body for `POST /api/provider-models/update` — partial update of one row.
///
/// Nullable columns use double-Option: field absent = keep, `null` = clear,
/// value = set.
// Request DTO with tri-state (double-Option) nullable columns: those emit
// `x?: T | null` — absent = keep, null = clear, value = set (plain
// `#[ts(optional)]` unwraps ONE Option level, leaving the inner `Option<T>`'s
// `T | null`). Non-tri-state fields emit plain `x?: T` — null there would
// just mean "keep", same as omitting the field, so the binding deliberately
// reserves `| null` for the fields where null CLEARS.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct UpdateProviderModelRequest {
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_model_name")]
    pub model: String,
    #[serde(default)]
    #[ts(optional)]
    pub enabled: Option<bool>,
    #[serde(default)]
    #[ts(optional, type = "number")]
    pub sort_order: Option<i64>,
    #[serde(default)]
    #[ts(optional)]
    pub tasks: Option<Vec<ModelTask>>,
    #[serde(default)]
    #[ts(optional)]
    pub traits: Option<Vec<ModelTrait>>,
    #[serde(
        default,
        deserialize_with = "crate::serde_util::deserialize_double_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(optional)]
    pub protocol: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "crate::serde_util::deserialize_double_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(optional)]
    pub connection_role: Option<Option<String>>,
    #[serde(default)]
    #[ts(optional, type = "unknown")]
    pub params: Option<serde_json::Value>,
    #[serde(
        default,
        deserialize_with = "crate::serde_util::deserialize_double_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(optional, type = "number | null")]
    pub context_limit: Option<Option<i64>>,
    #[serde(
        default,
        deserialize_with = "crate::serde_util::deserialize_double_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(optional)]
    pub description: Option<Option<String>>,
}

/// Body identifying one row by its composite natural key
/// (`POST /api/provider-models/delete`).
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
    use crate::provider::HealthStatus;
    use serde_json::json;

    const PROVIDER_ID: &str = "018f1234-5678-7abc-8def-012345678990";

    fn minimal() -> ProviderModelResponse {
        ProviderModelResponse {
            provider_id: PROVIDER_ID.into(),
            model: "gpt-4o".into(),
            enabled: true,
            sort_order: 0,
            tasks: vec![],
            traits: vec![],
            protocol: None,
            connection_role: None,
            params: serde_json::Value::Null,
            context_limit: None,
            description: None,
            source: ProfileSource::Inferred,
            health: None,
            health_checked_at: None,
            created_at: 1,
            updated_at: 2,
        }
    }

    #[test]
    fn minimal_serialization_skips_absent_optionals() {
        let json = serde_json::to_value(minimal()).unwrap();
        assert_eq!(json["provider_id"], PROVIDER_ID);
        assert_eq!(json["model"], "gpt-4o");
        assert_eq!(json["enabled"], true);
        assert_eq!(json["sort_order"], 0);
        assert_eq!(json["tasks"], json!([]));
        assert_eq!(json["traits"], json!([]));
        assert_eq!(json["params"], serde_json::Value::Null);
        assert_eq!(json["source"], "inferred");
        assert_eq!(json["created_at"], 1);
        assert_eq!(json["updated_at"], 2);
        for absent in [
            "protocol",
            "connection_role",
            "context_limit",
            "description",
            "health",
            "health_checked_at",
        ] {
            assert!(json.get(absent).is_none(), "{absent} must be skipped");
        }
    }

    #[test]
    fn full_roundtrip() {
        let full = ProviderModelResponse {
            tasks: vec![ModelTask::Chat],
            traits: vec![ModelTrait::VisionInput],
            protocol: Some("openai".into()),
            connection_role: Some("primary".into()),
            params: json!({"temperature": 0.7}),
            context_limit: Some(128_000),
            description: Some("general model".into()),
            source: ProfileSource::User,
            health: Some(ModelHealthStatus {
                status: HealthStatus::Healthy,
                last_check: Some(1712345678000),
                latency: Some(320),
                error: None,
            }),
            health_checked_at: Some(1712345678000),
            ..minimal()
        };
        let json = serde_json::to_value(&full).unwrap();
        assert_eq!(json["tasks"], json!(["chat"]));
        assert_eq!(json["traits"], json!(["vision_input"]));
        assert_eq!(json["protocol"], "openai");
        assert_eq!(json["connection_role"], "primary");
        assert_eq!(json["params"]["temperature"], 0.7);
        assert_eq!(json["context_limit"], 128_000);
        assert_eq!(json["source"], "user");
        assert_eq!(json["health"]["status"], "healthy");
        assert_eq!(json["health_checked_at"], 1712345678000_i64);
        let parsed: ProviderModelResponse = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, full);
    }

    #[test]
    fn deserialize_defaults_params_and_source() {
        let raw = json!({
            "provider_id": PROVIDER_ID,
            "model": "gpt-4o",
            "enabled": true,
            "sort_order": 3,
            "tasks": ["chat"],
            "traits": [],
            "created_at": 1,
            "updated_at": 2
        });
        let parsed: ProviderModelResponse = serde_json::from_value(raw).unwrap();
        assert_eq!(parsed.params, serde_json::Value::Null);
        assert_eq!(parsed.source, ProfileSource::Inferred);
        assert_eq!(parsed.sort_order, 3);
    }

    #[test]
    fn rejects_noncanonical_provider_id() {
        for provider_id in [
            json!("openai"),
            json!("550e8400-e29b-41d4-a716-446655440000"),
            json!("0190F5FE-7C00-7A00-8000-000000000042"),
        ] {
            let raw = json!({
                "provider_id": provider_id,
                "model": "gpt-4o",
                "enabled": true,
                "sort_order": 0,
                "tasks": [],
                "traits": [],
                "created_at": 1,
                "updated_at": 2
            });
            assert!(serde_json::from_value::<ProviderModelResponse>(raw).is_err());
        }
    }

    // --- CreateProviderModelRequest ---

    #[test]
    fn create_request_minimal_defaults() {
        let req: CreateProviderModelRequest = serde_json::from_value(json!({
            "provider_id": PROVIDER_ID,
            "model": "gpt-4o"
        }))
        .unwrap();
        assert!(req.enabled, "enabled defaults to true");
        assert!(req.tasks.is_empty());
        assert!(req.traits.is_empty());
        assert_eq!(req.protocol, None);
        assert_eq!(req.connection_role, None);
        assert_eq!(req.params, None);
        assert_eq!(req.context_limit, None);
        assert_eq!(req.description, None);
        assert_eq!(req.sort_order, None);
    }

    #[test]
    fn create_request_rejects_unknown_fields_and_bad_keys() {
        assert!(serde_json::from_value::<CreateProviderModelRequest>(json!({
            "provider_id": PROVIDER_ID,
            "model": "gpt-4o",
            "bogus": 1
        }))
        .is_err());
        assert!(serde_json::from_value::<CreateProviderModelRequest>(json!({
            "provider_id": "openai",
            "model": "gpt-4o"
        }))
        .is_err());
        assert!(serde_json::from_value::<CreateProviderModelRequest>(json!({
            "provider_id": PROVIDER_ID,
            "model": " padded "
        }))
        .is_err());
    }

    // --- UpdateProviderModelRequest (double-Option tri-state) ---

    #[test]
    fn update_request_absent_nullables_mean_keep() {
        let req: UpdateProviderModelRequest = serde_json::from_value(json!({
            "provider_id": PROVIDER_ID,
            "model": "gpt-4o"
        }))
        .unwrap();
        assert_eq!(req.protocol, None);
        assert_eq!(req.connection_role, None);
        assert_eq!(req.context_limit, None);
        assert_eq!(req.description, None);
        assert_eq!(req.enabled, None);
        assert_eq!(req.tasks, None);
        assert_eq!(req.traits, None);
        assert_eq!(req.params, None);
        assert_eq!(req.sort_order, None);
    }

    #[test]
    fn update_request_null_nullables_mean_clear() {
        let req: UpdateProviderModelRequest = serde_json::from_value(json!({
            "provider_id": PROVIDER_ID,
            "model": "gpt-4o",
            "protocol": null,
            "connection_role": null,
            "context_limit": null,
            "description": null
        }))
        .unwrap();
        assert_eq!(req.protocol, Some(None));
        assert_eq!(req.connection_role, Some(None));
        assert_eq!(req.context_limit, Some(None));
        assert_eq!(req.description, Some(None));
    }

    #[test]
    fn update_request_values_mean_set() {
        let req: UpdateProviderModelRequest = serde_json::from_value(json!({
            "provider_id": PROVIDER_ID,
            "model": "gpt-4o",
            "enabled": false,
            "sort_order": 9,
            "tasks": ["chat"],
            "traits": ["vision_input"],
            "protocol": "openai",
            "connection_role": "primary",
            "params": {"temperature": 0.1},
            "context_limit": 200000,
            "description": "desc"
        }))
        .unwrap();
        assert_eq!(req.enabled, Some(false));
        assert_eq!(req.sort_order, Some(9));
        assert_eq!(req.tasks, Some(vec![ModelTask::Chat]));
        assert_eq!(req.traits, Some(vec![ModelTrait::VisionInput]));
        assert_eq!(req.protocol, Some(Some("openai".into())));
        assert_eq!(req.connection_role, Some(Some("primary".into())));
        assert_eq!(req.context_limit, Some(Some(200_000)));
        assert_eq!(req.description, Some(Some("desc".into())));
    }

    #[test]
    fn update_request_rejects_unknown_fields() {
        assert!(serde_json::from_value::<UpdateProviderModelRequest>(json!({
            "provider_id": PROVIDER_ID,
            "model": "gpt-4o",
            "healthy": true
        }))
        .is_err());
    }

    // --- ProviderModelKeyRequest ---

    #[test]
    fn key_request_roundtrip_and_strictness() {
        let req: ProviderModelKeyRequest = serde_json::from_value(json!({
            "provider_id": PROVIDER_ID,
            "model": "gpt-4o"
        }))
        .unwrap();
        assert_eq!(req.provider_id, PROVIDER_ID);
        assert_eq!(req.model, "gpt-4o");
        assert!(serde_json::from_value::<ProviderModelKeyRequest>(json!({
            "provider_id": PROVIDER_ID,
            "model": "gpt-4o",
            "extra": true
        }))
        .is_err());
        assert!(serde_json::from_value::<ProviderModelKeyRequest>(json!({
            "provider_id": PROVIDER_ID,
            "model": ""
        }))
        .is_err());
    }
}
