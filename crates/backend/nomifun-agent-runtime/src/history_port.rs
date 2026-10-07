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
    /// Optional direct reader for an explicit current-user Turn reference.
    /// None means this host does not support direct addressing; callers may
    /// use the existing bounded cursor walk. Some(page) is an authoritative
    /// lookup outcome (including an unavailable/out-of-scope target), never a
    /// grant to cross Session, accepted-root, context-reset or Agent boundaries.
    async fn read_exact(
        &self,
        _causality: &ChatCausality,
        _operation: &str,
    ) -> Result<Option<AgentHistoryPage>, AgentEngineError> {
        Ok(None)
    }

    /// Optional owner proof for a Snapshot differing ONLY by its Chat route.
    /// False by default; never an execution/recovery or resource-scope grant.
    async fn model_snapshot_compatible(
        &self,
        _causality: &ChatCausality,
        _source: &nomifun_agent_contracts::ResolvedSnapshotRef,
    ) -> Result<bool, AgentEngineError> {
        Ok(false)
    }

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
