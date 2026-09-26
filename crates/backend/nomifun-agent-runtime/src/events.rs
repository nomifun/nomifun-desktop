use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{ActionId, CapabilityId, OperationId};
use nomifun_chat_model_broker::{ChatFinishReason, ChatToolCall, ChatUsage, ToolCallId};

use crate::engine::EngineBinding;
use crate::error::AgentEngineError;
use crate::tool::AgentToolResult;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentEngineEvent {
    TurnStarted {
        binding: EngineBinding,
        turn_operation_id: OperationId,
    },
    ExecutionBudgetPrepared {
        context_window_tokens: u32,
        max_output_tokens: u32,
        max_model_steps: u16,
    },
    ContextPrepared {
        dropped_history_messages: usize,
        warnings: Vec<String>,
    },
    /// Turn-local mechanisms activated by observed work. This records actual
    /// execution weight; it never changes the frozen Snapshot or ToolPlan.
    RuntimeModulesActivated {
        modules: Vec<crate::AgentRuntimeModule>,
        reason: crate::AgentRuntimeActivationReason,
    },
    ModelStepStarted {
        step: u16,
        operation_id: OperationId,
    },
    /// Metadata for the Store's atomic snapshot commit. The snapshot itself
    /// is latest-state data on the Turn, not another transcript copy.
    ExecutionCheckpointSaved {
        step: u16,
        revision: u64,
        digest: nomifun_agent_contracts::DigestHex,
    },
    ExecutionResumed {
        checkpoint_revision: u64,
        checkpoint_step: u16,
        model_steps: u16,
        execution_fence: u64,
        discarded_tool_call_ids: Vec<ToolCallId>,
    },
    /// The new window is already committed in this exact checkpoint. This
    /// event is observability, not an independent budget/authority grant.
    ExecutionSegmentRenewed {
        segment: u16,
        model_steps: u16,
        checkpoint_revision: u64,
        reason: crate::AgentSegmentReason,
    },
    ExecutionBudgetExhausted {
        model_steps: u16,
        segment: u16,
        checkpoint_revision: Option<u64>,
        reason: crate::AgentExecutionStopReason,
    },
    CompactionStarted {
        operation_id: OperationId,
        input_bytes: usize,
    },
    /// Typed prompt rejection before semantic output; only requests a bounded
    /// compaction. Does not assert that compaction or continuation succeeded.
    ContextLimitRecoveryStarted {
        rejected_step: u16,
    },
    /// The terminal was explicitly output-limited. Written before dropping
    /// the proposed batch; none of these calls reached tool admission.
    ModelOutputTruncated {
        step: u16,
        discarded_tool_call_ids: Vec<ToolCallId>,
        continuation: bool,
    },
    /// Non-native tool markup was rejected before any tool admission. This
    /// closes the proposed batch for replay; it is not a transport retry,
    /// an executed call/result, an output-limit claim or a success event.
    ModelResponseRejected {
        step: u16,
        discarded_tool_call_ids: Vec<ToolCallId>,
        continuation: bool,
    },
    ContextCompacted {
        input_bytes_before: usize,
        input_bytes_after: usize,
        summary: String,
        /// Ordered IDs of up to three contiguous complete batches ending at
        /// the latest exchange, at most 64 IDs, and their retained suffix
        /// (including intervening/later non-tool responses and accepted input),
        /// never new executions or effect-completion evidence.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        retained_tool_call_ids: Vec<ToolCallId>,
        /// Portable replacement after the summary. None is the legacy
        /// reference-only codec; accepted input references retain ownership.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retained_context: Option<Vec<crate::AgentCompactedItem>>,
    },
    CompactionUsage {
        usage: ChatUsage,
    },
    WorkStatus {
        status: crate::AgentWorkStatus,
    },
    PlanUpdated {
        plan: crate::AgentPlan,
    },
    InstructionsUpdated {
        context: String,
    },
    /// Write-ahead recovery obligations and explicit clearing, retained even
    /// when model history is compacted or chat messages are deleted.
    PatchRecoveryUpdated {
        state: crate::AgentPatchRecoveryState,
    },
    SteeringInputs {
        inputs: Vec<crate::AgentSteeringInput>,
    },
    TurnInputScope {
        wire_turn_id: String,
    },
    SteeringDeferred {
        inputs: Vec<crate::AgentSteeringInput>,
        reason: String,
    },
    CompletionReview {
        status: crate::AgentWorkStatus,
    },
    CompletionObservation {
        observation: crate::AgentCompletionObservation,
    },
    CompletionReported {
        report: crate::AgentCompletionReport,
    },
    OutputTextDelta {
        step: u16,
        text: String,
    },
    ReasoningDelta {
        step: u16,
        text: String,
    },
    ToolCallDelta {
        step: u16,
        call_id: ToolCallId,
        name: String,
        arguments_delta: String,
    },
    ToolCallCompleted {
        step: u16,
        call: ChatToolCall,
    },
    ToolStarted {
        step: u16,
        call_id: ToolCallId,
        capability_id: CapabilityId,
        action_id: ActionId,
    },
    ToolCompleted {
        step: u16,
        result: AgentToolResult,
    },
    /// An owner-backed observation of an earlier invocation, never another
    /// tool admission or current completion evidence.
    ToolOutcomeReconciled {
        step: u16,
        result: AgentToolResult,
        source: crate::AgentReconciliationSource,
        evidence_event_id: Option<String>,
        owner_operation_id: Option<OperationId>,
    },
    OwnerOutcomeReconciled {
        call_id: Option<ToolCallId>,
        effect_id: Option<String>,
        outcome: String,
        evidence_event_id: String,
        source: crate::AgentReconciliationSource,
    },
    ExecutionTailReconciled {
        model_steps: u16,
        source_checkpoint_revision: u64,
        discarded_tool_call_ids: Vec<ToolCallId>,
        retained_tool_call_ids: Vec<ToolCallId>,
        discard_last_model_step: bool,
        retry_stall_guards: bool,
    },
    /// Derived model-context ordering after all results are recorded. This is
    /// not execution order, model delivery acknowledgement or cleanup proof.
    ToolResultsOrdered {
        step: u16,
        call_ids: Vec<ToolCallId>,
    },
    Usage {
        step: u16,
        usage: ChatUsage,
    },
    TurnCompleted {
        model_steps: u16,
        finish_reason: ChatFinishReason,
    },
    TurnCancelled {
        model_steps: u16,
    },
    TurnPaused {
        model_steps: u16,
        reason: String,
    },
    TurnFailed {
        model_steps: u16,
        message: String,
    },
}

#[async_trait]
pub trait AgentEventSink: Send + Sync {
    async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError>;
    fn supports_checkpoints(&self) -> bool { false }

    async fn execution_pressure(&self) -> Result<crate::AgentExecutionPressure, AgentEngineError> {
        Ok(crate::AgentExecutionPressure::default())
    }

    /// None explicitly means this host does not persist native checkpoints.
    /// It must never be treated as a durable acknowledgement or resume grant.
    async fn save_checkpoint(&self, _checkpoint: crate::AgentExecutionCheckpoint)
        -> Result<Option<crate::AgentCheckpointReceipt>, AgentEngineError> {
        Ok(None)
    }

    /// Write-ahead admission, not an execution result. Hosts with concurrent
    /// input must serialize this with their inbox: false means no admission
    /// was recorded and the call must be deferred without invoking the tool.
    /// The default is for sinks without a concurrent input owner.
    async fn admit_tool(&self, event: AgentEngineEvent) -> Result<bool, AgentEngineError> {
        if !matches!(event, AgentEngineEvent::ToolStarted { .. }) {
            return Err(AgentEngineError::InvalidContract(
                "tool admission requires ToolStarted".into(),
            ));
        }
        self.emit(event).await?;
        Ok(true)
    }
}

#[derive(Clone, Default)]
pub struct NoopAgentEventSink;

#[async_trait]
impl AgentEventSink for NoopAgentEventSink {
    async fn emit(&self, _event: AgentEngineEvent) -> Result<(), AgentEngineError> {
        Ok(())
    }
}

pub(crate) type SharedAgentEventSink = Arc<dyn AgentEventSink>;
