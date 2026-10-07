//! Platform feature vocabulary used to materialize capability declarations.
//!
//! This is not an Engine support claim or an execution grant. Each exact
//! Engine build must still admit its snapshot through its own support policy.
//! No executable, source pin, RPC allowlist or permission mode belongs here.

use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{CanonicalErrorCode, RuntimeContractViolation, RuntimeFeatureId, VersionString};

pub const PLATFORM_FEATURE_INVENTORY_JSON: &str =
    include_str!("../contracts/engine/platform-feature-inventory.payload.json");

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlatformFeatureInventoryPayload {
    pub schema_version: VersionString,
    pub runtime_features: BTreeSet<RuntimeFeatureId>,
}

impl PlatformFeatureInventoryPayload {
    pub fn validate(&self) -> Result<(), RuntimeContractViolation> {
        let invalid = |message: &str| RuntimeContractViolation {
            code: CanonicalErrorCode::from("PLATFORM_FEATURE_INVENTORY_INVALID"),
            message: message.to_owned(),
        };
        if self.schema_version.as_ref() != "1.0.0" {
            return Err(invalid("unsupported platform feature inventory schema"));
        }
        if self.runtime_features.is_empty() || self.runtime_features.len() > 256 {
            return Err(invalid(
                "platform feature inventory must contain 1 to 256 feature IDs",
            ));
        }
        for feature in &self.runtime_features {
            let id = feature.as_ref();
            if id.is_empty()
                || id.len() > 128
                || !id.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'.' | b'_' | b'-')
                })
            {
                return Err(invalid(
                    "platform feature inventory contains an invalid feature ID",
                ));
            }
        }
        Ok(())
    }
}

pub fn platform_feature_inventory_payload()
-> Result<PlatformFeatureInventoryPayload, RuntimeContractViolation> {
    let inventory: PlatformFeatureInventoryPayload =
        serde_json::from_str(PLATFORM_FEATURE_INVENTORY_JSON).map_err(|_| {
            RuntimeContractViolation {
                code: CanonicalErrorCode::from("PLATFORM_FEATURE_INVENTORY_INVALID"),
                message: "compiled platform feature inventory is malformed".to_owned(),
            }
        })?;
    inventory.validate()?;
    Ok(inventory)
}
