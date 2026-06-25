//! Workspace resolution + per-agent metadata shared across factory
//! builders. Produced by `FactoryContext::resolve` at the top of
//! `build_agent`, then passed into the per-agent `build(..)` functions.

use nomifun_common::{AgentType, AppError};

use crate::factory::AgentFactoryDeps;
use crate::types::BuildTaskOptions;

pub(super) struct FactoryContext {
    pub conversation_id: String,
    pub workspace: String,
    pub is_custom_workspace: bool,
}

impl FactoryContext {
    pub async fn resolve(deps: &AgentFactoryDeps, options: &BuildTaskOptions) -> Result<Self, AppError> {
        let conversation_id = options.conversation_id.clone();

        // `is_custom_workspace` is the authoritative signal for "user
        // chose this path" — determined here and plumbed down to the
        // managers that care (currently AcpAgentManager, for first-message
        // injection). Do NOT re-derive it from the workspace string later:
        // user paths may incidentally contain "conversations" or "-temp-".
        let (workspace, is_custom_workspace) = if options.workspace.is_empty() {
            // Fallback workspace path: kept in sync with
            // ConversationService::create, which places auto-provisioned
            // workspaces under `{work_dir}/conversations/{label}-temp-{id}/`.
            // Reaching this branch means the caller did not supply an
            // `extra.workspace` — construct the same `{label}-temp-{id}`
            // layout so logs, cleanup scripts, and the frontend's "is this a
            // managed temp dir?" heuristic all see a single naming scheme.
            let label = workspace_label(&options.agent_type, options.extra.get("backend"));
            let dir = deps
                .work_dir
                .join("conversations")
                .join(format!("{label}-temp-{conversation_id}"));
            std::fs::create_dir_all(&dir)
                .map_err(|e| AppError::Internal(format!("Failed to create temp workspace: {e}")))?;
            (dir.to_string_lossy().into_owned(), false)
        } else {
            (options.workspace.clone(), true)
        };

        Ok(Self {
            conversation_id,
            workspace,
            is_custom_workspace,
        })
    }
}

/// Label used in auto-provisioned temp workspace directory names.
///
/// For ACP conversations the label is the vendor string from
/// `extra.backend` (e.g. `"claude"`); otherwise the agent type's serde
/// name (e.g. `"nomi"`). Must stay in sync with
/// `ConversationService::create`'s `conversation_label`.
fn workspace_label(agent_type: &AgentType, backend: Option<&serde_json::Value>) -> String {
    if *agent_type == AgentType::Acp
        && let Some(serde_json::Value::String(s)) = backend
        && !s.is_empty()
    {
        return s.clone();
    }
    agent_type.serde_name().to_owned()
}
