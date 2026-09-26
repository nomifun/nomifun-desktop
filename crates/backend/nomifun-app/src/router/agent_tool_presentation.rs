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
