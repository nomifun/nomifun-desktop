use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{ActionId, CapabilityId, OperationId};
use nomifun_chat_model_broker::{ChatFinishReason, ChatModelErrorCode, ChatToolCall, ChatUsage, ToolCallId};

use crate::engine::EngineBinding;
use crate::error::AgentEngineError;
use crate::tool::AgentToolResult;

/// Host-bound proof of one immutable public prefix in the canonical part
/// chain. It contains references and integrity data, never a mirrored body.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPublicOutputSource {
    pub message_id: String,
    pub part_count: u64,
    pub content_digest: nomifun_agent_contracts::DigestHex,
    pub through_event_id: nomifun_agent_contracts::EventId,
    pub binding_version: u64,
}

/// Typed model or native owner failure. Diagnostic prose never selects a
/// classification or recreates missing execution facts.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentTurnFailure {
    Model {
        code: ChatModelErrorCode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diagnostic: Option<nomifun_agent_contracts::ModelFailureDiagnostic>,
    },
    TaskIncomplete { reason: nomifun_agent_contracts::AgentTaskIncompleteReason },
    Runtime { stage: nomifun_agent_contracts::AgentFailureStage },
    ModelInvocation {
        #[serde(rename = "invocation_kind")]
        kind: nomifun_agent_contracts::ModelInvocationFailureKind,
        diagnostic: nomifun_agent_contracts::ModelFailureDiagnostic,
    },
    ResourceBindingIncomplete { diagnostic: nomifun_agent_contracts::AgentResourceBindingFailure },
    SessionConfigurationChanged,
    WorkspacePathUnsupported { path: String },
    WorkspaceDirectoryUnavailable { path: String },
    HostPermissionDenied,
    ModelStreamEndedWithoutTerminal,
    InvalidModelEvent,
    /// A fatal native preflight failure, preserving owner dispatch and cleanup
    /// facts without turning OS authentication cancellation into user Stop.
    SystemExecution { diagnostic: nomifun_agent_contracts::SystemExecutionFailure },
}

/// An identity-valid native proposal whose completed arguments were rejected.
/// Diagnostics contain no argument values and never authorize an invocation.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRejectedToolProposal {
    pub call_id: ToolCallId,
    pub name: String,
    pub diagnostic: nomifun_chat_model_broker::ToolArgumentDiagnostic,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentModelResponseRejection {
    NonNativeMarkup,
    EmptyTaskTerminal,
    /// A current blocked account still requires its explicit user-input declaration.
    RequiredInputHandoff,
    /// Accepted children cannot be treated as integrated without their exact
    /// terminal source results being consumed by this parent Turn.
    RequiredChildResults,
    RequiredDelegation,
    ToolArguments { diagnostics: Vec<AgentRejectedToolProposal> },
    /// Existing current-generation facts did not record their precise reason.
    /// Reading them preserves that uncertainty instead of inspecting prose.
    Unclassified,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentEngineEvent {
    TurnStarted {
        binding: EngineBinding,
        turn_operation_id: OperationId,
    },
    /// The host verified an exact canonical human question reply and the same
    /// binding before carrying unresolved child references into this Turn.
    /// This event carries no input body, resource grant or execution authority.
    CollaborationInherited { progress: crate::AgentCollaborationProgress },
    ExecutionBudgetPrepared {
        /// Zero means provider-defined/undeclared, never a zero-token ceiling.
        context_window_tokens: u32,
        /// Internal context reservation; provider wire ceiling may be absent.
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
        /// Exact earlier rejected model step in this same Turn. This is a
        /// context source, not a replacement for the accepted-input authority.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        correction_source: Option<u16>,
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
    /// A rejected summary draft is not a tool invocation or completion proof.
    /// Do not persist its arguments or treat it as accepted task instructions.
    CompactionSummaryRejected {
        operation_id: OperationId,
        reason: String,
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
    /// Non-native markup, invalid completed native arguments, or an active
    /// task terminal without public text/calls was rejected before admission.
    /// This closes any
    /// proposed batch for replay; it is not a transport retry,
    /// an executed call/result, an output-limit claim or a success event.
    ModelResponseRejected {
        step: u16,
        discarded_tool_call_ids: Vec<ToolCallId>,
        continuation: bool,
        /// Advisory repair constraint, validated against the exposed surface.
        /// It never represents a parsed or executed text tool call.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_hint: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rejection: Option<AgentModelResponseRejection>,
        /// Exact safe engine-authored feedback, shared by live and replay.
        /// Existing current-generation facts may lack this field. Their
        /// feedback remains unclassified and grants no new recovery authority.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        feedback: Option<String>,
    },
    /// The completed response passed model formatting and native argument
    /// syntax checks. This clears a pending protocol correction only; it is
    /// not Schema validation, tool admission, an effect or task completion.
    ModelResponseValidated { step: u16 },
    /// The exact public prefix already emitted by this model step is now
    /// irrevocable. This is a publication boundary, never a tool result or
    /// task-completion claim. Raw deltas remain retractable until this fact.
    PublicOutputCommitted { step: u16, #[serde(default,skip_serializing_if="Option::is_none")] source: Option<AgentPublicOutputSource> },
    /// Only an explicitly voice-started Immediate-policy Turn can emit this.
    /// The exact unadmitted proposal is withdrawn after owned abort/join;
    /// no Turn cancellation, tool result, applied input or success is claimed.
    VoiceModelStepSuperseded {
        step: u16,
        model_operation_id: OperationId,
        steering_receipt_ids: Vec<String>,
        discarded_tool_call_ids: Vec<ToolCallId>,
        cleanup: nomifun_chat_model_broker::OwnedModelCleanupReceipt,
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
    /// A host settlement check kept the turn open: durable host facts showed
    /// the accepted task's checked result was still missing. The relayed
    /// feedback is protocol correction, not a new accepted input, a tool
    /// result or a success receipt.
    CompletionCheckRejected {
        step: u16,
        feedback: String,
    },
    CompletionObservation {
        observation: crate::AgentCompletionObservation,
    },
    CompletionReported {
        report: crate::AgentCompletionReport,
    },
    /// Read-only recognition of an already-recorded current-generation candidate.
    /// New execution never writes this event or treats it as accepted delivery.
    CompletionCandidateRecorded {
        report: crate::AgentCompletionReport,
    },
    /// Read-only recognition of a superseded current-generation review response.
    /// New execution never creates this retired review phase.
    DeliveryReviewSuperseded {
        step: u16,
        discarded_tool_call_ids: Vec<ToolCallId>,
        continuation: bool,
    },
    /// Engine publication of the accepted report after the terminal input
    /// fence. This is not another provider/model stream event.
    CompletionDelivered {
        step: u16,
        text: String,
    },
    OutputTextDelta {
        step: u16,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<nomifun_agent_contracts::ChatOutputTextSource>,
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
        /// Absent on existing facts and ordinary control/owner observations.
        /// Never infer this decision from result prose or missing admission.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        runtime_hold: Option<crate::RuntimeToolHoldReason>,
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
    /// Explicit public-reply intent after the terminal input/host fences.
    /// This never proves physical movement or an asynchronous task completed.
    SilentCompletion {
        step: u16,
        tool_call_ids: Vec<ToolCallId>,
    },
    TurnCompleted {
        model_steps: u16,
        finish_reason: ChatFinishReason,
    },
    UserInputRequested {
        step: u16,
        request: nomifun_agent_contracts::AgentUserInputRequest,
    },
    EndIntentReview {
        status: crate::AgentWorkStatus,
    },
    TurnWaiting {
        model_steps: u16,
        request_id: Option<String>,
        reason: nomifun_agent_contracts::AgentWaitReason,
    },
    TurnWaitResolved { model_steps: u16 },
    /// The model is quiescent while this exact Turn awaits owned children.
    /// No resource cleanup, user-input handoff or terminal is implied.
    AwaitingChildren { model_steps: u16, request: crate::AgentChildrenWait },
    ChildrenWaitResolved { model_steps: u16, cursor: String },
    OwnedProcessObserved { model_steps: u16, observation: crate::AgentOwnedProcessObservation },
    TurnInputRequired {
        model_steps: u16,
        request: nomifun_agent_contracts::AgentUserInputRequest,
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure: Option<AgentTurnFailure>,
    },
}

impl AgentEngineEvent {
    /// Display-only reasoning body ownership. Narration is independent; `None`
    /// preserves the body, `Some(None)` closes it, and `Some(Some(step))` owns it.
    /// Live publication and canonical history use the same typed transitions.
    pub fn reasoning_display_transition(&self) -> Option<Option<u16>> {
        let kind = match self {
            Self::ReasoningDelta { .. } => "reasoning_delta",
            Self::OutputTextDelta { .. } => "output_text_delta",
            Self::ModelStepStarted { .. } => "model_step_started",
            Self::ExecutionResumed { .. } => "execution_resumed",
            Self::CompletionDelivered { .. } => "completion_delivered",
            Self::ToolCallDelta { .. } => "tool_call_delta",
            Self::ToolCallCompleted { .. } => "tool_call_completed",
            Self::ToolStarted { .. } => "tool_started",
            Self::ModelOutputTruncated { .. } => "model_output_truncated",
            Self::ModelResponseRejected { .. } => "model_response_rejected",
            Self::DeliveryReviewSuperseded { .. } => "delivery_review_superseded",
            Self::VoiceModelStepSuperseded { .. } => "voice_model_step_superseded",
            Self::SilentCompletion { .. } => "silent_completion",
            Self::TurnCompleted { .. } => "turn_completed",
            Self::UserInputRequested { .. } => "user_input_requested",
            Self::TurnWaiting { .. } => "turn_waiting",
            Self::TurnInputRequired { .. } => "turn_input_required",
            Self::TurnCancelled { .. } => "turn_cancelled",
            Self::TurnPaused { .. } => "turn_paused",
            Self::TurnFailed { .. } => "turn_failed",
            _ => return None,
        };
        use nomifun_agent_contracts::{reasoning_display_event, ReasoningDisplayEvent};
        match reasoning_display_event(kind) {
            ReasoningDisplayEvent::Reasoning => match self { Self::ReasoningDelta { step, .. } => Some(Some(*step)), _ => unreachable!() },
            ReasoningDisplayEvent::Boundary | ReasoningDisplayEvent::Invocation => Some(None),
            ReasoningDisplayEvent::Narration | ReasoningDisplayEvent::Other => None,
        }
    }
}

#[cfg(test)]
mod reasoning_display_tests {
    use super::*;

    #[test]
    fn reasoning_display_ownership_uses_the_shared_content_classifier() {
        assert_eq!(AgentEngineEvent::ReasoningDelta {step:1,text:"Call".into()}.reasoning_display_transition(),Some(Some(1)));
        for text in ["", "好的", "Public body"] {
            assert_eq!(AgentEngineEvent::OutputTextDelta {step:1,text:text.into(),source:None}.reasoning_display_transition(),None);
        }
        assert_eq!(AgentEngineEvent::ToolCompleted {step:1,runtime_hold:None,
            result:AgentToolResult::text("old".into(),"late",false)}.reasoning_display_transition(),None);
        assert_eq!(AgentEngineEvent::ToolCallDelta {step:1,call_id:"call".into(),name:"read_file".into(),arguments_delta:String::new()}
            .reasoning_display_transition(),Some(None));
    }
}

/// Outcome of the host's atomic write-ahead tool admission. A refusal records
/// no ToolStarted and grants no owner invocation, effect retry or extra budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentToolAdmission {
    Admitted,
    InputSuperseded,
    JournalBudget,
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
    /// input must serialize this with their inbox and result-storage capacity.
    /// A refusal means no admission was recorded and the owner cannot run.
    /// The default is for sinks without a concurrent input owner.
    async fn admit_tool(&self, event: AgentEngineEvent) -> Result<AgentToolAdmission, AgentEngineError> {
        if !matches!(event, AgentEngineEvent::ToolStarted { .. }) {
            return Err(AgentEngineError::InvalidContract(
                "tool admission requires ToolStarted".into(),
            ));
        }
        self.emit(event).await?;
        Ok(AgentToolAdmission::Admitted)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_hold_is_optional_for_existing_facts_and_preserves_its_typed_reason() {
        let old = serde_json::json!({"event":"tool_completed","step":1,
            "result":{"call_id":"call","output":[{"type":"text","text":"original feedback"}],"is_error":true}});
        let mut parsed: AgentEngineEvent = serde_json::from_value(old).unwrap();
        assert!(matches!(&parsed, AgentEngineEvent::ToolCompleted { runtime_hold: None, .. }));
        assert!(serde_json::to_value(&parsed).unwrap().get("runtime_hold").is_none());
        if let AgentEngineEvent::ToolCompleted { runtime_hold, .. } = &mut parsed {
            *runtime_hold = Some(crate::RuntimeToolHoldReason::ToolNotExposed);
        }
        let encoded = serde_json::to_value(&parsed).unwrap();
        assert_eq!(encoded["runtime_hold"], "tool_not_exposed");
        assert_eq!(serde_json::from_value::<AgentEngineEvent>(encoded).unwrap(), parsed);
    }

    #[test]
    fn failed_turn_keeps_optional_typed_cause_without_duplicating_its_diagnostic() {
        let generic: AgentEngineEvent = serde_json::from_value(serde_json::json!({
            "event":"turn_failed", "model_steps":1, "message":"existing completion guard",
        })).unwrap();
        assert!(matches!(&generic, AgentEngineEvent::TurnFailed { failure: None, .. }));
        assert!(serde_json::to_value(generic).unwrap().get("failure").is_none());
        let typed = AgentEngineEvent::TurnFailed { model_steps: 1, message: "original diagnostic".into(),
            failure: Some(AgentTurnFailure::Model { code: ChatModelErrorCode::AuthenticationFailed, diagnostic: None }),
        };
        let value = serde_json::to_value(&typed).unwrap();
        assert_eq!(value["failure"], serde_json::json!({"kind":"model","code":"AUTHENTICATION_FAILED"}));
        assert_eq!(value["message"], "original diagnostic");
        assert_eq!(serde_json::from_value::<AgentEngineEvent>(value).unwrap(), typed);
    }

    #[test]
    fn failed_turn_preserves_approved_diagnostic_fields_in_the_native_fact() {
        let mut diagnostic = nomifun_agent_contracts::ModelFailureDiagnostic::new(
            nomifun_agent_contracts::ModelFailureReason::TlsFailure);
        diagnostic.transport_detail = Some("TLS certificate verification failed".into());
        diagnostic.endpoint = Some("https://api.example.test/v1/chat/completions".into());
        diagnostic.provider_id = Some("0190f5fe-7c00-7a00-8000-000000000072".into());
        diagnostic.model_name = Some("actual-request-model".into());
        let event = AgentEngineEvent::TurnFailed { model_steps:1, message:"Safe fixed transport failure".into(),
            failure:Some(AgentTurnFailure::Model { code:ChatModelErrorCode::ProviderUnavailable, diagnostic:Some(diagnostic.clone()) }) };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["failure"]["diagnostic"], serde_json::to_value(&diagnostic).unwrap());
        assert_eq!(serde_json::from_value::<AgentEngineEvent>(value).unwrap(), event);
    }
}
