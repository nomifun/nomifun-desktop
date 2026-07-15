//! Agent-session operations on ConversationService.
//!
//! These forward to the active AgentRuntimeHandle (via `self.runtime_handle(id)`) for
//! mode/model/usage/slash-commands/side-question/openclaw-runtime queries,
//! plus workspace browsing that needs the conversations.extra.workspace
//! field.
//!
//! Kept in a separate file from service.rs to avoid pushing that file
//! over 2000 lines.

use nomifun_api_types::{
    AgentModeResponse, GetModelInfoResponse, SetModeRequest, SetModelRequest, SideQuestionRequest,
    SideQuestionResponse, SlashCommandItem, WorkspaceBrowseQuery, WorkspaceEntry,
};
use nomifun_common::AppError;
use nomifun_file::list_workspace_level;
use nomifun_db::models::ConversationRow;

use crate::service::ConversationService;

impl ConversationService {
    async fn require_owned_conversation(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<ConversationRow, AppError> {
        self.conversation_repo()
            .get(conversation_id)
            .await?
            .filter(|row| row.user_id == user_id)
            .ok_or_else(|| {
                AppError::NotFound(format!("Conversation '{conversation_id}' not found"))
            })
    }

    // ── Mode ────────────────────────────────────────────────────────

    pub async fn get_mode(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<AgentModeResponse, AppError> {
        self.require_owned_conversation(user_id, conversation_id)
            .await?;
        self.runtime_handle(conversation_id)?.get_mode().await
    }

    pub async fn set_mode(
        &self,
        user_id: &str,
        conversation_id: &str,
        req: SetModeRequest,
    ) -> Result<(), AppError> {
        self.require_owned_conversation(user_id, conversation_id)
            .await?;
        if req.mode.trim().is_empty() {
            return Err(AppError::BadRequest("mode must not be empty".into()));
        }
        self.runtime_handle(conversation_id)?.set_mode(&req.mode).await
    }

    // ── Model ───────────────────────────────────────────────────────

    pub async fn get_model(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<GetModelInfoResponse, AppError> {
        self.require_owned_conversation(user_id, conversation_id)
            .await?;
        self.runtime_handle(conversation_id)?.get_model().await
    }

    pub async fn set_model(
        &self,
        user_id: &str,
        conversation_id: &str,
        req: SetModelRequest,
    ) -> Result<(), AppError> {
        self.require_owned_conversation(user_id, conversation_id)
            .await?;
        if req.model_id.trim().is_empty() {
            return Err(AppError::BadRequest("model_id must not be empty".into()));
        }
        let runtime = match self.runtime_handle(conversation_id) {
            Ok(runtime) => runtime,
            Err(err) => {
                tracing::warn!(
                    conversation_id,
                    model_id = %req.model_id,
                    error_code = err.error_code(),
                    "Set model skipped because active Agent runtime is unavailable"
                );
                return Err(err);
            }
        };
        runtime.set_model(&req.model_id).await
    }

    // ── Usage / Slash commands ──────────────────────────────────────

    pub async fn get_usage(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<Option<serde_json::Value>, AppError> {
        self.require_owned_conversation(user_id, conversation_id)
            .await?;
        self.runtime_handle(conversation_id)?.get_usage().await
    }

    pub async fn get_slash_commands(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<Vec<SlashCommandItem>, AppError> {
        self.require_owned_conversation(user_id, conversation_id)
            .await?;
        self.runtime_handle(conversation_id)?.get_slash_commands().await
    }

    // ── Side question ───────────────────────────────────────────────

    pub async fn handle_side_question(
        &self,
        user_id: &str,
        conversation_id: &str,
        req: SideQuestionRequest,
    ) -> Result<SideQuestionResponse, AppError> {
        self.require_owned_conversation(user_id, conversation_id)
            .await?;
        // `AgentRuntimeHandle::handle_side_question` already validates that the
        // question is non-empty; no need to duplicate the check here.
        self.runtime_handle(conversation_id)?.handle_side_question(req).await
    }

    // ── OpenClaw runtime diagnostics ────────────────────────────────

    pub async fn get_openclaw_runtime(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<serde_json::Value, AppError> {
        self.require_owned_conversation(user_id, conversation_id)
            .await?;
        self.runtime_handle(conversation_id)?.get_openclaw_runtime().await
    }

    // ── Workspace browsing ──────────────────────────────────────────

    /// Enumerate entries under `query.path` inside the conversation's
    /// workspace root. Resolves the root from the conversation's
    /// `extra.workspace` and delegates the path-scoped listing (isolation
    /// guards + depth cap) to [`nomifun_file::list_workspace_level`].
    pub async fn browse_workspace(
        &self,
        user_id: &str,
        conversation_id: &str,
        query: WorkspaceBrowseQuery,
    ) -> Result<Vec<WorkspaceEntry>, AppError> {
        if query.path.trim().is_empty() {
            return Err(AppError::BadRequest("path must not be empty".into()));
        }

        let row = self
            .require_owned_conversation(user_id, conversation_id)
            .await?;

        let extra: serde_json::Value =
            serde_json::from_str(&row.extra).map_err(|e| AppError::Internal(format!("Invalid extra JSON: {e}")))?;
        let workspace = extra
            .get("workspace")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_owned();
        if workspace.is_empty() {
            return Err(AppError::BadRequest("Conversation has no workspace assigned".into()));
        }

        list_workspace_level(
            std::path::Path::new(&workspace),
            &query.path,
            query.search.as_deref(),
        )
    }
}
