//! NomiFun Model Gateway v1 control plane and its desktop API view.
//!
//! Upstream JSON is deserialized as strict signed 64-bit integers. Financial,
//! quota and rate values are serialized to the desktop API as decimal strings
//! so the renderer's JSON.parse cannot round them. This asymmetric response
//! view does not change the external gateway v1 JSON-integer contract.
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use crate::{ModelTask, ProviderModelInput, ProviderModelResponse};

/// Local synchronization provenance. Never forwarded to a model service.
pub const MODEL_GATEWAY_CATALOG_BASELINE_PARAM: &str = "_nomifun_gateway_catalog_baseline";

fn serialize_i64_decimal<S: serde::Serializer>(value: &i64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.to_string())
}

fn serialize_optional_i64_decimal<S: serde::Serializer>(value: &Option<i64>, serializer: S) -> Result<S::Ok, S::Error> {
    match value {
        Some(value) => serializer.serialize_some(&value.to_string()),
        None => serializer.serialize_none(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ModelGatewayOperator {
    pub name: String,
    pub homepage_url: Option<String>,
    pub console_url: Option<String>,
    pub purchase_url: Option<String>,
    pub terms_url: Option<String>,
    pub privacy_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ModelGatewayMetaResponse {
    pub contract_version: String,
    pub operator: ModelGatewayOperator,
    pub capabilities: Vec<String>,
    pub optional_endpoints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ModelGatewayTaskEndpoints {
    pub endpoints: Vec<String>,
    pub preferred_endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ModelGatewayPrice {
    pub task: ModelTask,
    pub meter: String,
    #[serde(serialize_with = "serialize_i64_decimal")]
    #[ts(type = "string")]
    pub unit_size: i64,
    #[serde(serialize_with = "serialize_i64_decimal")]
    #[ts(type = "string")]
    pub amount: i64,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ModelGatewayCatalogModel {
    pub id: String,
    pub display_name: String,
    pub vendor: String,
    pub tasks: Vec<ModelTask>,
    pub task_endpoints: BTreeMap<String, ModelGatewayTaskEndpoints>,
    #[ts(type = "number | null")]
    pub context_window: Option<i64>,
    #[ts(type = "number | null")]
    pub max_output_tokens: Option<i64>,
    pub input_modalities: Vec<String>,
    pub traits: Vec<String>,
    pub pricing: Vec<ModelGatewayPrice>,
    pub included_in_plan: bool,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ModelGatewayCatalogResponse {
    pub contract_version: String,
    pub models: Vec<ModelGatewayCatalogModel>,
    /// Exact validated configuration; no platform or name-based inference.
    pub imports: Vec<ProviderModelInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ModelGatewayQuota {
    pub unit: String,
    #[serde(serialize_with = "serialize_optional_i64_decimal")]
    #[ts(type = "string | null")]
    pub total: Option<i64>,
    #[serde(serialize_with = "serialize_i64_decimal")]
    #[ts(type = "string")]
    pub used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ModelGatewayPlan {
    pub name: String,
    pub period_start: Option<String>,
    pub period_end: Option<String>,
    pub quota: ModelGatewayQuota,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ModelGatewayBalance {
    #[serde(serialize_with = "serialize_i64_decimal")]
    #[ts(type = "string")]
    pub amount: i64,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ModelGatewayKey {
    pub name: String,
    pub expires_at: Option<String>,
    pub quota_unit: String,
    #[serde(serialize_with = "serialize_optional_i64_decimal")]
    #[ts(type = "string | null")]
    pub remaining_quota: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ModelGatewayRateLimits {
    #[serde(serialize_with = "serialize_optional_i64_decimal")]
    #[ts(type = "string | null")]
    pub requests_per_minute: Option<i64>,
    #[serde(serialize_with = "serialize_optional_i64_decimal")]
    #[ts(type = "string | null")]
    pub tokens_per_minute: Option<i64>,
    #[serde(serialize_with = "serialize_optional_i64_decimal")]
    #[ts(type = "string | null")]
    pub concurrent_requests: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct ModelGatewayAccountResponse {
    pub contract_version: String,
    pub plan: Option<ModelGatewayPlan>,
    pub balance: ModelGatewayBalance,
    pub key: ModelGatewayKey,
    pub rate_limits: ModelGatewayRateLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct ModelGatewayMetaRequest { pub base_url: String }

// Credential-bearing DTOs deliberately do not implement Debug.
#[derive(Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct ModelGatewayCatalogRequest { pub base_url: String, pub api_key: String }

#[derive(Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct CreateModelGatewayRequest {
    pub base_url: String,
    pub api_key: String,
    pub name: String,
    pub models: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
#[serde(deny_unknown_fields)]
pub struct UpdateModelGatewayConnectionRequest {
    pub base_url: String,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub api_key: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ts_rs::TS)]
#[ts(export_to = "../../../../ui/src/common/protocolBindings/")]
pub struct SyncModelGatewayResponse {
    #[ts(type = "number")]
    pub added: usize,
    #[ts(type = "number")]
    pub updated: usize,
    pub models: Vec<ProviderModelResponse>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_i64_boundaries_serialize_to_lossless_desktop_decimal_strings() {
        let balance: ModelGatewayBalance = serde_json::from_value(json!({"amount":i64::MIN,"currency":"USD"})).unwrap();
        assert_eq!(balance.amount,i64::MIN);
        assert_eq!(serde_json::to_value(balance).unwrap()["amount"],i64::MIN.to_string());
        let price: ModelGatewayPrice = serde_json::from_value(json!({"task":"chat","meter":"requests","unit_size":i64::MAX,"amount":i64::MAX,"currency":"USD"})).unwrap();
        let view=serde_json::to_value(price).unwrap();
        assert_eq!(view["unit_size"],i64::MAX.to_string()); assert_eq!(view["amount"],i64::MAX.to_string());
        let quota: ModelGatewayQuota=serde_json::from_value(json!({"unit":"tokens","total":i64::MAX,"used":i64::MAX})).unwrap();
        let view=serde_json::to_value(quota).unwrap();
        assert_eq!(view["total"],i64::MAX.to_string()); assert_eq!(view["used"],i64::MAX.to_string());
        let limits:ModelGatewayRateLimits=serde_json::from_value(json!({"requests_per_minute":i64::MAX,"tokens_per_minute":null,"concurrent_requests":0})).unwrap();
        let view=serde_json::to_value(limits).unwrap();
        assert_eq!(view["requests_per_minute"],i64::MAX.to_string()); assert!(view["tokens_per_minute"].is_null()); assert_eq!(view["concurrent_requests"],"0");
    }

    #[test]
    fn native_wire_rejects_decimal_strings_fractions_and_out_of_range_numbers() {
        for invalid in [json!("9223372036854775807"),json!(1.5),json!(i64::MAX as u64+1)] {
            assert!(serde_json::from_value::<ModelGatewayBalance>(json!({"amount":invalid,"currency":"USD"})).is_err());
            assert!(serde_json::from_value::<ModelGatewayPrice>(json!({"task":"chat","meter":"requests","unit_size":invalid,"amount":0,"currency":"USD"})).is_err());
            assert!(serde_json::from_value::<ModelGatewayQuota>(json!({"unit":"tokens","total":invalid,"used":0})).is_err());
            assert!(serde_json::from_value::<ModelGatewayKey>(json!({"name":"fixture","expires_at":null,"quota_unit":"tokens","remaining_quota":invalid})).is_err());
            assert!(serde_json::from_value::<ModelGatewayRateLimits>(json!({"requests_per_minute":invalid,"tokens_per_minute":null,"concurrent_requests":null})).is_err());
        }
        assert!(serde_json::from_str::<ModelGatewayBalance>(r#"{"amount":-9223372036854775809,"currency":"USD"}"#).is_err());
    }
}
