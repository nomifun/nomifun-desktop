use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{ActionId, CapabilityId, OperationId};
use nomifun_chat_model_broker::{ChatFinishReason, ChatToolCall, ChatUsage, ToolCallId};

use crate::engine::EngineBinding;
use crate::error::CodingEngineError;
use crate::tool::CodingToolResult;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum CodingEngineEvent {
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
        modules: Vec<crate::CodingRuntimeModule>,
        reason: crate::CodingRuntimeActivationReason,
    },
    ModelStepStarted {
        step: u16,
        operation_id: OperationId,
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
        retained_context: Option<Vec<crate::CodingCompactedItem>>,
    },
    CompactionUsage {
        usage: ChatUsage,
    },
    WorkStatus {
        status: crate::CodingWorkStatus,
    },
    PlanUpdated {
        plan: crate::CodingPlan,
    },
    InstructionsUpdated {
        context: String,
    },
    /// Write-ahead recovery obligations and explicit clearing, retained even
    /// when model history is compacted or chat messages are deleted.
    PatchRecoveryUpdated {
        state: crate::CodingPatchRecoveryState,
    },
    SteeringInputs {
        inputs: Vec<crate::CodingSteeringInput>,
    },
    TurnInputScope {
        wire_turn_id: String,
    },
    SteeringDeferred {
        inputs: Vec<crate::CodingSteeringInput>,
        reason: String,
    },
    /// Historical journal compatibility only. New turns never emit this event
    /// or restore authority from it; the platform supplies the frozen enabled
    /// set from the saved Snapshot.
    CapabilitiesActivated {
        previous_generation: u64,
        generation: u64,
        capability_id: CapabilityId,
        activated_bundle: Vec<CapabilityId>,
    },
    CompletionReview {
        status: crate::CodingWorkStatus,
    },
    CompletionObservation {
        observation: crate::CodingCompletionObservation,
    },
    CompletionReported {
        report: crate::CodingCompletionReport,
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
        result: CodingToolResult,
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
    TurnFailed {
        model_steps: u16,
        message: String,
    },
}

#[async_trait]
pub trait CodingEventSink: Send + Sync {
    async fn emit(&self, event: CodingEngineEvent) -> Result<(), CodingEngineError>;

    /// Write-ahead admission, not an execution result. Hosts with concurrent
    /// input must serialize this with their inbox: false means no admission
    /// was recorded and the call must be deferred without invoking the tool.
    /// The default is for sinks without a concurrent input owner.
    async fn admit_tool(&self, event: CodingEngineEvent) -> Result<bool, CodingEngineError> {
        if !matches!(event, CodingEngineEvent::ToolStarted { .. }) {
            return Err(CodingEngineError::InvalidContract(
                "tool admission requires ToolStarted".into(),
            ));
        }
        self.emit(event).await?;
        Ok(true)
    }
}

#[derive(Clone, Default)]
pub struct NoopCodingEventSink;

#[async_trait]
impl CodingEventSink for NoopCodingEventSink {
    async fn emit(&self, _event: CodingEngineEvent) -> Result<(), CodingEngineError> {
        Ok(())
    }
}

pub(crate) type SharedCodingEventSink = Arc<dyn CodingEventSink>;
