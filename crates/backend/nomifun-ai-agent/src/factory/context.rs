//! Workspace resolution + per-agent metadata shared across factory
//! builders. Produced by `FactoryContext::resolve` at the top of
//! `build_agent`, then passed into the per-agent `build(..)` functions.

use nomifun_common::{AgentType, AppError, ConversationId, validate_prefixed_id};

use crate::factory::AgentFactoryDeps;
use crate::types::AgentRuntimeBuildOptions;

const TEMP_WORKSPACE_ID_EXTRA_KEY: &str = "temp_workspace_id";

pub(super) struct FactoryContext {
    pub conversation_id: String,
    pub workspace: String,
    pub is_custom_workspace: bool,
}

impl FactoryContext {
    pub async fn resolve(deps: &AgentFactoryDeps, options: &AgentRuntimeBuildOptions) -> Result<Self, AppError> {
        ConversationId::parse(&options.conversation_id)
            .map_err(|error| AppError::BadRequest(format!("invalid Agent runtime conversation id: {error}")))?;
        let conversation_id = options.conversation_id.clone();

        // `is_custom_workspace` is the authoritative signal for "user
        // chose this path" — determined here and plumbed down to the
        // managers that care (currently AcpAgentManager, for first-message
        // injection). Do NOT re-derive it from the workspace string later:
        // user paths may incidentally contain "conversations" or "-temp-".
        //
        // A canonical `temp_workspace_id` is the durable marker for a
        // backend-managed workspace. Always rebase that workspace under this
        // installation's current `work_dir`; the persisted absolute workspace
        // may point at the source installation after restore/import.
        let (workspace, is_custom_workspace) = if options
            .extra
            .get(TEMP_WORKSPACE_ID_EXTRA_KEY)
            .is_some()
            || options.workspace.trim().is_empty()
        {
            let temp_workspace_id = temp_workspace_id_for_options(options)?;
            let label = workspace_label(&options.agent_type, options.extra.get("backend"));
            let dir = deps
                .work_dir
                .join("conversations")
                .join(format!("{label}-temp-{temp_workspace_id}"));
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

fn temp_workspace_id_for_options(
    options: &AgentRuntimeBuildOptions,
) -> Result<&str, AppError> {
    let value = options
        .extra
        .get(TEMP_WORKSPACE_ID_EXTRA_KEY)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::Internal(format!(
                "conversation {} has no canonical temp_workspace_id for its managed workspace",
                options.conversation_id
            ))
        })?;
    validate_prefixed_id(value, "ws").map_err(|error| {
        AppError::Internal(format!(
            "conversation {} has invalid temp_workspace_id '{value}': {error}",
            options.conversation_id
        ))
    })?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const WORKSPACE_ID: &str = "ws_0190f5fe-7c00-7a00-8abc-012345678901";

    fn options(extra: serde_json::Value) -> AgentRuntimeBuildOptions {
        AgentRuntimeBuildOptions {
            user_id: "user_0190f5fe-7c00-7a00-8000-000000000001".into(),
            agent_type: AgentType::Acp,
            workspace: String::new(),
            model: None,
            conversation_id: "conv_0190f5fe-7c00-7a00-8abc-012345678901".into(),
            delegation_policy: Default::default(),
            extra,
            conversation_created_at: Some(10),
        }
    }

    #[test]
    fn temp_workspace_id_accepts_backend_minted_canonical_token() {
        let opts = options(json!({
            "temp_workspace_id": WORKSPACE_ID,
            "backend": "claude"
        }));
        assert_eq!(temp_workspace_id_for_options(&opts).unwrap(), WORKSPACE_ID);
    }

    #[test]
    fn missing_or_malformed_temp_workspace_id_fails_closed() {
        for extra in [
            json!({ "backend": "claude" }),
            json!({ "backend": "claude", "temp_workspace_id": "" }),
            json!({ "backend": "claude", "temp_workspace_id": "ws_abc" }),
            json!({ "backend": "claude", "temp_workspace_id": 7 }),
        ] {
            let error = temp_workspace_id_for_options(&options(extra)).unwrap_err();
            assert!(matches!(error, AppError::Internal(message) if message.contains("temp_workspace_id")));
        }
    }

    #[test]
    fn managed_workspace_path_rebases_under_current_work_dir() {
        let work_dir =
            std::env::temp_dir().join(format!("nomifun-factory-rebase-{}", nomifun_common::generate_id()));
        let mut opts = options(json!({
            "backend": "claude",
            "temp_workspace_id": WORKSPACE_ID,
            "workspace": "/source-install/conversations/claude-temp-stale"
        }));
        opts.workspace =
            "/source-install/conversations/claude-temp-stale".to_owned();

        let temp_workspace_id = temp_workspace_id_for_options(&opts).unwrap();
        let workspace = work_dir
            .join("conversations")
            .join(format!(
                "{}-temp-{temp_workspace_id}",
                workspace_label(&opts.agent_type, opts.extra.get("backend"))
            ));

        assert_eq!(
            workspace,
            work_dir
                .join("conversations")
                .join(format!("claude-temp-{WORKSPACE_ID}"))
        );
        assert!(!workspace.starts_with("/source-install"));
    }
}
