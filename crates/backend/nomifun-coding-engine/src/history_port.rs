//! Optional read-only host history input. No store handles or effect authority
//! cross this boundary; Coding interprets its own persisted event codec.
use crate::{CodingEngineError, CodingEngineEvent};
use nomifun_chat_model_broker::{ChatCausality, ChatMessage};

pub struct CodingRecordedTurn {
    pub operation_id: String,
    pub receipt_status: String,
    pub requirement: ChatMessage,
    /// Empty means no compatible engine journal, not proof of no execution.
    pub events: Vec<CodingEngineEvent>,
}

pub struct CodingHistoryPage {
    pub turn: Option<CodingRecordedTurn>,
    pub has_older: bool,
}

#[async_trait::async_trait]
pub trait CodingHistoryPort: Send + Sync + std::fmt::Debug {
    /// Latest historical turn before the current accepted root, or before an
    /// exact older receipt cursor. Hosts must enforce owner/Session scope,
    /// fixed root cutoff, the owner's explicit clear-context floor, contiguous
    /// eligible rows and bounded reads. No tool replay or bypass of a reset.
    async fn read_previous(
        &self,
        causality: &ChatCausality,
        before_operation: Option<&str>,
    ) -> Result<CodingHistoryPage, CodingEngineError>;
}
