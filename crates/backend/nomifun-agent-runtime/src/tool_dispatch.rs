//! Batch-local invocation facts, not Kernel admission or owner-effect proof.
//! Never infer dispatch from model-visible output, including deferred errors.
use std::collections::BTreeSet;
use std::sync::Mutex;

use async_trait::async_trait;
use nomifun_chat_model_broker::ToolCallId;
use tokio_util::sync::CancellationToken;

use crate::{AgentEngineError, AgentToolInvocation, AgentToolInvoker, AgentToolResult};

pub(crate) struct ToolDispatchBatch<'a> {
    inner: &'a dyn AgentToolInvoker,
    planned: BTreeSet<ToolCallId>,
    attempted: Mutex<BTreeSet<ToolCallId>>,
}

impl<'a> ToolDispatchBatch<'a> {
    pub(crate) fn new(inner: &'a dyn AgentToolInvoker, call_ids: &[ToolCallId]) -> Self {
        Self {
            inner,
            planned: call_ids.iter().cloned().collect(),
            attempted: Mutex::new(BTreeSet::new()),
        }
    }

    pub(crate) fn attempted(&self, call_id: &ToolCallId) -> Result<bool, AgentEngineError> {
        self.attempted
            .lock()
            .map(|attempted| attempted.contains(call_id))
            .map_err(|_| invalid("tool invocation accounting lock poisoned"))
    }
}

#[async_trait]
impl AgentToolInvoker for ToolDispatchBatch<'_> {
    async fn invoke(
        &self,
        invocation: AgentToolInvocation,
        cancellation: CancellationToken,
    ) -> Result<AgentToolResult, AgentEngineError> {
        if self.planned.contains(&invocation.call.call_id) {
            // Mark before entering the port, not after success. Cancellation
            // or failure after this point cannot establish absence of effects.
            // Scope-discovery calls use reserved engine identities and must
            // not be mistaken for the model's proposed workspace calls.
            let mut attempted = self
                .attempted
                .lock()
                .map_err(|_| invalid("tool invocation accounting lock poisoned"))?;
            if !attempted.insert(invocation.call.call_id.clone()) {
                return Err(invalid("tool call already attempted in this batch"));
            }
        }
        // No lock guard crosses an await, including parallel read-only calls.
        self.inner.invoke(invocation, cancellation).await
    }
}

fn invalid(message: &str) -> AgentEngineError {
    AgentEngineError::InvalidContract(message.into())
}
