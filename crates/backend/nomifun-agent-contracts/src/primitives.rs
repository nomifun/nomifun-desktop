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
string_newtype!(ContributionId);
string_newtype!(CredentialId);
string_newtype!(CredentialSlotKey);
string_newtype!(CorrelationId);
string_newtype!(DigestHex);
string_newtype!(EventId);
string_newtype!(EventProducerId);
string_newtype!(HostPortId);
string_newtype!(IdempotencyKey);
string_newtype!(McpBindingId);
string_newtype!(McpServerId);
string_newtype!(McpToolKey);
string_newtype!(PluginId);
string_newtype!(PluginDraftId);
string_newtype!(PluginMutationId);
string_newtype!(ModelRouteId);
string_newtype!(OperationId);
string_newtype!(PackageId);
string_newtype!(AgentModuleId);
string_newtype!(ProjectionReducerId);
string_newtype!(RemoteBindingId);
string_newtype!(ResolvedSnapshotId);
string_newtype!(ResourceBindingId);
string_newtype!(ResourceId);
string_newtype!(ResourceKind);
string_newtype!(RuntimeBindingId);
string_newtype!(RuntimeFeatureId);
string_newtype!(RuntimeInstallationId);
string_newtype!(RuntimeTarget);
string_newtype!(ScopeKey);
string_newtype!(ServiceKeyId);
string_newtype!(SkillId);
string_newtype!(StateKey);
string_newtype!(StableSourceIdentity);
string_newtype!(ValidationCohortId);
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

/// Stable identity for one complete immutable resource definition. The current
/// Agent binding selects this definition; historical Effects can retain it
/// after any of its authority or configuration fields change. binding_id is
/// deliberately excluded, and no typed parameter is discarded.
pub fn resource_definition_id(
    binding: &TypedResourceBinding,
) -> Result<ResourceBindingId, crate::CanonicalDigestError> {
    let definition = serde_json::json!({
        "resource_kind": binding.resource_kind,
        "resource_id": binding.resource_id,
        "owner_id": binding.owner_id,
        "operations": binding.operations,
        "connection_config_ref": binding.connection_config_ref,
        "typed_parameters": binding.typed_parameters,
    });
    let digest = crate::digest_payload(&definition)?;
    // Fixed prefix plus SHA-256: long kinds, paths and options cannot enlarge
    // the identity beyond 84 bytes or expose those values in the key.
    Ok(ResourceBindingId::from(format!(
        "resource-definition:{}", digest.as_ref()
    )))
}

pub type TypedResourceBindings = Vec<TypedResourceBinding>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogicalArtifactRef {
    pub artifact_id: ArtifactId,
    pub normalized_relative_path: String,
    pub digest: DigestHex,
}

#[derive(
    Clone,
    Copy,
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
#[serde(rename_all = "snake_case")]
pub enum ContributionSourceKind {
    PlatformBuiltin,
    AgentModule,
    McpBinding,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity_test_resource() -> TypedResourceBinding {
        TypedResourceBinding {
            binding_id: ResourceBindingId::from("unassigned"),
            resource_kind: ResourceKind::from("knowledge_base"),
            resource_id: ResourceId::from("base-a"),
            owner_id: "owner".to_owned(),
            operations: BTreeSet::from(["read".to_owned(), "search".to_owned(), "write".to_owned()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::from([
                ("knowledge_root".to_owned(), "C:/knowledge".to_owned()),
                ("knowledge_name".to_owned(), "Knowledge A".to_owned()),
                ("knowledge_description".to_owned(), "Exact scoped data".to_owned()),
                ("knowledge_enabled".to_owned(), "true".to_owned()),
                ("knowledge_writeback".to_owned(), "false".to_owned()),
                ("knowledge_writeback_eagerness".to_owned(), "manual".to_owned()),
            ]),
        }
    }

    #[test]
    fn resource_definition_identity_is_stable_excludes_its_own_id_and_bounds_the_key() {
        let mut binding = identity_test_resource();
        let id = resource_definition_id(&binding).unwrap();
        binding.binding_id = id.clone();
        assert_eq!(resource_definition_id(&binding).unwrap(), id,
            "reusing one definition must not recursively hash its generated identity");
        binding.binding_id = ResourceBindingId::from("another-unassigned-id");
        assert_eq!(resource_definition_id(&binding).unwrap(), id);
        let original_parameters = binding.typed_parameters.clone();
        resource_definition_id(&binding).unwrap();
        assert_eq!(binding.typed_parameters, original_parameters, "every parameter remains intact");
        binding.typed_parameters.insert("extra_exact_data".to_owned(), "界".repeat(20_000));
        let bounded = resource_definition_id(&binding).unwrap();
        assert_eq!(bounded.as_ref().len(), 84);
        assert_ne!(bounded, id);
    }

    #[test]
    fn resource_definition_identity_changes_for_each_immutable_definition_field() {
        let original = identity_test_resource();
        let id = resource_definition_id(&original).unwrap();
        let mut variations = Vec::new();
        let mut changed = original.clone();
        changed.resource_kind = ResourceKind::from("another-resource-kind");
        variations.push(changed);
        let mut changed = original.clone();
        changed.resource_id = ResourceId::from("base-b");
        variations.push(changed);
        let mut changed = original.clone();
        changed.owner_id = "another-owner".to_owned();
        variations.push(changed);
        let mut changed = original.clone();
        changed.operations.remove("write");
        variations.push(changed);
        let mut changed = original.clone();
        changed.connection_config_ref = Some(ConnectionConfigRef::from("connection-b"));
        variations.push(changed);
        for (key, value) in [
            ("knowledge_root", "C:/another-root"),
            ("knowledge_name", "Renamed knowledge"),
            ("knowledge_description", "Updated exact data"),
            ("knowledge_enabled", "false"),
            ("knowledge_writeback", "true"),
            ("knowledge_writeback_eagerness", "auto"),
            ("extra_exact_data", "preserved"),
        ] {
            let mut changed = original.clone();
            changed.typed_parameters.insert(key.to_owned(), value.to_owned());
            variations.push(changed);
        }
        let changed_ids = variations.iter().map(|binding| resource_definition_id(binding).unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(changed_ids.len(), variations.len());
        assert!(!changed_ids.contains(&id));
    }

    #[test]
    fn same_physical_workspace_with_narrower_operations_has_a_new_definition_identity() {
        let mut workspace = identity_test_resource();
        workspace.resource_kind = ResourceKind::from("workspace");
        workspace.resource_id = ResourceId::from("selected-workspace");
        workspace.operations = BTreeSet::from(["read".to_owned(), "write".to_owned()]);
        workspace.typed_parameters = BTreeMap::from([("workspace_root".to_owned(), "C:/same-project".to_owned())]);
        let writable = resource_definition_id(&workspace).unwrap();
        workspace.operations.remove("write");
        let read_only = resource_definition_id(&workspace).unwrap();
        assert_ne!(writable, read_only);
        workspace.operations.insert("write".to_owned());
        assert_eq!(resource_definition_id(&workspace).unwrap(), writable,
            "returning to the exact definition must reuse its historical identity");
    }

    #[test]
    fn agent_module_source_kind_has_one_canonical_wire_value() {
        assert_eq!(
            serde_json::to_value(ContributionSourceKind::AgentModule).unwrap(),
            serde_json::json!("agent_module")
        );
        assert!(
            serde_json::from_value::<ContributionSourceKind>(serde_json::json!("plugin_mount"))
                .is_err()
        );
    }
}
