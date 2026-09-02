use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

macro_rules! string_newtype {
    ($name:ident) => {
        #[derive(
            Clone,
            Debug,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Serialize,
            Deserialize,
            JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

string_newtype!(ActionId);
string_newtype!(AgentPresetId);
string_newtype!(AgentSessionId);
string_newtype!(ArtifactId);
string_newtype!(CapabilityId);
string_newtype!(CanonicalErrorCode);
string_newtype!(CanonicalSchemaRef);
string_newtype!(ConnectionConfigRef);
string_newtype!(CorrelationId);
string_newtype!(DigestHex);
string_newtype!(EventId);
string_newtype!(EventProducerId);
string_newtype!(HostPortId);
string_newtype!(IdempotencyKey);
string_newtype!(McpServerId);
string_newtype!(McpToolKey);
string_newtype!(ModelRouteId);
string_newtype!(OperationId);
string_newtype!(PackageId);
string_newtype!(PluginMountId);
string_newtype!(ProjectionReducerId);
string_newtype!(RemoteBindingId);
string_newtype!(ResolvedSnapshotId);
string_newtype!(ResourceBindingId);
string_newtype!(ResourceId);
string_newtype!(ResourceKind);
string_newtype!(RuntimeBindingId);
string_newtype!(RuntimeFeatureId);
string_newtype!(RuntimeTarget);
string_newtype!(ScopeKey);
string_newtype!(ServiceKeyId);
string_newtype!(SkillId);
string_newtype!(StateKey);
string_newtype!(UserId);
string_newtype!(VersionString);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct StrictJsonValue(pub Value);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrincipalRef {
    pub principal_kind: String,
    pub principal_id: String,
}

#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct ExactVersionRef<T> {
    pub id: T,
    pub version: VersionString,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TypedResourceBinding {
    pub binding_id: ResourceBindingId,
    pub resource_kind: ResourceKind,
    pub resource_id: ResourceId,
    pub owner_id: String,
    pub operations: BTreeSet<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_config_ref: Option<ConnectionConfigRef>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub typed_parameters: BTreeMap<String, String>,
}

pub type TypedResourceBindings = Vec<TypedResourceBinding>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogicalArtifactRef {
    pub artifact_id: ArtifactId,
    pub normalized_relative_path: String,
    pub digest: DigestHex,
}
