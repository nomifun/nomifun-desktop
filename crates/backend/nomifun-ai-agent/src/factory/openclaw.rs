use std::sync::Arc;

use nomifun_api_types::OpenClawBuildExtra;
use nomifun_common::{AgentType, AppError};

use crate::agent_task::AgentInstance;
use crate::factory::AgentFactoryDeps;
use crate::factory::context::FactoryContext;
use crate::manager::openclaw::OpenClawAgentManager;
use crate::types::BuildTaskOptions;

pub(super) async fn build(
    deps: Arc<AgentFactoryDeps>,
    options: BuildTaskOptions,
    ctx: FactoryContext,
) -> Result<AgentInstance, AppError> {
    let mut config: OpenClawBuildExtra = serde_json::from_value(options.extra)
        .map_err(|e| AppError::BadRequest(format!("Invalid OpenClaw build options: {e}")))?;

    // OpenClaw lives in the catalog as an internal row; reuse
    // the registry-resolved path instead of re-running `which()`.
    if config.gateway.cli_path.is_none()
        && let Some(cli) = deps
            .agent_registry
            .list_by_agent_type(AgentType::OpenclawGateway)
            .await
            .into_iter()
            .find_map(|m| m.resolved_command)
            .map(|p| p.to_string_lossy().into_owned())
    {
        config.gateway.cli_path = Some(cli);
    }

    let resume_session_key = config.session_key.clone();
    let agent = OpenClawAgentManager::new(
        ctx.conversation_id,
        ctx.workspace,
        config,
        resume_session_key,
        deps.data_dir.clone(),
    )
    .await?;
    let arc = Arc::new(agent);
    arc.start_event_relay();
    Ok(AgentInstance::OpenClaw(arc))
}
