//! Typed Session boundary used by Cron execution.

use async_trait::async_trait;
use nomifun_agent_contracts::AgentSessionId;

use nomifun_api_types::{AgentResolvedSnapshot, ConversationResponse, CreateConversationRequest};

use nomifun_common::{AgentType, AppError, ProviderWithModel};

/// One Cron turn handed to the canonical Session owner.
///
/// Cron supplies only the user-visible message and the server-owned per-run
/// overlay (for example the Cron id and selected skills). The Session port
/// resolves the authoritative agent type, model, delegation policy, workspace
/// identity and conversation creation timestamp from its own projection before
/// it constructs any runtime preparation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnMessage {
    pub content: String,
    pub files: Vec<String>,
    pub inject_skills: Vec<String>,
    pub hidden: bool,
    pub origin: Option<String>,
    pub channel_platform: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnRuntimeOverlay {
    /// Cron-owned annotation used for artifacts, diagnostics, and runtime
    /// attribution. It is not a Session identity or runtime selector.
    pub cron_job_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnRuntimePreparation {
    pub overlay: CronTurnRuntimeOverlay,
    pub clear_context: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnRequest {
    pub message: CronTurnMessage,
    pub runtime: CronTurnRuntimePreparation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronRuntimePreparationRequest {
    pub owner_id: String,
    pub agent_session_id: AgentSessionId,
    pub idempotency_key: String,
    pub turn: CronTurnRequest,
}

/// Canonical handle exposed to Cron execution.  The scheduler does not need
/// the legacy Conversation DTO or its mutable `extra` map; it only needs the
/// stable AgentSession identity and the host-resolved workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSessionHandle {
    pub agent_session_id: AgentSessionId,
    pub workspace: String,
}

/// Canonical Session projection used by Cron scheduling and execution.
///
/// Every field is an explicit capability input. Cron never receives or scans a
/// mutable Session metadata bag in production.
#[derive(Debug, Clone, PartialEq)]
pub struct CronSessionProjection {
    pub agent_session_id: AgentSessionId,
    pub owner_id: String,
    pub name: String,
    pub agent_type: AgentType,
    pub model: Option<ProviderWithModel>,
    pub workspace: String,
    pub cron_job_id: Option<String>,
    pub temp_workspace_id: Option<String>,
    pub skills: Vec<String>,
    pub agent_name: Option<String>,
    pub cli_path: Option<String>,
    pub custom_agent_id: Option<String>,
    pub preset_id: Option<String>,
    pub preset_revision: Option<i64>,
    pub agent_snapshot: Option<AgentResolvedSnapshot>,
}

pub type CronScheduledSession = CronSessionProjection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronScheduledSessionLookup {
    pub owner_id: String,
    pub cron_job_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSessionLookup {
    pub owner_id: String,
    pub agent_session_id: AgentSessionId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSessionCronBindingRequest {
    pub owner_id: String,
    pub agent_session_id: AgentSessionId,
    pub cron_job_id: String,
}

/// Stable terminal receipt owned by the Cron boundary.  The consumer never
/// needs the Conversation repository row or runtime handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnDelivery {
    pub message_id: String,
    pub replayed: bool,
    pub completed: bool,
    pub result_ok: Option<bool>,
    pub result_text: Option<String>,
    pub result_error: Option<String>,
    pub result_error_code: Option<String>,
    pub result_error_retryable: Option<bool>,
}

/// Result of the host-owned preparation gate.
///
/// `workspace` is the canonical workspace resolved from the latest Session
/// snapshot inside the same preparation lease that admitted the turn. Cron
/// may use it for post-turn Cron-owned artifacts, but cannot choose or
/// override it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronPreparedTurnDelivery {
    pub delivery: CronTurnDelivery,
    pub workspace: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CronTurnReceiptState {
    Missing,
    Accepted { message_id: String },
    Completed(CronTurnDelivery),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnReceiptQuery {
    pub owner_id: String,
    pub agent_session_id: AgentSessionId,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnReconciliationRequest {
    pub owner_id: String,
    pub agent_session_id: AgentSessionId,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnDeliveryQuery {
    pub owner_id: String,
    pub agent_session_id: AgentSessionId,
    pub idempotency_key: String,
    pub message: CronTurnMessage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CronTurnReconciliation {
    LiveExactOwnerWait,
    ReconciledOrTerminalReRead,
    ExternalProofRequiredFailClosed,
    StaleConflict,
}

/// Exact Session operations needed by the Cron domain.
#[async_trait]
pub trait CronSessionPort: Send + Sync {
    async fn get_session(
        &self,
        query: &CronSessionLookup,
    ) -> Result<CronSessionProjection, AppError>;

    /// Legacy HTTP projection retained for the existing Cron conversations
    /// endpoint. Core scheduling and execution use `get_session` with the
    /// relation selected from the authoritative Cron row.
    async fn list_conversation_responses_for_cron(
        &self,
        query: &CronScheduledSessionLookup,
    ) -> Result<Vec<ConversationResponse>, AppError>;

    async fn bind_cron_relation(
        &self,
        request: &CronSessionCronBindingRequest,
    ) -> Result<(), AppError>;

    async fn read_turn_receipt(
        &self,
        query: &CronTurnReceiptQuery,
    ) -> Result<CronTurnReceiptState, AppError>;

    async fn reconcile_turn_receipt(
        &self,
        request: &CronTurnReconciliationRequest,
    ) -> Result<CronTurnReconciliation, AppError>;

    async fn create_idempotent(
        &self,
        user_id: &str,
        request: CreateConversationRequest,
        snapshot: Option<AgentResolvedSnapshot>,
        creation_key: &str,
    ) -> Result<CronSessionHandle, AppError>;

    async fn prepare_runtime_and_send(
        &self,
        request: CronRuntimePreparationRequest,
    ) -> Result<CronPreparedTurnDelivery, AppError>;

    async fn delivery_result(
        &self,
        query: &CronTurnDeliveryQuery,
    ) -> Result<Option<CronTurnDelivery>, AppError>;

    /// Append a durable owner-visible scheduler notice to the canonical
    /// AgentSession projection. Implementations must not write legacy message
    /// tables. Test adapters may ignore notices.
    async fn append_notice(
        &self,
        _owner_id: &str,
        _agent_session_id: &AgentSessionId,
        _content: &str,
        _notice_kind: &str,
    ) -> Result<(), AppError> {
        Ok(())
    }
}
