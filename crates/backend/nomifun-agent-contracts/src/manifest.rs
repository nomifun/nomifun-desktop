use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ArtifactEnvelope, DigestHex, VersionString};

pub type CanonicalV4SchemaManifest =
    ArtifactEnvelope<CanonicalV4SchemaManifestPayload>;
pub type ContractDigestLedger = ArtifactEnvelope<ContractDigestLedgerPayload>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonicalV4SchemaManifestPayload {
    pub manifest_version: VersionString,
    pub database_schema_digest: DigestHex,
    pub rust_contract_schema_digest: DigestHex,
    pub package_schema_digest: DigestHex,
    pub official_preset_seed_manifest_digest: DigestHex,
    pub canonical_api_inventory_digest: DigestHex,
    pub session_event_registry_digest: DigestHex,
    pub error_registry_digest: DigestHex,
    pub runtime_protocol_digest: DigestHex,
    pub runtime_feature_inventory_digest: DigestHex,
    pub deletion_manifest_set_digest: DigestHex,
    pub platform_validation_contract_digest: DigestHex,
    pub confirmed_decision_contract_digest: DigestHex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContractDigestLedgerPayload {
    pub ledger_version: VersionString,
    pub base_source_sha: String,
    pub artifacts: BTreeMap<String, DigestHex>,
}
