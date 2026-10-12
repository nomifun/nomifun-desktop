//! Presentation within an already compiled frozen ToolPlan, never authority
//! selection. Large catalogs retain their full capabilities and exact schemas.
use nomifun_agent_runtime::AgentToolPlan;
use nomifun_common::AppError;

const INITIAL_TOOL_BYTES: usize = 24 * 1024;
pub(super) fn project(plan: AgentToolPlan, discovery_available: bool) -> Result<AgentToolPlan, AppError> {
    let bytes = serde_json::to_vec(&plan.model_definitions())
        .map_err(|error| AppError::Internal(error.to_string()))?.len();
    if !discovery_available || bytes <= INITIAL_TOOL_BYTES {
        return Ok(plan);
    }
    AgentToolPlan::new(plan.model_definitions().iter().map(|definition| {
        let mut binding = plan.binding(&definition.name).expect("compiled tool has a binding").clone();
        // Keep ordinary workspace work and the general-purpose web/delegation
        // entry points ready. Heavy business/provider schemas remain in the
        // exact same plan and are revealed by the selected ToolSearch policy.
        if !matches!(binding.capability_id.as_ref(),
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
        ActionId, CanonicalSchemaRef, CapabilityId, DigestHex, StrictJsonValue,
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
    fn large_catalog_preserves_every_exact_tool_and_defers_heavy_schemas() {
        let plan = AgentToolPlan::new([
            binding("mcp_lookup", "nomi.mcp.v1.example", "Application MCP tool"),
            binding("crm_lookup", "crm.contacts", &"x".repeat(INITIAL_TOOL_BYTES)),
            binding("files_read", "workspace.files", "read"),
        ]).unwrap();
        let projected = project(plan.clone(), true).unwrap();
        assert_eq!(projected.len(), plan.len());
        for name in ["mcp_lookup", "crm_lookup"] {
            let mut binding = projected.binding(name).unwrap().clone();
            assert!(binding.definition.deferred);
            binding.definition.deferred = false;
            assert_eq!(&binding, plan.binding(name).unwrap());
        }
        assert!(!projected.binding("files_read").unwrap().definition.deferred);
    }
}
