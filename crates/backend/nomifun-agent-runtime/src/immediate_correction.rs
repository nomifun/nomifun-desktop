//! Explicit host policy for a voice-started Turn. Ordinary requests have no
//! port and retain the original model stream and steering boundaries.
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::OperationId;
use nomifun_chat_model_broker::{ChatCausality, OwnedModelCleanupReceipt};

use crate::{AgentEngineError, AgentModelPort};

#[async_trait]
pub trait AgentImmediateCorrectionPort: Send + Sync + std::fmt::Debug {
    /// The same admitted Broker/route/plugin pipeline, with owned attempts.
    fn model_port(&self) -> Arc<dyn AgentModelPort>;

    /// Exact committed voice receipt IDs. This neither consumes an input nor
    /// cancels the Turn or any admitted tool/effect.
    async fn wait(&self, causality: &ChatCausality) -> Result<Vec<String>, AgentEngineError>;
    async fn pending(&self, causality: &ChatCausality) -> Result<Vec<String>, AgentEngineError>;
    async fn has_admitted_tools(&self, causality: &ChatCausality, step:u16) -> Result<bool, AgentEngineError>;

    /// Succeeds only after the actual producer exited and its task was joined.
    async fn quiesce_model(
        &self,
        operation: &OperationId,
        deadline: tokio::time::Instant,
    ) -> Result<OwnedModelCleanupReceipt, AgentEngineError>;

    /// Includes any bounded compaction/opening attempt owned by this same
    /// opted-in Turn. Hosts also use it before the original cleanup witness.
    async fn quiesce_all(
        &self,
        deadline: tokio::time::Instant,
    ) -> Result<Vec<OwnedModelCleanupReceipt>, AgentEngineError>;
}
