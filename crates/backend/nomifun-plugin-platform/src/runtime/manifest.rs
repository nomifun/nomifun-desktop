//! Authoring contract for composable plugin runtime roles.
//!
//! This file belongs to the source snapshot, so changes to capabilities, data
//! requirements or migrations participate in the same build and release digest.
use std::collections::BTreeMap;

use nomifun_agent_contracts::{
    CanonicalSchemaRef, CredentialSlotDeclaration, PluginMigration,
    PluginResourceContract, PackageContributions, StrictJsonValue,
    ActionId, CapabilityActionDescriptor, CapabilityContributions, CapabilityConsumer,
    CapabilityKind, CapabilityManifest, EffectClass, LocalizedMetadata, PackageRef,
    PlatformConstraint, ToolPresentationKind, capability_surface_declarations, digest_payload,
};
use serde::{Deserialize, Serialize};

pub const PLUGIN_RUNTIME_MANIFEST_PATH: &str = "nomifun.plugin.json";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PluginRuntimeSourceManifest {
    /// Explicit opt-in; an ordinary HTML page is not an Agent view.
    pub agent_view: Option<PluginAgentViewSource>,
    pub actions: Vec<PluginActionSource>,
    pub lifecycle: Option<nomifun_agent_contracts::PluginServiceLifecycle>,
    pub contributions: PackageContributions,
    pub schemas: BTreeMap<CanonicalSchemaRef, StrictJsonValue>,
    pub credential_slots: Vec<CredentialSlotDeclaration>,
    pub resource_contract: PluginResourceContract,
    pub migrations: Vec<PluginMigration>,
    pub uses_files: bool,
    pub uses_private_database: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginAgentViewSource {
    pub name: String,
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authored_tool_check() -> PluginRuntimeSourceManifest {
        let action = nomifun_agent_contracts::tool_middleware::before_action();
        let schemas = nomifun_agent_contracts::tool_middleware::schemas();
        PluginRuntimeSourceManifest {
            actions: vec![PluginActionSource {
                id: action.action_id.as_ref().to_owned(), name: "Business check".into(),
                description: "Inspect a tool request before dispatch".into(),
                input_schema: schemas[&action.input_schema].clone(),
                output_schema: schemas[&action.output_schema].clone(), effect: EffectClass::Pure,
            }], ..Default::default()
        }
    }

    #[test]
    fn authored_tool_check_materializes_the_real_hidden_consumer_contract() {
        let mut source = authored_tool_check();
        source.materialize_actions(&PackageRef { id: "plugin.authored".into(), version: "1.0.0".into() }).unwrap();
        let capability = &source.contributions.capabilities[0];
        assert_eq!(capability.kind, CapabilityKind::TurnMiddleware);
        assert_eq!(nomifun_agent_contracts::tool_middleware::phase_for_actions(&capability.contributions.actions), Some("before_tool"));
        nomifun_agent_contracts::tool_middleware::validate_manifest(capability).unwrap();
        nomifun_agent_contracts::validate_release_schema_registry(&source.contributions, &source.schemas).unwrap();
        assert!(capability.contributions.actions.iter().all(|a| a.presentation == ToolPresentationKind::Hidden));
    }

    #[test]
    fn authored_tool_check_cannot_relabel_a_different_schema_or_effect() {
        for mutation in 0..3 {
            let mut source = authored_tool_check();
            match mutation {
                0 => source.actions[0].effect = EffectClass::ExecuteLocal,
                1 => source.actions[0].input_schema = StrictJsonValue(serde_json::json!({"type":"object"})),
                _ => source.actions[0].output_schema = StrictJsonValue(serde_json::json!({"type":"object"})),
            }
            assert!(source.materialize_actions(&PackageRef { id: "plugin.authored".into(), version: "1.0.0".into() }).is_err());
        }
    }

    #[test]
    fn historical_agent_view_parses_but_cannot_materialize_new_publications() {
        let package = PackageRef { id: "plugin.example".into(), version: "1.0.0".into() };
        let source = br#"{"agent_view":{"name":"My view","description":"Historical page"},"actions":[{"id":"echo","name":"Echo","description":"Echo","input_schema":{"type":"object"},"output_schema":{"type":"object"},"effect":"pure"}]}"#;
        let mut historical = PluginRuntimeSourceManifest::parse(source).unwrap();
        assert!(historical.agent_view.is_some());
        assert_eq!(historical.actions.len(), 1);
        assert!(historical.materialize_actions(&package).unwrap_err().contains("unsupported"));
        historical.agent_view = None;
        historical.materialize_actions(&package).unwrap();
        assert_eq!(historical.contributions.capabilities.len(), 1);
        assert_eq!(historical.contributions.capabilities[0].kind, CapabilityKind::Tool);
    }

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
        if self.agent_view.is_some() || self.contributions.capabilities.iter().any(|capability| {
            capability.contributions.ui_slot == Some(nomifun_agent_contracts::UiContributionSlot::AgentSession)
        }) {
            return Err("Plugin Agent Session views are unsupported; remove the agent_view/AgentSession declaration before publishing".into());
        }
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
            let hook = match action.id.as_str() {
                nomifun_agent_contracts::tool_middleware::BEFORE_ACTION_ID => Some((
                    nomifun_agent_contracts::tool_middleware::before_action(),
                    nomifun_agent_contracts::tool_middleware::schemas())),
                nomifun_agent_contracts::model_middleware::ACTION_ID => Some((
                    nomifun_agent_contracts::model_middleware::action(),
                    nomifun_agent_contracts::model_middleware::schemas())),
                _ => None,
            };
            let (input, output) = if let Some((descriptor, schemas)) = &hook {
                if action.effect != EffectClass::Pure
                    || schemas.get(&descriptor.input_schema) != Some(&action.input_schema)
                    || schemas.get(&descriptor.output_schema) != Some(&action.output_schema) {
                    return Err("Agent execution extensions must preserve the exact host schemas and pure effect".into());
                }
                (descriptor.input_schema.clone(), descriptor.output_schema.clone())
            } else {
                (schema_ref("input", &action.input_schema)?, schema_ref("output", &action.output_schema)?)
            };
            self.schemas.insert(input.clone(), action.input_schema.clone());
            self.schemas.insert(output.clone(), action.output_schema.clone());
            self.contributions.capabilities.push(CapabilityManifest {
                id: id.clone().into(), contribution_id: format!("capability:{id}").into(),
                version: package.version.clone(), kind: if hook.is_some() { CapabilityKind::TurnMiddleware } else { CapabilityKind::Tool }, package: package.clone(),
                display: LocalizedMetadata { name: action.name.clone(), description: action.description.clone(), localized_names: BTreeMap::new(), localized_descriptions: BTreeMap::new() },
                requires: vec![], conflicts: vec![], requires_runtime_features: vec![],
                supported_surfaces: capability_surface_declarations(["desktop"], [CapabilityConsumer::Agent, CapabilityConsumer::Ui, CapabilityConsumer::PluginService]),
                supported_platforms: vec![PlatformConstraint::Any],
                config_schema: StrictJsonValue(serde_json::json!({"type":"object"})),
                contributions: CapabilityContributions {
                    actions: vec![hook.map(|(descriptor, _)| descriptor).unwrap_or(CapabilityActionDescriptor {
                        action_id: ActionId::from(action.id.clone()), input_schema: input, output_schema: output,
                        effect_class: action.effect, presentation: ToolPresentationKind::FunctionTool,
                    })], ..Default::default()
                },
            });
        }
        Ok(())
    }
}
