//! Repository trait for the `acp_session` table.
//!
//! Each ACP-type conversation owns exactly one `acp_session` row. The
//! row is created alongside the conversation (not on first message) so
//! the runtime-state write path can assume the row exists.
//!
//! `session_config` is a JSON blob that carries everything that is not
//! session identity. Under the `"runtime"` key it holds the user's last
//! per-session choices: current mode, current model, config selections,
//! context usage. `AcpAgentService` updates those fields through
//! [`IAcpSessionRepository::save_runtime_state`] and
//! `AcpAgentManager` preloads them on resume through
//! [`IAcpSessionRepository::load_runtime_state`].

use crate::error::DbError;
use crate::models::AcpSessionRow;

/// Parameters for [`IAcpSessionRepository::create`].
///
/// `session_id` stays `None` until the CLI returns one (first
/// `session/new` or `session/load`), at which point the caller flips
/// it through [`IAcpSessionRepository::update_session_id`].
#[derive(Debug, Clone)]
pub struct CreateAcpSessionParams<'a> {
    pub conversation_id: i64,
    pub agent_backend: &'a str,
    pub agent_source: &'a str,
    pub agent_id: &'a str,
}

/// The decoded `session_config.runtime` payload. See module docs.
///
/// All fields are optional because we persist partials — the service
/// may write just the mode or just the usage without touching siblings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersistedSessionState {
    pub current_mode_id: Option<String>,
    pub current_model_id: Option<String>,
    /// JSON-encoded map of `config_id -> value`. Stored as a raw string
    /// so the repository layer does not have to know the shape.
    pub config_selections_json: Option<String>,
    /// JSON-encoded `UsageUpdate`. Same rationale as
    /// `config_selections_json`.
    pub context_usage_json: Option<String>,
}

/// Partial update for [`IAcpSessionRepository::save_runtime_state`].
///
/// `Option<Option<_>>` lets callers distinguish "leave untouched"
/// (outer `None`) from "clear to null" (inner `None`).
#[derive(Debug, Clone, Default)]
pub struct SaveRuntimeStateParams<'a> {
    pub current_mode_id: Option<Option<&'a str>>,
    pub current_model_id: Option<Option<&'a str>>,
    pub config_selections_json: Option<Option<&'a str>>,
    pub context_usage_json: Option<Option<&'a str>>,
}

impl SaveRuntimeStateParams<'_> {
    pub fn is_empty(&self) -> bool {
        self.current_mode_id.is_none()
            && self.current_model_id.is_none()
            && self.config_selections_json.is_none()
            && self.context_usage_json.is_none()
    }
}

#[async_trait::async_trait]
pub trait IAcpSessionRepository: Send + Sync {
    /// Fetch the full row by conversation id.
    async fn get(&self, conversation_id: i64) -> Result<Option<AcpSessionRow>, DbError>;

    /// Insert a fresh `acp_session` row. Called by `ConversationService`
    /// when an ACP-type conversation is created; primary-key conflict
    /// surfaces as `DbError::Conflict`.
    async fn create(&self, params: &CreateAcpSessionParams<'_>) -> Result<AcpSessionRow, DbError>;

    /// Record the CLI-assigned `session_id` after `session/new` or
    /// `session/load` succeeds. Returns `true` when the row existed.
    async fn update_session_id(&self, conversation_id: i64, session_id: &str) -> Result<bool, DbError>;

    /// Forget the CLI session for a conversation: NULL the `session_id`,
    /// reset `session_status` to `idle`, and drop the cached
    /// `session_config.runtime.context_usage` so the token meter reflects a
    /// fresh start. Used by the "clear context" flow — after this, the next
    /// prompt re-issues `session/new` instead of resuming. Returns `true`
    /// when the row existed.
    async fn clear_session_id(&self, conversation_id: i64) -> Result<bool, DbError>;

    /// Delete the row. Called by the conversation delete hook — no DB
    /// foreign key, so this must be invoked explicitly.
    async fn delete(&self, conversation_id: i64) -> Result<bool, DbError>;

    /// Decode and return the `session_config.runtime` sub-object.
    /// Returns `None` when the row does not exist or the JSON lacks a
    /// `runtime` key; returns `Some(Default::default())` when the key
    /// is present but empty.
    async fn load_runtime_state(&self, conversation_id: i64) -> Result<Option<PersistedSessionState>, DbError>;

    /// Merge a partial runtime update into `session_config.runtime`.
    /// Assumes the row exists (created alongside the conversation);
    /// returns `Ok(false)` when it does not.
    async fn save_runtime_state(
        &self,
        conversation_id: i64,
        params: &SaveRuntimeStateParams<'_>,
    ) -> Result<bool, DbError>;
}
