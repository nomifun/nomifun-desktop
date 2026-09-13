//! Authoring contract for composable plugin runtime roles.
//!
//! This file belongs to the source snapshot, so changes to capabilities, data
//! requirements or migrations participate in the same build and release digest.
use std::collections::BTreeMap;

use nomifun_agent_contracts::{
    CanonicalSchemaRef, CredentialSlotDeclaration, MiniAppMigration,
    MiniAppResourceContract, PackageContributions, StrictJsonValue,
    ActionId, CapabilityActionDescriptor, CapabilityContributions, CapabilityConsumer,
    CapabilityKind, CapabilityManifest, EffectClass, LocalizedMetadata, PackageRef,
    PlatformConstraint, ToolPresentationKind, capability_surface_declarations, digest_payload,
};
use serde::{Deserialize, Serialize};

pub const PLUGIN_RUNTIME_MANIFEST_PATH: &str = "nomifun.plugin.json";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PluginRuntimeSourceManifest {
    pub actions: Vec<PluginActionSource>,
    pub lifecycle: Option<nomifun_agent_contracts::MiniAppServiceLifecycle>,
    pub contributions: PackageContributions,
    pub schemas: BTreeMap<CanonicalSchemaRef, StrictJsonValue>,
    pub credential_slots: Vec<CredentialSlotDeclaration>,
    pub resource_contract: MiniAppResourceContract,
    pub migrations: Vec<MiniAppMigration>,
    pub uses_files: bool,
    pub uses_private_database: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_bind_to_the_plugin_package_and_schema_digests() {
        let mut source = PluginRuntimeSourceManifest::parse(br#"{"actions":[{"id":"echo","name":"Echo","description":"Return the input","input_schema":{"type":"object"},"output_schema":{"type":"object"},"effect":"pure"}]}"#).unwrap();
        let package = PackageRef { id: "plugin.example".into(), version: "1.0.0".into() };
        source.materialize_actions(&package).unwrap();
        let capability = &source.contributions.capabilities[0];
        assert_eq!(capability.id.as_ref(), "plugin.example.echo");
        assert_eq!(capability.package, package);
        assert_eq!(capability.contributions.actions[0].action_id.as_ref(), "echo");
        assert!(source.schemas.contains_key(&capability.contributions.actions[0].input_schema));
        source.actions.push(source.actions[0].clone());
        assert!(source.materialize_actions(&package).is_err());
        assert!(PluginRuntimeSourceManifest::parse(br#"{"allow_everything":true}"#).is_err());
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginActionSource {
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_schema: StrictJsonValue,
    pub output_schema: StrictJsonValue,
    pub effect: EffectClass,
}

impl PluginRuntimeSourceManifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    pub fn materialize_actions(&mut self, package: &PackageRef) -> Result<(), String> {
        let mut ids = std::collections::BTreeSet::new();
        for action in &self.actions {
            if action.id.is_empty() || action.id.len() > 96
                || !action.id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
                || !ids.insert(&action.id) || action.name.trim().is_empty() || action.description.trim().is_empty()
                || !action.input_schema.0.is_object() || !action.output_schema.0.is_object()
            {
                return Err("Plugin actions require unique method names, descriptions, and object schemas".into());
            }
            let id = format!("{}.{}", package.id.as_ref(), action.id);
            let schema_ref = |kind: &str, value: &StrictJsonValue| -> Result<CanonicalSchemaRef, String> {
                Ok(format!("schema://{id}/{kind}@1#{}", digest_payload(&value.0).map_err(|e| e.to_string())?.as_ref()).into())
            };
            let input = schema_ref("input", &action.input_schema)?;
            let output = schema_ref("output", &action.output_schema)?;
            self.schemas.insert(input.clone(), action.input_schema.clone());
            self.schemas.insert(output.clone(), action.output_schema.clone());
            self.contributions.capabilities.push(CapabilityManifest {
                id: id.clone().into(), contribution_id: format!("capability:{id}").into(),
                version: package.version.clone(), kind: CapabilityKind::Tool, package: package.clone(),
                display: LocalizedMetadata { name: action.name.clone(), description: action.description.clone(), localized_names: BTreeMap::new(), localized_descriptions: BTreeMap::new() },
                requires: vec![], conflicts: vec![], requires_runtime_features: vec![],
                supported_surfaces: capability_surface_declarations(["desktop"], [CapabilityConsumer::Agent, CapabilityConsumer::Ui, CapabilityConsumer::MiniAppService]),
                supported_platforms: vec![PlatformConstraint::Any],
                config_schema: StrictJsonValue(serde_json::json!({"type":"object"})),
                contributions: CapabilityContributions {
                    actions: vec![CapabilityActionDescriptor {
                        action_id: ActionId::from(action.id.clone()), input_schema: input, output_schema: output,
                        effect_class: action.effect, presentation: ToolPresentationKind::FunctionTool,
                    }], ..Default::default()
                },
            });
        }
        Ok(())
    }
}
