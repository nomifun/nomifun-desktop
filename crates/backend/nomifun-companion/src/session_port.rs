//! Typed Session boundary for Companion thread management.

use std::sync::Arc;

use async_trait::async_trait;

use nomifun_api_types::{ConversationResponse, CreateConversationRequest};
use nomifun_common::AppError;

use nomifun_db::MessageDayBucket;

use crate::archive_port::{CompanionArchiveSessionPort, SessionArchivePort};
use crate::archiver::ArchiveConversationPort;

use crate::evolution::TranscriptSource;

/// The two concurrent execution roles of one Companion identity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionRole {
    #[default]
    Companion,
    Robot,
}

impl CompanionRole {
    pub fn as_str(self) -> &'static str {
        match self { Self::Companion => "companion", Self::Robot => "robot" }
    }
}

impl TryFrom<&str> for CompanionRole {
    type Error = AppError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "companion" => Ok(Self::Companion),
            "robot" => Ok(Self::Robot),
            _ => Err(AppError::BadRequest(format!("invalid companion role '{value}'"))),
        }
    }
}

/// Host-authored creation scope. The Session owner records this in its typed
/// canonical opening; mutable UI metadata and the thread registry cannot grant
/// ownership or change a role. Retrying a command uses the same creation key.
#[derive(Debug, Clone)]
pub struct CompanionSessionCreateContext {
    pub companion_id: String,
    pub role: CompanionRole,
    pub robot_id: Option<String>,
    pub idempotency_key: String,
    /// None preserves the desktop role's saved product selection. The robot
    /// role supplies its independent explicit selection, including empty.
    pub mcp_server_ids: Option<Vec<String>>,
}

/// Keeps the host's admission and workspace lifecycle fences alive while the
/// domain removes the settled shared directory and Companion profile.
pub struct CompanionDeletionGuard {
    _resources: Box<dyn Send + Sync>,
    managed_workspaces: Vec<String>,
}

impl std::fmt::Debug for CompanionDeletionGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CompanionDeletionGuard")
    }
}

impl CompanionDeletionGuard {
    pub fn new<T: Send + Sync + 'static>(resources: T) -> Self {
        Self::with_workspaces(resources, Vec::new())
    }

    pub fn with_workspaces<T: Send + Sync + 'static>(resources: T, managed_workspaces: Vec<String>) -> Self {
        Self { _resources: Box::new(resources), managed_workspaces }
    }

    pub fn managed_workspaces(&self) -> &[String] {
        &self.managed_workspaces
    }
}

#[derive(Debug, Clone)]
pub struct CompanionHistoryRecallQuery {
    pub query: String,
    pub limit: u32,
    pub byte_budget: usize,
    pub source_session_id: Option<String>,
    pub since_ts: Option<i64>,
    pub until_ts: Option<i64>,
    pub before: Option<CompanionHistoryRecallCursor>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompanionHistoryRecallCursor {
    pub recorded_at: i64,
    pub session_id: String,
    pub source_seq: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CompanionHistoryFragment {
    pub session_id: String,
    pub turn_id: String,
    pub event_ids: Vec<String>,
    pub recorded_at: i64,
    pub text: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CompanionHistoryRecallPage {
    pub fragments: Vec<CompanionHistoryFragment>,
    pub has_more: bool,
    pub next_cursor: Option<CompanionHistoryRecallCursor>,
    pub truncated: bool,
}

/// Narrow command/query surface used by Companion thread management.
#[async_trait]
pub trait CompanionSessionPort: Send + Sync {
    /// Explicit ensure/new registers the proven shared workspace location.
    /// Reads and frozen Agent authorization remain unchanged.
    async fn prepare_companion_workspace(
        &self, owner_id: &str, companion_id: &str,
    ) -> Result<(), AppError>;

    async fn get(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<ConversationResponse, AppError>;

    /// Immutable canonical product role at this exact Context snapshot.
    /// Ordinary Persona Agents return None; extras cannot declare this role.
    async fn context_role(
        &self, owner_id:&str, companion_id:&str, session_id:&str,
        expected_snapshot:&nomifun_agent_contracts::ResolvedSnapshotRef,
    )->Result<Option<CompanionRole>,AppError>;

    async fn create(
        &self,
        owner_id: &str,
        request: CreateConversationRequest,
        skill_names: Vec<String>,
        context: CompanionSessionCreateContext,
    ) -> Result<ConversationResponse, AppError>;

    async fn delete(&self, owner_id: &str, session_id: &str) -> Result<(), AppError>;

    /// Enumerate immutable canonical affiliation, including associated task
    /// Sessions that do not appear in the top-level thread navigation.
    async fn list_companion_sessions(
        &self, owner_id: &str, companion_id: &str,
    ) -> Result<Vec<ConversationResponse>, AppError>;

    /// Required lifecycle fence: close admission and settle/delete every
    /// canonically affiliated Session and directed execution before the domain
    /// removes the shared Companion workspace or identity.
    async fn delete_companion_sessions(
        &self, owner_id: &str, companion_id: &str, workspace: Option<&str>,
    ) -> Result<CompanionDeletionGuard, AppError>;

    /// Read query-relevant typed canonical inputs and sealed public results.
    /// Implementations validate Companion affiliation for every returned source;
    /// projection transcripts, thinking and raw tool payloads never substitute.
    async fn recall_companion_history(
        &self, owner_id: &str, companion_id: &str, query: CompanionHistoryRecallQuery,
    ) -> Result<CompanionHistoryRecallPage, AppError>;

    async fn message_local_day_index(
        &self,
        owner_id: &str,
        session_id: &str,
    ) -> Result<Vec<MessageDayBucket>, AppError>;
}

/// All late-bound Session collaborators needed by Companion.
pub struct CompanionHostPorts {
    pub sessions: Arc<dyn CompanionSessionPort>,
    pub archive: Arc<dyn ArchiveConversationPort>,
    pub transcript: Arc<dyn TranscriptSource>,
}

/// Transitional host implementation for the archive-specific Session contract.
///
/// All Conversation DTO/metadata translation is kept here, at the edge. The
/// archiver and its typed host contract only see normalized dialogue messages.

/// Best-effort plain-text extraction from a message's `content` JSON. Structured
/// tool payloads intentionally do not cross the archive Session boundary.

/// Build Companion's transitional Conversation-backed ports.
///
/// All adapters delegate to the same owner/repository. They retain no Session
/// facts, runtime cache, fallback, or alternate identity.

/// Build Companion's host ports from an already-composed Session owner.
///
/// Production Nomi-core assembly uses this entry point so the domain does not
/// construct another Conversation-backed Session adapter. The archive and
/// transcript readers remain small, read-only adapters over the same service.

/// Compose Companion from host-owned typed Session collaborators.
///
/// This is the migration target for the application host: the caller supplies
/// the normal thread-management port, the archive-specific port, and the
/// transcript source independently. No Conversation implementation is needed
/// by this composition function.
pub fn companion_ports_from_typed_host(
    owner_id: Arc<str>,
    sessions: Arc<dyn CompanionSessionPort>,
    archive_sessions: Arc<dyn CompanionArchiveSessionPort>,
    transcript: Arc<dyn TranscriptSource>,
) -> CompanionHostPorts {
    CompanionHostPorts {
        sessions,
        archive: Arc::new(SessionArchivePort::new(owner_id, archive_sessions)),
        transcript,
    }
}
