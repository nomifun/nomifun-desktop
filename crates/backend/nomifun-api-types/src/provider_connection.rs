//! Wire DTOs for non-default per-role provider connection profiles.
//!
//! The providers row itself is the explicit `default` connection; these
//! DTOs cover the extra `(provider_id, role)` connection rows. Credentials are
//! write-only: requests may carry them, responses never echo them back.

use serde::{Deserialize, Serialize};

/// Response never echoes credentials back; `has_credentials` signals presence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct ProviderConnectionResponse {
    #[serde(deserialize_with = "crate::serde_util::deserialize_uuidv7")]
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
    #[ts(type = "unknown")]
    pub extra: serde_json::Value,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

/// Named connection input nested in aggregate provider creation. Every child
/// row is new, so its typed credential payload is explicit. Authentication
/// schemes such as SDK default-chain may use an empty object, but the field is
/// never omitted.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct ProviderConnectionInput {
    pub role: String,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub label: Option<String>,
    pub base_url: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_non_empty_string")]
    pub auth_scheme: String,
    /// Write-only structured credentials (shape depends on `auth_scheme`). Key
    /// schemes use `{ "api_keys": ["..."] }`. Values are encrypted at rest.
    #[ts(type = "unknown")]
    pub credentials: serde_json::Value,
    #[serde(default)]
    #[ts(optional = nullable, type = "unknown")]
    pub extra: Option<serde_json::Value>,
}

/// Full metadata save for `PUT /api/providers/:id/connections`.
///
/// Unlike aggregate creation, omitting `credentials` means preserve the
/// existing encrypted payload. The service rejects an omitted payload when
/// the `(provider_id, role)` row does not yet exist.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct SaveProviderConnectionRequest {
    pub role: String,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub label: Option<String>,
    pub base_url: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_non_empty_string")]
    pub auth_scheme: String,
    /// Omitted keeps an existing connection's encrypted credential payload.
    #[serde(default)]
    #[ts(optional = nullable, type = "unknown")]
    pub credentials: Option<serde_json::Value>,
    #[serde(default)]
    #[ts(optional = nullable, type = "unknown")]
    pub extra: Option<serde_json::Value>,
}
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
    fn response_requires_complete_extra_and_rejects_unknown_fields() {
        assert!(
            serde_json::from_value::<ProviderConnectionResponse>(json!({
                "connection_id": "018f1234-5678-7abc-8def-012345678991",
                "provider_id": PROVIDER_ID,
                "role": "voice",
                "base_url": "https://voice.example.com/v1",
                "auth_scheme": "bearer",
                "has_credentials": false,
                "created_at": 1,
                "updated_at": 2
            }))
            .is_err()
        );
        let mut value = serde_json::to_value(sample_response()).unwrap();
        value["credentials"] = json!({"api_keys":["must-not-be-accepted"]});
        assert!(serde_json::from_value::<ProviderConnectionResponse>(value).is_err());
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

        let mut value = serde_json::to_value(sample_response()).unwrap();
        value["connection_id"] = json!("voice");
        assert!(serde_json::from_value::<ProviderConnectionResponse>(value).is_err());
    }

    #[test]
    fn aggregate_create_requires_explicit_auth_scheme_and_credentials() {
        assert!(
            serde_json::from_value::<ProviderConnectionInput>(json!({
                "role": "voice",
                "base_url": "https://voice.example.com/v1",
                "credentials": {}
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<ProviderConnectionInput>(json!({
                "role": "voice",
                "base_url": "https://voice.example.com/v1",
                "auth_scheme": " ",
                "credentials": {}
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<ProviderConnectionInput>(json!({
                "role": "voice",
                "base_url": "https://voice.example.com/v1",
                "auth_scheme": "bearer"
            }))
            .is_err()
        );
        let parsed: ProviderConnectionInput = serde_json::from_value(json!({
            "role": "voice",
            "base_url": "https://voice.example.com/v1",
            "auth_scheme": "bearer",
            "credentials": {}
        }))
        .unwrap();
        assert_eq!(parsed.role, "voice");
        assert_eq!(parsed.label, None);
        assert_eq!(parsed.auth_scheme, "bearer");
        assert_eq!(parsed.credentials, json!({}));
        assert!(parsed.extra.is_none());
    }

    #[test]
    fn request_carries_write_only_credentials() {
        let parsed: ProviderConnectionInput = serde_json::from_value(json!({
            "role": "voice",
            "base_url": "https://voice.example.com/v1",
            "auth_scheme": "header_key:x-api-key",
            "credentials": { "api_keys": ["sk-live-1234"] },
            "extra": { "region": "eu" }
        }))
        .unwrap();
        assert_eq!(parsed.auth_scheme, "header_key:x-api-key");
        assert_eq!(parsed.credentials, json!({ "api_keys": ["sk-live-1234"] }));
        assert_eq!(parsed.extra, Some(json!({ "region": "eu" })));
    }

    #[test]
    fn request_rejects_unknown_fields() {
        let result = serde_json::from_value::<ProviderConnectionInput>(json!({
            "role": "voice",
            "base_url": "https://voice.example.com/v1",
            "auth_scheme": "bearer",
            "credentials": {},
            "credentials_encrypted": "sneaky-ciphertext"
        }));
        assert!(
            result.is_err(),
            "deny_unknown_fields must reject stray fields"
        );
    }

    #[test]
    fn save_request_distinguishes_omitted_credentials_from_replacement() {
        let omitted: SaveProviderConnectionRequest = serde_json::from_value(json!({
            "role": "voice",
            "base_url": "https://voice.example.com/v1",
            "auth_scheme": "bearer"
        }))
        .unwrap();
        assert!(omitted.credentials.is_none());

        let replacement: SaveProviderConnectionRequest = serde_json::from_value(json!({
            "role": "voice",
            "base_url": "https://voice.example.com/v1",
            "auth_scheme": "bearer",
            "credentials": { "api_keys": ["new-key"] }
        }))
        .unwrap();
        assert_eq!(
            replacement.credentials,
            Some(json!({ "api_keys": ["new-key"] }))
        );
    }
}
