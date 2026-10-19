//! Canonical Agent Turn delivery projections shared by product adapters.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdempotentMessageDelivery {
    pub message_id: String,
    pub replayed: bool,
    pub completed: bool,
    pub turn_outcome: Option<nomifun_agent_contracts::AgentTurnOutcome>,
    pub result_ok: Option<bool>,
    pub result_text: Option<String>,
    pub result_error: Option<String>,
    pub result_error_code: Option<String>,
    pub result_error_retryable: Option<bool>,
    pub user_input_request: Option<nomifun_agent_contracts::AgentUserInputRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnavailableInputCause {
    CancelledByOwner,
    ContextCleared,
    PreparationBlocked { reason: String },
}

impl UnavailableInputCause {
    pub fn code(&self) -> &str {
        match self {
            Self::CancelledByOwner => "CANCELLED_BY_OWNER",
            Self::ContextCleared => "CONTEXT_CLEARED",
            Self::PreparationBlocked { reason } => reason,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicTurnDeliveryState {
    Missing,
    Accepted { message_id: String },
    /// The accepted input exists, but no Turn was admitted for it. This is a
    /// read projection of its original canonical input lifecycle, not a Turn
    /// completion or a second execution receipt.
    InputUnavailable { message_id: String, cause: UnavailableInputCause, event_seq: u64 },
    Completed(IdempotentMessageDelivery),
}

pub trait BackgroundTaskRegistrar: Send + Sync {
    /// Transfer process-owned cleanup to shutdown without aborting native work.
    fn shutdown_signal(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>;

    fn spawn(
        &self,
        task: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>,
    ) -> bool;
}
