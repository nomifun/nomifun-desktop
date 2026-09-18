//! Optional read-only host history input. No store handles or effect authority
//! cross this boundary; the runtime interprets its persisted event codec.
use crate::{AgentEngineError, AgentEngineEvent};
use nomifun_chat_model_broker::{ChatCausality, ChatMessage};

pub struct AgentRecordedTurn {
    pub operation_id: String,
    pub receipt_status: String,
    pub requirement: ChatMessage,
    /// Empty means no compatible engine journal, not proof of no execution.
    pub events: Vec<AgentEngineEvent>,
}

pub struct AgentHistoryPage {
    pub turn: Option<AgentRecordedTurn>,
    pub has_older: bool,
}

#[async_trait::async_trait]
pub trait AgentHistoryPort: Send + Sync + std::fmt::Debug {
    /// Latest historical turn before the current accepted root, or before an
    /// exact older receipt cursor. Hosts must enforce owner/Session scope,
    /// fixed root cutoff, the owner's explicit clear-context floor, contiguous
    /// eligible rows and bounded reads. No tool replay or bypass of a reset.
    async fn read_previous(
        &self,
        causality: &ChatCausality,
        before_operation: Option<&str>,
    ) -> Result<AgentHistoryPage, AgentEngineError>;
}
