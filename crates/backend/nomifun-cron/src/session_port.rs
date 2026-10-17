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
    /// Exact durable reservation that authorized this turn. New-conversation
    /// jobs intentionally do not store their per-run Session on `cron_jobs`,
    /// so the reservation is the canonical job/Session relation.
    pub cron_job_run_id: String,
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

/// Exact Agent authority requested for a newly-created Cron Session.
///
/// Preset-backed schedules carry their already-frozen Snapshot. The explicit
/// model-only variant asks the host Agent application service to materialize a
/// minimal immutable binding from the job's provider/model selection. An
/// untyped `None` would let an invalid legacy configuration reach Session
/// creation and fail only after scheduler admission.
#[derive(Debug, Clone, PartialEq)]
pub enum CronSessionAgentBinding {
    Frozen(AgentResolvedSnapshot),
    ModelOnly,
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

/// Exact operation receipt owned by the Cron boundary, including nonterminal
/// suspension. Consumers never need a Conversation row or runtime handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronTurnDelivery {
    pub message_id: String,
    pub replayed: bool,
    pub completed: bool,
    pub turn_outcome: Option<nomifun_agent_contracts::AgentTurnOutcome>,
    pub user_input_request: Option<nomifun_agent_contracts::AgentUserInputRequest>,
    pub result_ok: Option<bool>,
    pub result_text: Option<String>,
    pub result_error: Option<String>,
    pub result_error_code: Option<String>,
    pub result_error_retryable: Option<bool>,
}

impl CronTurnDelivery {
    /// A closed execution is not necessarily a delivered scheduled result.
    /// Waiting and paused facts remain nonterminal; missing facts fail closed.
    /// A closed question requires human input and never authorizes replay.
    pub(crate) fn disposition(&self) -> Option<CronTurnDisposition> {
        use nomifun_agent_contracts::AgentTurnOutcome;
        let outcome = self.turn_outcome.as_ref()?;
        if matches!(outcome, AgentTurnOutcome::Waiting { .. }) {
            return self.completed.then(|| CronTurnDisposition::Failed(
                "A completed scheduled Turn cannot retain a waiting disposition".into()));
        }
        if !self.completed { return None; }
        let failure = || self.result_error.clone().or_else(|| self.result_text.clone())
            .unwrap_or_else(|| "The scheduled Agent turn ended without a successful public delivery".into());
        match outcome {
            AgentTurnOutcome::InputRequired { request_id } => Some(match &self.user_input_request {
                Some(request) if request.request_id.as_ref() == request_id && request.validate().is_ok() =>
                    CronTurnDisposition::InputRequired,
                _ => CronTurnDisposition::Failed("The scheduled Agent input handoff has no matching canonical question".into()),
            }),
            AgentTurnOutcome::Answered { .. } if self.user_input_request.is_none() =>
                self.result_ok.map(|ok| if ok { CronTurnDisposition::Delivered } else { CronTurnDisposition::Failed(failure()) }),
            AgentTurnOutcome::Unavailable => Some(CronTurnDisposition::Failed(
                "The scheduled Agent turn closed without a public answer or input handoff; its effects will not be replayed".into())),
            AgentTurnOutcome::Paused { .. } => None,
            AgentTurnOutcome::Cancelled { .. } => Some(CronTurnDisposition::Cancelled),
            _ => Some(CronTurnDisposition::Failed(failure())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CronTurnDisposition { Delivered, InputRequired, Cancelled, Failed(String) }

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
    Suspended(CronTurnDelivery),
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
        agent_binding: CronSessionAgentBinding,
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

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::{Mutex, atomic::{AtomicUsize, Ordering}};

    /// Receipt consumers must never turn observation into execution.
    pub(crate) struct ReceiptOnlySessions {
        pub receipt: Mutex<CronTurnReceiptState>,
        pub reads: AtomicUsize,
        pub transient_failures: AtomicUsize,
    }
    impl ReceiptOnlySessions {
        pub fn new(receipt: CronTurnReceiptState) -> Self {
            Self { receipt: Mutex::new(receipt), reads: AtomicUsize::new(0), transient_failures: AtomicUsize::new(0) }
        }
    }
    #[async_trait]
    impl CronSessionPort for ReceiptOnlySessions {
        async fn read_turn_receipt(&self, _: &CronTurnReceiptQuery) -> Result<CronTurnReceiptState, AppError> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if self.transient_failures.fetch_update(Ordering::SeqCst, Ordering::SeqCst,
                |remaining| (remaining > 0).then(|| remaining - 1)).is_ok() {
                return Err(AppError::Internal("transient receipt query interruption".into()));
            }
            Ok(self.receipt.lock().unwrap().clone())
        }
        async fn reconcile_turn_receipt(&self, _: &CronTurnReconciliationRequest) -> Result<CronTurnReconciliation, AppError> {
            Ok(CronTurnReconciliation::LiveExactOwnerWait)
        }
        async fn get_session(&self, _: &CronSessionLookup) -> Result<CronSessionProjection, AppError> {
            panic!("receipt observation cannot prepare a Session")
        }
        async fn list_conversation_responses_for_cron(&self, _: &CronScheduledSessionLookup) -> Result<Vec<ConversationResponse>, AppError> {
            panic!("receipt observation cannot discover another Session")
        }
        async fn bind_cron_relation(&self, _: &CronSessionCronBindingRequest) -> Result<(), AppError> {
            panic!("receipt observation cannot bind another Session")
        }
        async fn create_idempotent(&self, _: &str, _: CreateConversationRequest, _: CronSessionAgentBinding, _: &str) -> Result<CronSessionHandle, AppError> {
            panic!("receipt observation cannot create a Session")
        }
        async fn prepare_runtime_and_send(&self, _: CronRuntimePreparationRequest) -> Result<CronPreparedTurnDelivery, AppError> {
            panic!("receipt observation cannot prepare or send another Turn")
        }
        async fn delivery_result(&self, _: &CronTurnDeliveryQuery) -> Result<Option<CronTurnDelivery>, AppError> {
            panic!("receipt observation only reads its exact durable receipt")
        }
    }
}
