//! Presentation within an already compiled frozen ToolPlan, never authority
//! selection. Large catalogs retain their full capabilities and exact schemas.
use nomifun_agent_runtime::AgentToolPlan;
use nomifun_common::AppError;

const INITIAL_TOOL_BYTES: usize = 24 * 1024;
/// User-installed Plugin Actions stay directly visible while their own
/// serialized definitions fit this budget; beyond it they defer like other
/// heavy tools.
const PLUGIN_TOOL_BYTES: usize = 8 * 1024;

pub(super) fn project(plan: AgentToolPlan, discovery_available: bool) -> Result<AgentToolPlan, AppError> {
    let bytes = serde_json::to_vec(&plan.model_definitions())
        .map_err(|error| AppError::Internal(error.to_string()))?.len();
    if !discovery_available || bytes <= INITIAL_TOOL_BYTES {
        return Ok(plan);
    }
    let plugin_bytes = serde_json::to_vec(&plan.model_definitions().iter()
        .filter(|definition| plan.binding(&definition.name).is_some_and(|binding|
            binding.capability_id.as_ref()
                .starts_with(nomifun_agent_contracts::plugin::PLUGIN_ACTION_ID_PREFIX)))
        .collect::<Vec<_>>())
        .map_err(|error| AppError::Internal(error.to_string()))?.len();
    let defer_plugins = plugin_bytes > PLUGIN_TOOL_BYTES;
    AgentToolPlan::new(plan.model_definitions().iter().map(|definition| {
        let mut binding = plan.binding(&definition.name).expect("compiled tool has a binding").clone();
        // Keep ordinary workspace work and the general-purpose web/delegation
        // entry points ready. Heavy business/provider schemas remain in the
        // exact same plan and are revealed by the selected ToolSearch policy.
        // User-installed Agent tools fit the same rule: within the plugin
        // budget they stay directly visible, beyond it they defer like any
        // heavy schema — deferral never alters the frozen binding identity.
        let plugin_tool = binding.capability_id.as_ref()
            .starts_with(nomifun_agent_contracts::plugin::PLUGIN_ACTION_ID_PREFIX);
        if plugin_tool {
            if defer_plugins {
                binding.definition.deferred = true;
            }
        } else if !matches!(binding.capability_id.as_ref(),
            "workspace.files" | "workspace.vcs" | "workspace.process" | "web.research" | "agent.collaboration") {
            binding.definition.deferred = true;
        }
        binding
    })).map_err(|error| AppError::Internal(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use nomifun_agent_contracts::{
        ActionId, CanonicalSchemaRef, CapabilityId, DigestHex, PluginId, StrictJsonValue,
        plugin::plugin_action_id,
    };
    use nomifun_chat_model_broker::ChatToolDefinition;
    use nomifun_engine_core::{
        EngineEffectClass, EngineToolBinding, input_schema_digest,
    };
    use serde_json::json;

    use super::*;

    fn binding(name: &str, capability_id: &str, description: &str) -> EngineToolBinding {
        let schema = StrictJsonValue(json!({
            "type": "object",
            "properties": {"text": {"type": "string"}}
        }));
        EngineToolBinding {
            model_name: name.to_owned(),
            definition: ChatToolDefinition {
                name: name.to_owned(),
                description: description.to_owned(),
                input_schema: schema.clone(),
                deferred: false,
            },
            schema_digest: input_schema_digest(&schema).unwrap(),
            canonical_input_schema_ref: CanonicalSchemaRef::from(format!(
                "schema://{capability_id}/{name}/input"
            )),
            capability_contract_digest: DigestHex::from("b".repeat(64)),
            capability_id: CapabilityId::from(capability_id),
            action_id: ActionId::from(format!("{capability_id}/{name}")),
            resource_binding_ids: BTreeSet::new(),
            effect_class: EngineEffectClass::ReadOnly,
            parallel_safe: true,
        }
    }

    #[test]
    fn plugin_actions_stay_directly_visible_on_a_deferred_surface() {
        let plugin_capability = plugin_action_id(
            &PluginId::from("019b0000-0000-7000-8000-000000000123"),
            "trim_and_uppercase",
        );
        assert!(plugin_capability.starts_with("plugin:"));
        let plan = AgentToolPlan::new([
            binding("plugin_trim", &plugin_capability, "Plugin Action"),
            binding("crm_lookup", "crm.contacts", &"x".repeat(INITIAL_TOOL_BYTES)),
            binding("files_read", "workspace.files", "read"),
        ])
        .unwrap();
        let projected = project(plan, true).unwrap();
        assert!(!projected.binding("plugin_trim").unwrap().definition.deferred);
        assert!(projected.binding("crm_lookup").unwrap().definition.deferred);
        assert!(!projected.binding("files_read").unwrap().definition.deferred);
    }

    /// Beyond the plugin budget installed Plugin Actions defer like other
    /// heavy tools; the frozen binding identity stays exact.
    #[test]
    fn plugin_actions_beyond_the_visible_budget_are_deferred() {
        let plugin_capability = |action: &str| {
            plugin_action_id(
                &PluginId::from("019b0000-0000-7000-8000-000000000123"),
                action,
            )
        };
        let plan = AgentToolPlan::new([
            binding("plugin_alpha", &plugin_capability("alpha"), &"a".repeat(PLUGIN_TOOL_BYTES)),
            binding("plugin_beta", &plugin_capability("beta"), &"b".repeat(PLUGIN_TOOL_BYTES)),
            binding("plugin_gamma", &plugin_capability("gamma"), &"g".repeat(PLUGIN_TOOL_BYTES)),
            binding("files_read", "workspace.files", "read"),
        ])
        .unwrap();
        let projected = project(plan, true).unwrap();
        for name in ["plugin_alpha", "plugin_beta", "plugin_gamma"] {
            let deferred = projected.binding(name).unwrap();
            assert!(deferred.definition.deferred, "{name}");
            assert!(deferred.capability_id.as_ref().starts_with("plugin:"));
        }
        assert!(!projected.binding("files_read").unwrap().definition.deferred);
    }
}
