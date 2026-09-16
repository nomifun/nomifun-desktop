//! Projection of selected, exact Snapshot contributions into Nomi tools.
//! No global catalog discovery and no executable/runtime loading occurs here.
use nomifun_agent_contracts::{CapabilityKind, ContributionSourceKind, ToolPresentationKind};
use nomifun_agent_kernel::{ActiveCapabilitySetSnapshot, CompiledSnapshot, MaterializedRegistry};
use nomifun_ai_agent::NomiPluginToolSchemaResolver;
use nomifun_chat_model_broker::ChatToolDefinition;
use nomifun_coding_engine::{
    CodingToolExposure, CodingToolPlan, compile_coding_tool_plan, standard_coding_tool_exposures,
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
) -> Result<CodingToolPlan, AppError> {
    if snapshot.registry_generation != registry.generation
        || snapshot.registry_digest != registry.registry_digest
    {
        return Err(error("registry changed after Snapshot compilation"));
    }
    if snapshot.content().enabled_capabilities.len() > 128
    {
        return Err(error("Nomi supports at most 128 selected capabilities"));
    }
    let mut exposures = standard_coding_tool_exposures();
    exposures.retain(|item| {
        active.active.contains(&item.capability_id)
            && snapshot
                .content()
                .enabled_capabilities
                .iter()

                .any(|selected| {
                    selected.capability.id == item.capability_id
                        && selected.contribution_lock.source_kind
                            == ContributionSourceKind::PlatformBuiltin
                })
    });
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
            exposures.push(CodingToolExposure {
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
        if capability.manifest.kind != CapabilityKind::Tool
            || capability.manifest.contributions.actions.is_empty()
        {
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
        for action in &capability.manifest.contributions.actions {
            if !policy.allowed_actions.contains(&action.action_id)
                || matches!(
                    action.presentation,
                    ToolPresentationKind::Hidden | ToolPresentationKind::CodeMode
                )
            {
                continue;
            }
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
            exposures.push(CodingToolExposure {
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
    compile_coding_tool_plan(snapshot, active, registry, exposures).map_err(error)
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
