use std::sync::Arc;

use nomifun_common::{AgentType, AppError};

use crate::agent_task::AgentInstance;
use crate::factory::AgentFactoryDeps;
use crate::factory::context::FactoryContext;
use crate::manager::nanobot::NanobotAgentManager;
use crate::types::BuildTaskOptions;

pub(super) async fn build(
    deps: Arc<AgentFactoryDeps>,
    _options: BuildTaskOptions,
    ctx: FactoryContext,
) -> Result<AgentInstance, AppError> {
    // Nanobot lives in the catalog as an internal row; reuse the
    // registry-resolved path instead of re-running `which()`.
    let cli_path = deps
        .agent_registry
        .list_by_agent_type(AgentType::Nanobot)
        .await
        .into_iter()
        .find_map(|m| m.resolved_command)
        .ok_or_else(|| AppError::BadRequest("Nanobot CLI not found in PATH".into()))?;
    let agent = NanobotAgentManager::new(ctx.conversation_id, ctx.workspace, cli_path, deps.data_dir.clone()).await?;
    Ok(AgentInstance::Nanobot(Arc::new(agent)))
}
