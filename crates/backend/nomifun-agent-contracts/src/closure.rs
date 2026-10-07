use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ArtifactEnvelope, VersionString};

pub type ContractClosureArtifact = ArtifactEnvelope<ContractClosurePayload>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    Confirmed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfirmedDecision {
    pub decision_id: String,
    pub status: DecisionStatus,
    pub contract: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonicalSourceOwner {
    pub source_kind: String,
    pub logical_path: String,
    pub owns: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContractClosureResolution {
    pub resolution_id: String,
    pub canonical_rule: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContractClosurePayload {
    pub schema_version: VersionString,
    pub decision_revision: String,
    pub decisions: Vec<ConfirmedDecision>,
    pub canonical_sources: Vec<CanonicalSourceOwner>,
    pub closure_resolutions: Vec<ContractClosureResolution>,
    pub production_behavior_included: bool,
    pub unresolved_decisions: Vec<String>,
}
