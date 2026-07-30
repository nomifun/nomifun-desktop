//! Wire DTOs for non-default per-role provider connection profiles.
//!
//! The providers row itself remains the implicit `default` connection; these
//! DTOs cover the extra `(provider_id, role)` connection rows. Credentials are
//! write-only: requests may carry them, responses never echo them back.

use serde::{Deserialize, Serialize};

/// Response never echoes credentials back; `has_credentials` signals presence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ProviderConnectionResponse {
    pub connection_id: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_provider_id")]
    pub provider_id: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub label: Option<String>,
    pub base_url: String,
    pub auth_scheme: String,
    pub has_credentials: bool,
    #[serde(default)]
    pub is_full_url: bool,
    #[serde(default)]
    #[ts(type = "unknown")]
    pub extra: serde_json::Value,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

// Request DTO: `#[serde(default)]` fields may be omitted by the client → `?`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct UpsertProviderConnectionRequest {
    pub role: String,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub label: Option<String>,
    pub base_url: String,
    #[serde(default = "default_bearer")]
    #[ts(optional = nullable)]
    pub auth_scheme: String,
    /// Write-only structured credentials (shape depends on auth_scheme),
    /// encrypted at rest. `None` on update keeps the stored credentials.
    #[serde(default)]
    #[ts(optional = nullable, type = "unknown")]
    pub credentials: Option<serde_json::Value>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub is_full_url: bool,
    #[serde(default)]
    #[ts(optional = nullable, type = "unknown")]
    pub extra: Option<serde_json::Value>,
}
fn default_bearer() -> String { "bearer".into() }

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PROVIDER_ID: &str = "018f1234-5678-7abc-8def-012345678990";

    fn sample_response() -> ProviderConnectionResponse {
        ProviderConnectionResponse {
            connection_id: "018f1234-5678-7abc-8def-012345678991".into(),
            provider_id: PROVIDER_ID.into(),
            role: "voice".into(),
            label: None,
            base_url: "https://voice.example.com/v1".into(),
            auth_scheme: "bearer".into(),
            has_credentials: true,
            is_full_url: false,
            extra: json!({}),
            created_at: 1,
            updated_at: 2,
        }
    }

    #[test]
    fn response_never_serializes_credentials() {
        let value = serde_json::to_value(sample_response()).unwrap();
        let map = value.as_object().unwrap();
        assert!(map.get("credentials").is_none());
        assert!(map.get("credentials_encrypted").is_none());
        assert_eq!(value["has_credentials"], true);
        assert_eq!(value["role"], "voice");
        assert_eq!(value["provider_id"], PROVIDER_ID);
    }

    #[test]
    fn response_skips_none_label_and_round_trips() {
        let response = sample_response();
        let value = serde_json::to_value(&response).unwrap();
        assert!(value.as_object().unwrap().get("label").is_none());
        let parsed: ProviderConnectionResponse = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, response);
    }

    #[test]
    fn response_deserialize_defaults_is_full_url_and_extra() {
        let parsed: ProviderConnectionResponse = serde_json::from_value(json!({
            "connection_id": "018f1234-5678-7abc-8def-012345678991",
            "provider_id": PROVIDER_ID,
            "role": "voice",
            "base_url": "https://voice.example.com/v1",
            "auth_scheme": "bearer",
            "has_credentials": false,
            "created_at": 1,
            "updated_at": 2
        }))
        .unwrap();
        assert!(!parsed.is_full_url);
        assert_eq!(parsed.extra, serde_json::Value::Null);
        assert!(!parsed.has_credentials);
    }

    #[test]
    fn response_rejects_non_canonical_provider_id() {
        let result = serde_json::from_value::<ProviderConnectionResponse>(json!({
            "connection_id": "018f1234-5678-7abc-8def-012345678991",
            "provider_id": "not-a-provider-id",
            "role": "voice",
            "base_url": "https://voice.example.com/v1",
            "auth_scheme": "bearer",
            "has_credentials": false,
            "created_at": 1,
            "updated_at": 2
        }));
        assert!(result.is_err());
    }

    #[test]
    fn request_minimal_applies_defaults() {
        let parsed: UpsertProviderConnectionRequest = serde_json::from_value(json!({
            "role": "voice",
            "base_url": "https://voice.example.com/v1"
        }))
        .unwrap();
        assert_eq!(parsed.role, "voice");
        assert_eq!(parsed.label, None);
        assert_eq!(parsed.auth_scheme, "bearer");
        assert!(parsed.credentials.is_none());
        assert!(!parsed.is_full_url);
        assert!(parsed.extra.is_none());
    }

    #[test]
    fn request_carries_write_only_credentials() {
        let parsed: UpsertProviderConnectionRequest = serde_json::from_value(json!({
            "role": "voice",
            "base_url": "https://voice.example.com/v1",
            "auth_scheme": "api_key",
            "credentials": { "api_key": "sk-live-1234" },
            "is_full_url": true,
            "extra": { "region": "eu" }
        }))
        .unwrap();
        assert_eq!(parsed.auth_scheme, "api_key");
        assert_eq!(parsed.credentials, Some(json!({ "api_key": "sk-live-1234" })));
        assert!(parsed.is_full_url);
        assert_eq!(parsed.extra, Some(json!({ "region": "eu" })));
    }

    #[test]
    fn request_rejects_unknown_fields() {
        let result = serde_json::from_value::<UpsertProviderConnectionRequest>(json!({
            "role": "voice",
            "base_url": "https://voice.example.com/v1",
            "credentials_encrypted": "sneaky-ciphertext"
        }));
        assert!(result.is_err(), "deny_unknown_fields must reject stray fields");
    }
}
