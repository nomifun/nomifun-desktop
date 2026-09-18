//! Projection of selected, exact Snapshot contributions into Nomi tools.
//! No global catalog discovery and no executable/runtime loading occurs here.
use nomifun_agent_contracts::{
    ActionId, CapabilityId, CapabilityManifest, ContributionSourceKind, ToolPresentationKind,
};
use nomifun_agent_kernel::{ActiveCapabilitySetSnapshot, CompiledSnapshot, MaterializedRegistry};
use nomifun_ai_agent::NomiPluginToolSchemaResolver;
use nomifun_chat_model_broker::ChatToolDefinition;
use nomifun_agent_runtime::{
    AgentToolExposure, AgentToolPlan, compile_agent_tool_plan, standard_agent_tool_exposures,
};
use nomifun_common::AppError;
use sha2::{Digest, Sha256};

fn error(value: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Nomi tool surface: {value}"))
}

pub(super) async fn compile(
    snapshot: &CompiledSnapshot,
    active: &ActiveCapabilitySetSnapshot,
    registry: &MaterializedRegistry,
    plugin_schemas: &dyn NomiPluginToolSchemaResolver,
) -> Result<AgentToolPlan, AppError> {
    if snapshot.registry_generation != registry.generation
        || snapshot.registry_digest != registry.registry_digest
    {
        return Err(error("registry changed after Snapshot compilation"));
    }
    if snapshot.content().enabled_capabilities.len() > 128
    {
        return Err(error("Nomi supports at most 128 selected capabilities"));
    }
    reject_retired_extension_authority(
        snapshot
            .content()
            .enabled_capabilities
            .iter()
            .map(|selected| &selected.capability.id),
    )?;
    let allowed_platform_actions = snapshot
        .content()
        .enabled_capabilities
        .iter()
        .filter(|selected| {
            selected.contribution_lock.source_kind == ContributionSourceKind::PlatformBuiltin
        })
        .filter_map(|selected| {
            snapshot.policy(&selected.capability.id).map(|policy| {
                (
                    selected.capability.id.clone(),
                    policy.allowed_actions.clone(),
                )
            })
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut exposures = standard_agent_tool_exposures();
    retain_exact_platform_actions(&mut exposures, &active.active, &allowed_platform_actions);
    for exposure in &mut exposures {
        let capability = registry
            .capability(&exposure.capability_id)
            .ok_or_else(|| error("tool is not materialized"))?;
        let action = capability
            .manifest
            .contributions
            .actions
            .iter()
            .find(|action| action.action_id == exposure.action_id)
            .ok_or_else(|| error("tool action is unavailable"))?;
        let schema = nomifun_agent_domain_wave2::resolve_action_schema(
            exposure.capability_id.as_ref(),
            &action.input_schema,
        )
        .map_err(error)?;
        // Every standard builtin has a concrete contract. Fail assembly if a
        // future owner accidentally falls back to an open/opaque object again;
        // otherwise the model loses the fields needed to invoke the tool.
        if !concrete_object_schema(
            &schema.0,
            exposure.action_id.as_ref() == "workspace.vcs/status",
        ) {
            return Err(error(format!(
                "{} has no strict canonical tool schema",
                exposure.capability_id.as_ref()
            )));
        }
        exposure.definition.input_schema = schema;
    }
    for selected in snapshot
        .content()
        .enabled_capabilities
        .iter()

    {
        if active.active.contains(&selected.capability.id)
            && selected.contribution_lock.source_kind == ContributionSourceKind::McpBinding
        {
            let tool = super::nomi_core_mcp_catalog::frozen_tool(registry, selected)?;
            if !snapshot.content().mcp_tool_locks.contains(&tool.lock) {
                return Err(error(
                    "MCP tool descriptor differs from its Snapshot mapping",
                ));
            }
            let action = tool.action();
            let suffix = Sha256::digest(selected.capability.id.as_ref().as_bytes());
            exposures.push(AgentToolExposure {
                definition: ChatToolDefinition {
                    name: format!("mcp_{suffix:x}")[..63].to_owned(),
                    description: format!(
                        "{}: {}. Remote MCP tool; executes only through the frozen platform grant.",
                        tool.display_name, tool.description
                    ),
                    input_schema: nomifun_agent_contracts::StrictJsonValue(tool.input_schema),
                    deferred: false,
                },
                capability_id: selected.capability.id.clone(),
                action_id: action,
            });
            continue;
        }
        if !active.active.contains(&selected.capability.id)
            || selected.contribution_lock.source_kind != ContributionSourceKind::PluginMount
        {
            continue;
        }
        let capability = registry
            .capability(&selected.capability.id)
            .ok_or_else(|| error("selected Plugin capability is unavailable"))?;
        let policy = snapshot
            .policy(&selected.capability.id)
            .ok_or_else(|| error("selected Plugin capability has no authority policy"))?;
        if capability.manifest.contributions.actions.is_empty() {
            return Err(error(format!(
                "{} has no function-tool action; Nomi has not admitted its lifecycle",
                selected.capability.id.as_ref()
            )));
        }
        if capability.manifest.display.name.len() > 256
            || capability.manifest.display.description.len() > 4096
        {
            return Err(error("Plugin tool description exceeds context bounds"));
        }
        let start = exposures.len();
        for action in admitted_plugin_actions(&capability.manifest, &policy.allowed_actions) {
            let schema = plugin_schemas
                .resolve(selected, &action.input_schema)
                .await
                .map_err(error)?;
            // Fixed-length, provider-safe and collision-resistant. Mapping is
            // still locked to the full canonical capability/action identity.
            let identity = format!(
                "{}\0{}",
                selected.capability.id.as_ref(),
                action.action_id.as_ref()
            );
            let name = format!("plugin_{:x}", Sha256::digest(identity.as_bytes()));
            exposures.push(AgentToolExposure {
                definition: ChatToolDefinition {
                    name: name[..63].to_owned(),
                    description: format!("{}: {}\nCapability {}, action {}. Executes through the Agent's frozen platform authority.", capability.manifest.display.name, capability.manifest.display.description, selected.capability.id.as_ref(), action.action_id.as_ref()),
                    input_schema: schema,
                    deferred: false,
                },
                capability_id: selected.capability.id.clone(), action_id: action.action_id.clone(),
            });
        }
        if exposures.len() == start {
            return Err(error(
                "selected Plugin has no allowed function-tool actions",
            ));
        }
    }
    if exposures.len() > 128 {
        return Err(error(
            "selected tool surface exceeds 128 actions; narrow the Agent selection",
        ));
    }
    compile_agent_tool_plan(snapshot, active, registry, exposures).map_err(error)
}

fn reject_retired_extension_authority<'a>(
    capability_ids: impl IntoIterator<Item = &'a CapabilityId>,
) -> Result<(), AppError> {
    if let Some(retired) = capability_ids.into_iter().find(|capability_id| {
        nomifun_agent_contracts::is_retired_extension_authoring_capability(
            capability_id.as_ref(),
        )
    }) {
        return Err(error(format!(
            "retired extension authority {} cannot enter a Runtime Session",
            retired.as_ref()
        )));
    }
    Ok(())
}

fn admitted_plugin_actions<'a>(
    manifest: &'a CapabilityManifest,
    allowed_actions: &'a std::collections::BTreeSet<ActionId>,
) -> impl Iterator<Item = &'a nomifun_agent_contracts::CapabilityActionDescriptor> + 'a {
    manifest.contributions.actions.iter().filter(|action| {
        allowed_actions.contains(&action.action_id)
            && !matches!(
                action.presentation,
                ToolPresentationKind::Hidden | ToolPresentationKind::CodeMode
            )
    })
}

fn retain_exact_platform_actions(
    exposures: &mut Vec<AgentToolExposure>,
    active_modules: &std::collections::BTreeSet<CapabilityId>,
    allowed_actions: &std::collections::BTreeMap<CapabilityId, std::collections::BTreeSet<ActionId>>,
) {
    exposures.retain(|exposure| {
        active_modules.contains(&exposure.capability_id)
            && allowed_actions
                .get(&exposure.capability_id)
                .is_some_and(|allowed| allowed.contains(&exposure.action_id))
    });
}

fn concrete_object_schema(schema: &serde_json::Value, allow_empty: bool) -> bool {
    let strict = |schema: &serde_json::Value| {
        schema.get("type").and_then(|value| value.as_str()) == Some("object")
            && schema
                .get("additionalProperties")
                .and_then(|value| value.as_bool())
                == Some(false)
            && schema
                .get("properties")
                .and_then(|value| value.as_object())
                .is_some_and(|properties| allow_empty || !properties.is_empty())
    };
    strict(schema)
        || (schema.get("type").and_then(|value| value.as_str()) == Some("object")
            && schema
                .get("oneOf")
                .and_then(|value| value.as_array())
                .is_some_and(|variants| {
                    !variants.is_empty() && variants.len() <= 16 && variants.iter().all(strict)
                }))
}

pub(super) fn validate_session_mcp(
    snapshot: &nomifun_agent_contracts::ResolvedSnapshotEnvelope,
    resources: &[nomifun_agent_contracts::TypedResourceBinding],
    extra: &serde_json::Value,
) -> Result<(), AppError> {
    super::nomi_core_mcp_catalog::validate_session_selection(snapshot, resources, extra)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use nomifun_agent_contracts::{
        CapabilityActionDescriptor, CapabilityConsumer, CapabilityContributions, CapabilityKind,
        EffectClass, LocalizedMetadata, PackageRef, PlatformConstraint, StrictJsonValue,
        capability_surface_declarations,
    };
    use serde_json::json;

    use super::*;

    #[test]
    fn partial_module_grant_exposes_only_the_exact_allowed_action() {
        let module = CapabilityId::from("workspace.files");
        let mut exposures = standard_agent_tool_exposures();
        retain_exact_platform_actions(
            &mut exposures,
            &BTreeSet::from([module.clone()]),
            &BTreeMap::from([(
                module.clone(),
                BTreeSet::from([ActionId::from("workspace.files/read")]),
            )]),
        );

        assert_eq!(exposures.len(), 1);
        assert_eq!(exposures[0].capability_id, module);
        assert_eq!(exposures[0].action_id.as_ref(), "workspace.files/read");
        assert_eq!(exposures[0].definition.name, "read_file");
    }

    #[test]
    fn display_kind_does_not_change_an_exact_plugin_action_grant() {
        let action_id = ActionId::from("mixed.module/run");
        let mut manifest = CapabilityManifest {
            id: CapabilityId::from("mixed.module"),
            contribution_id: "capability:mixed.module".into(),
            version: "1.0.0".into(),
            kind: CapabilityKind::Tool,
            package: PackageRef {
                id: "fixture.package".into(),
                version: "1.0.0".into(),
            },
            display: LocalizedMetadata {
                name: "Mixed".into(),
                description: "Action plus Context/Event contributions".into(),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            requires: Vec::new(),
            conflicts: Vec::new(),
            supported_surfaces: capability_surface_declarations(
                ["desktop"],
                [CapabilityConsumer::Agent],
            ),
            requires_runtime_features: Vec::new(),
            supported_platforms: vec![PlatformConstraint::Any],
            config_schema: StrictJsonValue(json!({"type":"object"})),
            contributions: CapabilityContributions {
                actions: vec![CapabilityActionDescriptor {
                    action_id: action_id.clone(),
                    input_schema: "schema://mixed/run/input@1".into(),
                    output_schema: "schema://mixed/run/output@1".into(),
                    effect_class: EffectClass::Pure,
                    presentation: ToolPresentationKind::FunctionTool,
                }],
                context_schema_refs: vec!["schema://mixed/context@1".into()],
                event_schema_refs: vec!["schema://mixed/event@1".into()],
                ..Default::default()
            },
        };
        let allowed = BTreeSet::from([action_id.clone()]);
        for kind in [
            CapabilityKind::Tool,
            CapabilityKind::ContextContributor,
            CapabilityKind::EventSource,
        ] {
            manifest.kind = kind;
            assert_eq!(
                admitted_plugin_actions(&manifest, &allowed)
                    .map(|action| action.action_id.clone())
                    .collect::<Vec<_>>(),
                vec![action_id.clone()],
            );
        }
    }

    #[test]
    fn retired_extension_authority_never_enters_the_runtime_surface() {
        let safe = CapabilityId::from("mcp.server");
        reject_retired_extension_authority([&safe]).unwrap();
        for retired in nomifun_agent_contracts::RETIRED_EXTENSION_AUTHORING_CAPABILITY_IDS {
            let retired = CapabilityId::from(retired);
            let error = reject_retired_extension_authority([&retired]).unwrap_err();
            assert!(error.to_string().contains("retired extension authority"));
        }
    }
}
