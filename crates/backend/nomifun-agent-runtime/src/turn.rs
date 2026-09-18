use std::collections::BTreeMap;
use std::sync::Arc;

use futures::StreamExt;
use nomifun_agent_contracts::{AgentSessionId, OperationId, PrincipalRef};
use nomifun_chat_model_broker::{
    ChatContentPart, ChatFinishReason, ChatMessage, ChatModelErrorCode, ChatModelEvent,
    ChatModelRequest, ChatRole, ChatToolCall, ChatToolChoice, ProviderRoundId, ToolCallId,
};
use tokio_util::sync::CancellationToken;

use crate::engine::EngineBinding;
use crate::context::{AgentContextAssembler, AgentContextBudget};
use crate::error::AgentEngineError;
use crate::events::{AgentEngineEvent, AgentEventSink};
use crate::model::AgentModelPort;
use crate::tool::{
    invocation_for, parse_completed_arguments, validate_tool_argument_size, AgentEffectClass,
    AgentToolInvoker, AgentToolPlan, AgentToolResult,
};

const DEFAULT_MAX_MODEL_STEPS: u16 = 32;
const MAX_CALLS_PER_STEP: usize = 64;

#[derive(Clone, Debug)]
pub struct AgentTurnRequest {
    pub model_request: ChatModelRequest,
    pub tool_plan: AgentToolPlan,
    pub principal: PrincipalRef,
    /// Platform-supplied compatibility generation for the frozen enabled set.
    /// The runtime loop never changes it or the admitted tool plan.
    pub active_set_generation: u64,
    pub max_model_steps: u16,
    pub model_budget: crate::AgentModelBudget,
    pub context_resources: Arc<BTreeMap<String, crate::AgentContextResource>>,
    /// Host-admitted for this turn; false unless both vision authority and the
    /// frozen primary model route support images (Skill/workspace readers).
    /// Broker revalidates on send.
    pub context_image_input: bool,
    pub input_port: Option<Arc<dyn crate::AgentInputPort>>,
    pub live_context_port: Option<Arc<dyn crate::AgentLiveContextPort>>,
    pub resource_port: Option<Arc<dyn nomifun_engine_core::EngineResourcePort>>,
    pub tool_discovery_port: Option<Arc<dyn crate::AgentToolDiscoveryPort>>,
    pub history_port: Option<Arc<dyn crate::AgentHistoryPort>>,
    /// Canonical latest closed task, not a checkpoint or live execution state.
    pub prior_task: Option<crate::AgentPriorTask>,
    /// Host-loaded permanent engine state, independent of the history window.
    pub patch_recovery: crate::AgentPatchRecoveryState,
}

impl AgentTurnRequest {
    pub fn new(
        model_request: ChatModelRequest,
        tool_plan: AgentToolPlan,
        principal: PrincipalRef,
        active_set_generation: u64,
    ) -> Self {
        Self {
            model_request,
            tool_plan,
            principal,
            active_set_generation,
            max_model_steps: DEFAULT_MAX_MODEL_STEPS,
            model_budget: crate::AgentModelBudget::default(),
            context_resources: Arc::default(),
            context_image_input: false,
            input_port: None,
            live_context_port: None,
            resource_port: None,
            tool_discovery_port: None,
            history_port: None,
            prior_task: None,
            patch_recovery: Default::default(),
        }
    }

    pub fn with_max_model_steps(mut self, max_model_steps: u16) -> Self {
        self.max_model_steps = max_model_steps;
        self
    }

    pub fn with_model_budget(mut self, budget: crate::AgentModelBudget) -> Self {
        self.model_budget = budget;
        self
    }

    pub fn with_input_port(mut self, port: Arc<dyn crate::AgentInputPort>) -> Self {
        self.input_port = Some(port);
        self
    }

    pub fn with_live_context_port(mut self, port: Arc<dyn crate::AgentLiveContextPort>) -> Self {
        self.live_context_port = Some(port);
        self
    }

    pub fn with_prior_task(mut self, prior_task: Option<crate::AgentPriorTask>) -> Self {
        self.prior_task = prior_task;
        self
    }

    pub fn with_patch_recovery(mut self, state: crate::AgentPatchRecoveryState) -> Self {
        self.patch_recovery = state;
        self
    }

    pub fn with_context_resources(mut self, resources: Arc<BTreeMap<String, crate::AgentContextResource>>) -> Self {
        self.context_resources = resources;
        self
    }

    pub fn with_resource_port(mut self, port: Arc<dyn nomifun_engine_core::EngineResourcePort>) -> Self {
        self.resource_port = Some(port);
        self
    }

    pub fn with_tool_discovery_port(
        mut self,
        port: Arc<dyn crate::AgentToolDiscoveryPort>,
    ) -> Self {
        self.tool_discovery_port = Some(port);
        self
    }

    pub fn with_context_image_input(mut self, allowed: bool) -> Self {
        self.context_image_input = allowed;
        self
    }

    pub fn with_history_port(mut self, port: Arc<dyn crate::AgentHistoryPort>) -> Self {
        self.history_port = Some(port);
        self
    }

    fn validate_for(
        &self,
        binding: &EngineBinding,
    ) -> Result<(), AgentEngineError> {
        if self.max_model_steps == 0 {
            return Err(AgentEngineError::InvalidContract(
                "max_model_steps must be greater than zero".to_owned(),
            ));
        }
        if self.model_request.causality.agent_session_id.as_ref().is_empty() {
            return Err(AgentEngineError::InvalidContract(
                "model request must identify an AgentSession".to_owned(),
            ));
        }
        if self.principal.principal_id.trim().is_empty()
            || self.principal.principal_kind.trim().is_empty()
        {
            return Err(AgentEngineError::InvalidContract(
                "Agent Runtime turn principal must be complete".to_owned(),
            ));
        }
        if &self.model_request.causality.agent_session_id != binding.agent_session_id() {
            return Err(AgentEngineError::TurnBindingMismatch {
                field: "agent_session_id",
            });
        }
        if &self.model_request.causality.resolved_snapshot_ref != binding.resolved_snapshot_ref() {
            return Err(AgentEngineError::TurnBindingMismatch {
                field: "resolved_snapshot_ref",
            });
        }
        if let Some(prior) = &self.prior_task {
            prior.validate_for(binding, self.model_request.causality.turn_operation_id.as_ref())?;
        }
        self.patch_recovery.validate()?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentTurnTerminal {
    Completed { finish_reason: ChatFinishReason },
    Cancelled,
    Failed { message: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentTurnResult {
    pub agent_session_id: AgentSessionId,
    pub turn_operation_id: OperationId,
    pub model_steps: u16,
    pub output_text: String,
    pub reasoning_text: String,
    pub tool_call_count: u32,
    pub provider_round_id: Option<ProviderRoundId>,
    pub terminal: AgentTurnTerminal,
}

pub(crate) async fn run_turn(
    binding: EngineBinding,
    model: Arc<dyn AgentModelPort>,
    tools: Arc<dyn AgentToolInvoker>,
    event_sink: Arc<dyn AgentEventSink>,
    mut request: AgentTurnRequest,
    context_budget: AgentContextBudget,
    cancellation: CancellationToken,
) -> Result<AgentTurnResult, AgentEngineError> {
    request.validate_for(&binding)?;
    request.model_budget = request.model_budget.for_request(request.model_request.input.max_output_tokens)?;
    let mut context_lifecycle = crate::context_lifecycle::ContextLifecycle::new(request.model_budget, context_budget)?;

    // Emit the root first so instruction reads have durable turn authority.
    event_sink.emit(AgentEngineEvent::TurnStarted {
        binding: binding.clone(),
        turn_operation_id: request.model_request.causality.turn_operation_id.clone(),
    }).await?;
    event_sink.emit(AgentEngineEvent::ExecutionBudgetPrepared {
        context_window_tokens: request.model_budget.context_window_tokens,
        max_output_tokens: request.model_budget.max_output_tokens,
        max_model_steps: request.max_model_steps,
    }).await?;
    let mut adaptive = crate::adaptive::AdaptiveExecution::default();
    let mut scoped_instructions = crate::workspace_context::ScopedInstructions::new(&request);
    let mut patch_recovery = crate::patch_recovery::PatchRecovery::restore(&request.patch_recovery)?;
    if request.prior_task.is_some() {
        adaptive
            .activate(
                [crate::AgentRuntimeModule::TaskContinuation],
                crate::AgentRuntimeActivationReason::HistoricalTaskCandidate,
                event_sink.as_ref(),
            )
            .await?;
    }
    if patch_recovery.pending() {
        adaptive
            .activate(
                crate::adaptive::LONG_HORIZON_MODULES
                    .into_iter()
                    .chain([crate::AgentRuntimeModule::PatchRecovery]),
                crate::AgentRuntimeActivationReason::PendingPatchRecovery,
                event_sink.as_ref(),
            )
            .await?;
    }
    let mut model_request = request.model_request;
    model_request
        .input
        .instructions
        .insert(0, crate::workflow::MINIMAL_EXECUTION_INSTRUCTIONS.into());
    model_request.input.max_output_tokens = Some(request.model_budget.max_output_tokens);
    model_request.input.instructions.push(request.model_budget.execution_context(request.max_model_steps));
    let requested_tool_choice = model_request.input.tool_choice.clone();
    if let Some(prior) = &request.prior_task {
        model_request.input.instructions.push(prior.context()?);
    }
    if !request.context_resources.is_empty() {
        model_request.input.instructions.push(crate::context_resources::index(&request.context_resources, request.context_image_input)?);
    }
    let agent_session_id = model_request.causality.agent_session_id.clone();
    let turn_operation_id = model_request.causality.turn_operation_id.clone();
    let tool_archive_scope = serde_json::json!([agent_session_id, turn_operation_id]).to_string();
    let mut tool_archive = adaptive
        .tool_history()
        .then(|| crate::tool_archive::ToolArchive::new(tool_archive_scope.clone()));
    let mut discovered_tools = std::collections::BTreeSet::new();

    // The host supplies canonical facts; the engine selects its model context.
    // Do this only at turn entry: trimming individual messages inside an active
    // tool cycle would break call/result pairing or discard the accepted input.
    let requirement = model_request.input.messages.last().cloned()
        .ok_or_else(|| AgentEngineError::ContextAssembly("missing accepted requirement".into()))?;
    let mut retained_inputs = vec![requirement.clone()];
    let mut steering_receipts = std::collections::BTreeSet::new();
    // Compact before the resource assembler would discard entire old turns.
    // Repository instructions are retained independently, never summarized away.
    let mut long_horizon = adaptive.task_ledger().then(LongHorizonState::default);
    let mut adaptive_slots = AdaptiveContextSlots::default();
    let live_context_slot = if let Some(port) = &request.live_context_port {
        let slot = model_request.input.instructions.len();
        model_request.input.instructions.push(crate::live_context::read(port.as_ref(),
            &model_request.causality, request.active_set_generation, &cancellation).await?);
        Some(slot)
    } else { None };
    synchronize_adaptive_context(
        &mut model_request,
        &request.tool_plan,
        &requested_tool_choice,
        &adaptive,
        &scoped_instructions,
        &patch_recovery,
        tool_archive.as_ref(),
        long_horizon.as_ref(),
        retained_inputs.len(),
        !request.context_resources.is_empty(),
        request.prior_task.is_some(),
        request.resource_port.is_some(),
        request.history_port.is_some(),
        request.tool_discovery_port.is_some(),
        &discovered_tools,
        &mut adaptive_slots,
    )?;
    model_request
        .validate()
        .map_err(|error| AgentEngineError::InvalidContract(error.to_string()))?;
    context_lifecycle.prepare(&mut model_request, &retained_inputs, &binding, model.clone(), event_sink.as_ref(), cancellation.clone()).await?;
    let mut input = model_request.input;
    let current = input.messages.pop().ok_or_else(|| AgentEngineError::ContextAssembly(
        "canonical context has no current message".into(),
    ))?;
    let history = std::mem::take(&mut input.messages);
    let (input, diagnostics) = AgentContextAssembler::assemble(
        input, history, current, &crate::AgentsMdContext::default(), context_budget,
    )?;
    model_request.input = input;
    event_sink.emit(AgentEngineEvent::ContextPrepared {
        dropped_history_messages: diagnostics.dropped_history_messages,
        warnings: diagnostics.warnings,
    }).await?;

    let mut output_text = String::new();
    let mut reasoning_text = String::new();
    let mut model_steps = 0_u16;
    let mut tool_call_count = 0_u32;
    let mut provider_round_id = None;
    let mut completion_review_used = false;
    let mut admitted_call_ids = std::collections::BTreeSet::new();
    let mut stream_budget = crate::stream_limits::StreamBudget::default();
    let mut output_limit_recovery = crate::output_limit::OutputLimitRecovery::default();

    if cancellation.is_cancelled() {
        return cancelled_turn(
            &event_sink,
            &agent_session_id,
            &turn_operation_id,
            model_steps,
            &output_text,
            &reasoning_text,
            tool_call_count,
            provider_round_id.clone(),
        )
        .await;
    }

    'model_steps: while model_steps < request.max_model_steps {
        if cancellation.is_cancelled() {
            return cancelled_turn(
                &event_sink,
                &agent_session_id,
                &turn_operation_id,
                model_steps,
                &output_text,
                &reasoning_text,
                tool_call_count,
                provider_round_id.clone(),
            )
            .await;
        }

        // This boundary is reached only after the previous batch has settled.
        // Never compact a stream with incomplete tool calls/results.
        if let Some(port) = &request.input_port {
            let inputs = port.take(&model_request.causality, false).await?;
            if crate::steering::incorporate(inputs, &mut model_request, &mut retained_inputs, &mut steering_receipts)? {
                completion_review_used = false;
                adaptive
                    .activate(
                        crate::adaptive::LEDGER_MODULES,
                        crate::AgentRuntimeActivationReason::Steering,
                        event_sink.as_ref(),
                    )
                    .await?;
                let state = long_horizon.get_or_insert_with(LongHorizonState::default);
                state.execution_plan.needs_replan = true;
                state.completion.invalidate();
            }
        }
        let refreshed_instructions = scoped_instructions.before_model(tools.as_ref(), event_sink.as_ref(), cancellation.clone()).await?;
        // before_calls may already have replaced layers. Comparing
        // the actual model slot also catches those changes on read-only batches
        // where no effect marked the instruction view dirty.
        if refreshed_instructions {
            completion_review_used = false;
            model_request.input.provider_round_parent = None;
            if let Some(state) = long_horizon.as_mut() {
                state.execution_plan.needs_replan = true;
                state.completion.invalidate();
                if adaptive.task_ledger() {
                    event_sink.emit(AgentEngineEvent::PlanUpdated {
                        plan: state.execution_plan.clone(),
                    }).await?;
                }
            }
        }
        let recovery_context = patch_recovery.context();
        if adaptive_slots
            .patch_recovery
            .is_some_and(|slot| model_request.input.instructions[slot] != recovery_context)
        {
            completion_review_used = false;
            model_request.input.provider_round_parent = None;
            if let Some(state) = long_horizon.as_mut() {
                state.execution_plan.needs_replan = true;
                state.completion.invalidate();
            }
        }
        if let (Some(port), Some(slot)) = (&request.live_context_port, live_context_slot) {
            let latest = crate::live_context::read(port.as_ref(), &model_request.causality,
                request.active_set_generation, &cancellation).await?;
            if model_request.input.instructions[slot] != latest {
                model_request.input.provider_round_parent = None;
                completion_review_used = false;
                if let Some(state) = long_horizon.as_mut() {
                    state.completion.invalidate();
                }
            }
            model_request.input.instructions[slot] = latest;
        }
        synchronize_adaptive_context(
            &mut model_request,
            &request.tool_plan,
            &requested_tool_choice,
            &adaptive,
            &scoped_instructions,
            &patch_recovery,
            tool_archive.as_ref(),
            long_horizon.as_ref(),
            retained_inputs.len(),
            !request.context_resources.is_empty(),
            request.prior_task.is_some(),
            request.resource_port.is_some(),
            request.history_port.is_some(),
            request.tool_discovery_port.is_some(),
            &discovered_tools,
            &mut adaptive_slots,
        )?;
        context_lifecycle.prepare(&mut model_request, &retained_inputs, &binding, model.clone(), event_sink.as_ref(), cancellation.clone()).await?;
        let context_bytes = serde_json::to_vec(&model_request.input)
            .map_err(|error| AgentEngineError::ContextAssembly(error.to_string()))?.len();
        if context_bytes > context_budget.max_context_bytes {
            return fail_turn(&event_sink, model_steps,
                format!("active turn context still exceeds the Nomi byte budget after compaction ({} > {})",
                    context_bytes, context_budget.max_context_bytes)).await;
        }
        model_steps = model_steps.saturating_add(1);
        let model_operation_id =
            OperationId::from(format!("{}:model:{}", turn_operation_id.as_ref(), model_steps));
        model_request.causality.operation_id = model_operation_id.clone();
        event_sink
            .emit(AgentEngineEvent::ModelStepStarted {
                step: model_steps,
                operation_id: model_operation_id,
            })
            .await?;

        let open_stream = model.open_stream(model_request.clone(), cancellation.clone());
        let mut stream = match tokio::select! {
            _ = cancellation.cancelled() => {
                return cancelled_turn(
                    &event_sink,
                    &agent_session_id,
                    &turn_operation_id,
                    model_steps,
                    &output_text,
                    &reasoning_text,
                    tool_call_count,
                    provider_round_id.clone(),
                )
                .await;
            }
            result = open_stream => result,
        } {
            Ok(stream) => stream,
            Err(error)
                if cancellation.is_cancelled()
                    || error.code == ChatModelErrorCode::Cancelled =>
            {
                return cancelled_turn(
                    &event_sink,
                    &agent_session_id,
                    &turn_operation_id,
                    model_steps,
                    &output_text,
                    &reasoning_text,
                    tool_call_count,
                    provider_round_id.clone(),
                )
                .await;
            }
            Err(error) => {
                if model_steps < request.max_model_steps
                    && context_lifecycle.request_overflow_recovery(&error, false, &model_request.input)?
                {
                    event_sink.emit(AgentEngineEvent::ContextLimitRecoveryStarted {
                        rejected_step: model_steps,
                    }).await?;
                    model_request.input.provider_round_parent = None;
                    continue 'model_steps;
                }
                return Err(AgentEngineError::from_model_error(error));
            }
        };
        let mut step = StepState::default();
        let mut saw_terminal = false;
        let mut semantic_output_seen = false;

        loop {
            let next = tokio::select! {
                _ = cancellation.cancelled() => {
                    return cancelled_turn(
                        &event_sink,
                        &agent_session_id,
                        &turn_operation_id,
                        model_steps,
                        &output_text,
                        &reasoning_text,
                        tool_call_count,
                        provider_round_id.clone(),
                    )
                    .await;
                }
                item = stream.next() => item,
            };

            let Some(item) = next else {
                break;
            };
            let event = match item {
                Ok(event) => event,
                Err(error)
                    if cancellation.is_cancelled()
                        || error.code == ChatModelErrorCode::Cancelled =>
                {
                    return cancelled_turn(
                        &event_sink,
                        &agent_session_id,
                        &turn_operation_id,
                        model_steps,
                        &output_text,
                        &reasoning_text,
                        tool_call_count,
                        provider_round_id.clone(),
                    )
                    .await;
                }
                Err(error) => {
                    if model_steps < request.max_model_steps
                        && context_lifecycle.request_overflow_recovery(&error, semantic_output_seen, &model_request.input)?
                    {
                        // Release the failed stream before a new model claim.
                        // No tool from this rejected step has been admitted.
                        drop(stream);
                        event_sink.emit(AgentEngineEvent::ContextLimitRecoveryStarted {
                            rejected_step: model_steps,
                        }).await?;
                        model_request.input.provider_round_parent = None;
                        continue 'model_steps;
                    }
                    return Err(AgentEngineError::from_model_error(error));
                }
            };
            stream_budget.admit(&event)?;
            semantic_output_seen |= event.is_semantic_output();
            match event {
                ChatModelEvent::ResponseStarted { .. } => {}
                ChatModelEvent::OutputTextDelta { text } => {
                    if text.is_empty() {
                        return fail_turn(
                            &event_sink,
                            model_steps,
                            "model emitted an empty output text delta",
                        )
                        .await;
                    }
                    output_text.push_str(&text);
                    step.append_text(&text);
                    event_sink
                        .emit(AgentEngineEvent::OutputTextDelta {
                            step: model_steps,
                            text,
                        })
                        .await?;
                }
                ChatModelEvent::ReasoningDelta { text } => {
                    if text.is_empty() {
                        return fail_turn(
                            &event_sink,
                            model_steps,
                            "model emitted an empty reasoning delta",
                        )
                        .await;
                    }
                    reasoning_text.push_str(&text);
                    step.append_reasoning(&text);
                    event_sink
                        .emit(AgentEngineEvent::ReasoningDelta {
                            step: model_steps,
                            text,
                        })
                        .await?;
                }
                ChatModelEvent::ReasoningSignature { signature } => {
                    if signature.is_empty() {
                        return fail_turn(
                            &event_sink,
                            model_steps,
                            "model emitted an empty reasoning signature",
                        )
                        .await;
                    }
                    step.set_reasoning_signature(signature)?;
                }
                ChatModelEvent::ReasoningBlock { text, encrypted_content } => {
                    if step.pending_reasoning_signature.is_some()
                        || encrypted_content.as_ref().is_some_and(String::is_empty)
                        || (text.is_empty() && encrypted_content.is_none())
                    {
                        return fail_turn(&event_sink, model_steps, "invalid completed reasoning block").await;
                    }
                    // Retain empty-summary opaque state and distinct block
                    // boundaries in live model history. Never log the blob.
                    step.assistant_content.push(ChatContentPart::Reasoning {
                        text: text.clone(), signature: None, encrypted_content,
                    });
                    if !text.is_empty() {
                        reasoning_text.push_str(&text);
                        event_sink.emit(AgentEngineEvent::ReasoningDelta { step: model_steps, text }).await?;
                    }
                }
                ChatModelEvent::ProviderReasoningBlock { block } => {
                    block.validate().map_err(|message| AgentEngineError::InvalidModelEvent(message.into()))?;
                    if step.pending_reasoning_signature.is_some() {
                        return fail_turn(&event_sink, model_steps, "provider block conflicts with a pending reasoning signature").await;
                    }
                    // Only ordinary thinking text reaches the event journal
                    // and UI. Signed/hidden payload stays in live context.
                    if let Some(text) = block.visible_text() {
                        reasoning_text.push_str(text);
                        event_sink.emit(AgentEngineEvent::ReasoningDelta {
                            step: model_steps, text: text.to_owned(),
                        }).await?;
                    }
                    step.assistant_content.push(ChatContentPart::ProviderReasoning { block });
                }
                ChatModelEvent::ToolCallDelta {
                    call_id,
                    name,
                    arguments_delta,
                } => {
                    if admitted_call_ids.contains(&call_id) && !step.calls.contains_key(&call_id) {
                        return fail_turn(&event_sink, model_steps, "model reused a prior or discarded tool-call identity").await;
                    }
                    step.record_tool_delta(&call_id, &name, &arguments_delta)?;
                    event_sink
                        .emit(AgentEngineEvent::ToolCallDelta {
                            step: model_steps,
                            call_id,
                            name,
                            arguments_delta,
                        })
                        .await?;
                }
                ChatModelEvent::ToolCallCompleted { call } => {
                    if call.call_id.as_ref().starts_with("agent-instructions:")
                        || !admitted_call_ids.insert(call.call_id.clone()) {
                        return fail_turn(&event_sink, model_steps, "model reused a tool-call identity within this turn").await;
                    }
                    step.record_tool_completed(&call)?;
                    event_sink
                        .emit(AgentEngineEvent::ToolCallCompleted {
                            step: model_steps,
                            call,
                        })
                        .await?;
                }
                ChatModelEvent::ProviderRoundId { round_id } => {
                    provider_round_id = Some(round_id.clone());
                    step.provider_round_id = Some(round_id);
                }
                ChatModelEvent::Usage { usage } => {
                    context_lifecycle.observe_usage(&usage);
                    event_sink
                        .emit(AgentEngineEvent::Usage {
                            step: model_steps,
                            usage,
                        })
                        .await?;
                }
                ChatModelEvent::Completed { finish_reason } => {
                    if saw_terminal {
                        return fail_turn(
                            &event_sink,
                            model_steps,
                            "model emitted more than one terminal event",
                        )
                        .await;
                    }
                    saw_terminal = true;
                    step.finish_reason = Some(finish_reason);
                    break;
                }
                ChatModelEvent::NativeResponsesItem { item_type, .. } => {
                    return fail_turn(
                        &event_sink,
                        model_steps,
                        format!(
                            "native Responses item {item_type:?} is not yet representable in Nomi history"
                        ),
                    )
                    .await;
                }
                ChatModelEvent::OutputAudioDelta { .. } => {
                    return fail_turn(
                        &event_sink,
                        model_steps,
                        "audio output is outside the Nomi Runtime core",
                    )
                    .await;
                }
            }
        }

        if !saw_terminal {
            if cancellation.is_cancelled() {
                return cancelled_turn(
                    &event_sink,
                    &agent_session_id,
                    &turn_operation_id,
                    model_steps,
                    &output_text,
                    &reasoning_text,
                    tool_call_count,
                    provider_round_id.clone(),
                )
                .await;
            }
            return Err(AgentEngineError::ModelStreamEndedWithoutTerminal);
        }

        let finish_reason = step
            .finish_reason
            .ok_or(AgentEngineError::ModelStreamEndedWithoutTerminal)?;
        if cancellation.is_cancelled() || matches!(finish_reason, ChatFinishReason::Cancelled) {
            return cancelled_turn(
                &event_sink,
                &agent_session_id,
                &turn_operation_id,
                model_steps,
                &output_text,
                &reasoning_text,
                tool_call_count,
                provider_round_id.clone(),
            )
                .await;
        }
        if matches!(finish_reason, ChatFinishReason::MaxOutputTokens) {
            // No invoke_tool_calls/admit_tool path has run for this step.
            // Even complete calls in a truncated batch are NOT executable.
            let discarded_tool_call_ids = step.call_order.clone();
            crate::output_limit::validate_discarded(model_steps, &discarded_tool_call_ids)?;
            let continuation = output_limit_recovery.admit(model_steps < request.max_model_steps);
            event_sink.emit(AgentEngineEvent::ModelOutputTruncated {
                step: model_steps, discarded_tool_call_ids: discarded_tool_call_ids.clone(), continuation,
            }).await?;
            admitted_call_ids.extend(discarded_tool_call_ids);
            crate::output_limit::retain_partial_text(&mut model_request, &step.assistant_content);
            model_request.input.messages.push(crate::output_limit::notice(continuation));
            provider_round_id = None;
            if !continuation {
                return fail_turn(&event_sink, model_steps,
                    "model output remained truncated after the bounded continuation budget; task completion was not accepted").await;
            }
            // Steering, instruction refresh, compaction, cancellation and all
            // normal owner fences still run at the next model-step boundary.
            continue 'model_steps;
        }
        step.finalize()?;
        if !step.has_tool_calls() && matches!(finish_reason, ChatFinishReason::ToolCalls) {
            return fail_turn(
                &event_sink,
                model_steps,
                "model returned a tool-call terminal reason without Tool Calls",
            )
            .await;
        }
        // A correction accepted while the model was streaming invalidates its
        // proposed batch. Close every call/result pair without executing it.
        if let Some(port) = &request.input_port {
            let inputs = port.take(&model_request.causality, false).await?;
            if !inputs.is_empty() {
                adaptive
                    .activate(
                        crate::adaptive::LONG_HORIZON_MODULES,
                        crate::AgentRuntimeActivationReason::Steering,
                        event_sink.as_ref(),
                    )
                    .await?;
                let state = long_horizon.get_or_insert_with(LongHorizonState::default);
                let archive = tool_archive.get_or_insert_with(|| {
                    crate::tool_archive::ToolArchive::new(tool_archive_scope.clone())
                });
                append_assistant_step(&mut model_request, &step)?;
                let deferred = step.call_order.iter().map(|id| (id.clone(), Ok(AgentToolResult::text(id.clone(),
                    "Not executed: a new accepted user instruction arrived. Reconsider the task and update_plan before further effects.", true)))).collect();
                let results = finish_tool_results(deferred, event_sink.as_ref(), model_steps, &cancellation).await?;
                tool_call_count = tool_call_count.saturating_add(results.len() as u32);
                for (_, result) in results {
                    if let Some(call) = step.calls.get(&result.call_id).and_then(|pending| pending.completed.as_ref()) {
                        archive.record(&call.name, &call.call_id, &call.arguments,
                            &result.output, result.is_error, Some(model_steps), Some(false))?;
                    }
                    model_request.input.messages.push(ChatMessage { role: ChatRole::Tool,
                        content: vec![ChatContentPart::ToolResult { call_id: result.call_id, output: result.output, is_error: result.is_error }], provider_round_id: None });
                }
                crate::steering::incorporate(inputs, &mut model_request, &mut retained_inputs, &mut steering_receipts)?;
                state.execution_plan.needs_replan = true;
                state.completion.invalidate();
                completion_review_used = false;
                continue;
            }
        }
        if step.has_tool_calls() {
            if !matches!(finish_reason, ChatFinishReason::ToolCalls) {
                return fail_turn(
                    &event_sink,
                    model_steps,
                    "model returned Tool Calls with a non-tool terminal reason",
                )
                .await;
            }

            let mut external_calls = 0usize;
            let mut effectful = false;
            let mut explicit_continuation = false;
            let mut explicit_plan = false;
            for call_id in &step.call_order {
                let call = step
                    .calls
                    .get(call_id)
                    .and_then(|pending| pending.completed.as_ref())
                    .ok_or_else(|| {
                        AgentEngineError::InvalidModelEvent(
                            "incomplete tool call during adaptive classification".into(),
                        )
                    })?;
                if let Some(tool) = request.tool_plan.binding(&call.name) {
                    external_calls = external_calls.saturating_add(1);
                    effectful |= !matches!(tool.effect_class, AgentEffectClass::ReadOnly);
                } else if matches!(
                    call.name.as_str(),
                    crate::remote_resources::LIST
                        | crate::remote_resources::READ
                        | crate::remote_resources::TEMPLATES
                ) {
                    external_calls = external_calls.saturating_add(1);
                    // Opening a remote resource can start a connection or
                    // local stdio owner even when the requested operation is a read.
                    effectful = true;
                } else if call.name == crate::context_resources::TOOL_NAME {
                    external_calls = external_calls.saturating_add(1);
                } else if call.name == crate::task_continuation::TOOL_NAME {
                    explicit_continuation = true;
                } else if call.name == crate::planning::TOOL_NAME {
                    explicit_plan = true;
                }
            }
            let discovery_only = step.call_order.iter().all(|call_id| {
                step.calls
                    .get(call_id)
                    .and_then(|pending| pending.completed.as_ref())
                    .is_some_and(|call| call.name == crate::tool_discovery::TOOL_NAME)
            });
            if !discovery_only {
                adaptive
                    .activate(
                        crate::adaptive::TOOL_MODULES,
                        crate::AgentRuntimeActivationReason::ToolCall,
                        event_sink.as_ref(),
                    )
                    .await?;
            }
            let multi_step = adaptive.observe_external_batch(external_calls);
            if explicit_continuation {
                adaptive
                    .activate(
                        crate::adaptive::LONG_HORIZON_MODULES
                            .into_iter()
                            .chain([crate::AgentRuntimeModule::TaskContinuation]),
                        crate::AgentRuntimeActivationReason::ExplicitTaskContinuation,
                        event_sink.as_ref(),
                    )
                    .await?;
            } else if explicit_plan {
                adaptive
                    .activate(
                        crate::adaptive::LEDGER_MODULES,
                        crate::AgentRuntimeActivationReason::ExplicitPlan,
                        event_sink.as_ref(),
                    )
                    .await?;
            } else if effectful {
                adaptive
                    .activate(
                        crate::adaptive::LONG_HORIZON_MODULES,
                        crate::AgentRuntimeActivationReason::EffectfulToolCall,
                        event_sink.as_ref(),
                    )
                    .await?;
            } else if multi_step {
                adaptive
                    .activate(
                        crate::adaptive::LONG_HORIZON_MODULES,
                        crate::AgentRuntimeActivationReason::MultiStepToolUse,
                        event_sink.as_ref(),
                    )
                    .await?;
            }
            let state = long_horizon.get_or_insert_with(LongHorizonState::default);
            let archive = tool_archive.get_or_insert_with(|| {
                crate::tool_archive::ToolArchive::new(tool_archive_scope.clone())
            });

            append_assistant_step(&mut model_request, &step)?;
            let dispatch = crate::tool_dispatch::ToolDispatchBatch::new(tools.as_ref(), &step.call_order);
            let results = match invoke_tool_calls(
                &agent_session_id,
                &request.principal,
                &model_request,
                &request.active_set_generation,
                &request.tool_plan,
                &dispatch,
                event_sink.as_ref(),
                &step,
                model_steps,
                &cancellation,
                &mut state.execution_plan,
                &mut state.completion,
                &mut state.work_status,
                &retained_inputs,
                &mut scoped_instructions,
                &mut patch_recovery,
                &request.context_resources,
                &request.context_image_input,
                request.input_port.as_deref(),
                request.prior_task.as_ref(),
                request.resource_port.as_deref(),
                request.tool_discovery_port.as_deref(),
                &mut discovered_tools,
                archive,
                request.history_port.as_deref(),
                &binding,
            )
            .await
            {
                Ok(results) => results,
                Err(AgentEngineError::Cancelled) => {
                    return cancelled_turn(
                        &event_sink,
                        &agent_session_id,
                        &turn_operation_id,
                        model_steps,
                        &output_text,
                        &reasoning_text,
                        tool_call_count,
                        provider_round_id.clone(),
                    )
                    .await;
                }
                Err(error) => return Err(error),
            };
            tool_call_count = tool_call_count.saturating_add(results.len() as u32);
            for (expected_call_id, result) in results {
                result.validate_for(&expected_call_id)?;
                if let Some(call) = step.calls.get(&expected_call_id).and_then(|pending| pending.completed.as_ref()) {
                    if let Some(binding) = request.tool_plan.binding(&call.name) {
                        let attempted = dispatch.attempted(&expected_call_id)?;
                        if attempted {
                            state.work_status.observe(
                                binding,
                                call,
                                &result,
                                &mut state.command_tracker,
                            );
                        } else {
                            if !result.is_error {
                                return Err(AgentEngineError::InvalidContract("unattempted platform tool returned success".into()));
                            }
                            state.work_status.observe_deferred();
                        }
                        if attempted && binding.action_id.as_ref() == "workspace.files/read" && state.work_status.running_processes.is_empty() {
                            patch_recovery.observe_read(call, &result);
                        }
                        if (attempted && (!matches!(binding.effect_class, crate::AgentEffectClass::ReadOnly)
                            || binding.capability_id.as_ref() == "workspace.process"))
                            || !state.work_status.running_processes.is_empty()
                        {
                            // Failed calls may have partial effects too.
                            scoped_instructions.invalidate();
                        }
                        let observation = state.completion.observe(
                            &state.work_status,
                            binding,
                            call,
                            &result,
                            attempted,
                        );
                        if adaptive.task_ledger() {
                            event_sink.emit(AgentEngineEvent::CompletionObservation { observation }).await?;
                        }
                        // New observations require a fresh completion account;
                        // the model-step bound limits repeated work/review.
                        completion_review_used = false;
                        if result.is_error {
                            state.execution_plan.needs_replan = true;
                        }
                    } else if call.name != crate::completion::TOOL_NAME {
                        // Planning/resource control can change the
                        // model's account even though it supplies no evidence.
                        state.completion.invalidate();
                        completion_review_used = false;
                    }
                }
                if let Some(call) = step.calls.get(&expected_call_id).and_then(|pending| pending.completed.as_ref()) {
                    let attempted = if request.tool_plan.binding(&call.name).is_some() {
                        Some(dispatch.attempted(&expected_call_id)?)
                    } else { None };
                    archive.record(&call.name, &call.call_id, &call.arguments,
                        &result.output, result.is_error, Some(model_steps), attempted)?;
                }
                // Evidence/patch recovery and the archive above consume the
                // original result. Only the next model's context is reduced.
                let context_kind = step.calls.get(&expected_call_id)
                    .and_then(|pending| pending.completed.as_ref())
                    .and_then(|call| request.tool_plan.binding(&call.name))
                            .and_then(|binding| crate::tool_context::ToolContextKind::for_action(binding.action_id.as_ref()));
                let result = match context_kind {
                    Some(kind) if dispatch.attempted(&expected_call_id)? => crate::tool_context::project(kind, &result),
                    _ => result,
                };
                model_request.input.messages.push(ChatMessage {
                    role: ChatRole::Tool,
                    content: vec![ChatContentPart::ToolResult {
                        call_id: result.call_id,
                        output: result.output,
                        is_error: result.is_error,
                    }],
                    provider_round_id: None,
                });
            }
            patch_recovery.end_batch();
            patch_recovery.persist(event_sink.as_ref()).await?;
            if adaptive.task_ledger() {
                event_sink.emit(AgentEngineEvent::WorkStatus {
                    status: state.work_status.clone(),
                }).await?;
                event_sink.emit(AgentEngineEvent::PlanUpdated {
                    plan: state.execution_plan.clone(),
                }).await?;
            }
            if let Some(round_id) = step.provider_round_id {
                model_request.input.provider_round_parent = Some(round_id);
            }
            model_request
                .validate()
                .map_err(|error| AgentEngineError::InvalidContract(error.to_string()))?;
            continue;
        }

        append_assistant_step(&mut model_request, &step)?;
        // At most one evidence review, never an unbounded self-retry. The
        // model may report a blocker/unverified result instead of invoking a
        // command; a user prohibition on verification remains authoritative.
        if matches!(finish_reason, ChatFinishReason::Completed) && adaptive.task_ledger() {
            let state = long_horizon.as_ref().ok_or_else(|| {
                AgentEngineError::InvalidContract(
                    "active task ledger has no turn-local state".into(),
                )
            })?;
            if state
                .completion
                .current(
                    &state.execution_plan,
                    &state.work_status,
                    retained_inputs.len(),
                )
                .is_none()
                && !completion_review_used
                && model_steps < request.max_model_steps
            {
                completion_review_used = true;
                event_sink.emit(AgentEngineEvent::CompletionReview {
                    status: state.work_status.clone(),
                }).await?;
                model_request.input.messages.push(state.work_status.completion_review_message()?);
                model_request.input.provider_round_parent = None;
                continue;
            }
        }
        if matches!(finish_reason, ChatFinishReason::Completed)
            && adaptive.task_ledger()
            && long_horizon
                .as_ref()
                .is_some_and(|state| state.execution_plan.is_open())
        {
            return fail_turn(&event_sink, model_steps, "execution plan remains unresolved; completion was not accepted").await;
        }
        if matches!(finish_reason, ChatFinishReason::Completed) && patch_recovery.pending() {
            return fail_turn(&event_sink, model_steps, "failed patch targets have not been re-observed; task completion was not accepted").await;
        }
        if matches!(finish_reason, ChatFinishReason::Completed)
            && long_horizon
                .as_ref()
                .is_some_and(|state| !state.work_status.running_processes.is_empty())
        {
            return fail_turn(&event_sink, model_steps, "processes remain running; poll or cancel them explicitly before completion (host cleanup will still reap them)").await;
        }
        if matches!(finish_reason, ChatFinishReason::Completed) && adaptive.task_ledger() {
            let state = long_horizon.as_ref().ok_or_else(|| {
                AgentEngineError::InvalidContract(
                    "active task ledger has no turn-local state".into(),
                )
            })?;
            let Some(report) = state.completion.current(
                &state.execution_plan,
                &state.work_status,
                retained_inputs.len(),
            ) else {
                return fail_turn(&event_sink, model_steps, "completion account is missing or stale; call report_completion after the latest plan, input and tool observations").await;
            };
            if report.is_blocked() {
                return fail_turn(&event_sink, model_steps, "completion account contains blocked work; this turn cannot be published as task completion").await;
            }
        }
        model_request
            .validate()
            .map_err(|error| AgentEngineError::InvalidContract(error.to_string()))?;
        if let Some(port) = &request.input_port {
            let inputs = port.take(&model_request.causality, true).await?;
            if crate::steering::incorporate(inputs, &mut model_request, &mut retained_inputs, &mut steering_receipts)? {
                completion_review_used = false;
                adaptive
                    .activate(
                        crate::adaptive::LEDGER_MODULES,
                        crate::AgentRuntimeActivationReason::Steering,
                        event_sink.as_ref(),
                    )
                    .await?;
                let state = long_horizon.get_or_insert_with(LongHorizonState::default);
                state.execution_plan.needs_replan = true;
                state.completion.invalidate();
                continue;
            }
        }
        if matches!(finish_reason, ChatFinishReason::Completed) {
            if let Some(report) = long_horizon.as_ref().and_then(|state| {
                state.completion.current(
                    &state.execution_plan,
                    &state.work_status,
                    retained_inputs.len(),
                )
            }) {
                if let Some(disclosure) = report.unverified_disclosure() {
                    output_text.push_str(&disclosure);
                    event_sink.emit(AgentEngineEvent::OutputTextDelta { step: model_steps, text: disclosure }).await?;
                }
            }
        }
        let result = AgentTurnResult {
            agent_session_id,
            turn_operation_id,
            model_steps,
            output_text,
            reasoning_text,
            tool_call_count,
            provider_round_id,
            terminal: AgentTurnTerminal::Completed { finish_reason },
        };
        event_sink
            .emit(AgentEngineEvent::TurnCompleted {
                model_steps,
                finish_reason,
            })
            .await?;
        return Ok(result);
    }

    fail_turn(
        &event_sink,
        model_steps,
        format!(
            "model step limit of {} exceeded",
            request.max_model_steps
        ),
    )
    .await
}

#[derive(Default)]
struct LongHorizonState {
    execution_plan: crate::AgentPlan,
    work_status: crate::AgentWorkStatus,
    completion: crate::completion::CompletionTracker,
    command_tracker: crate::workflow::CommandTracker,
}

#[derive(Default)]
struct AdaptiveContextSlots {
    long_horizon_policy: Option<usize>,
    scoped_instructions: Option<usize>,
    patch_recovery: Option<usize>,
    tool_history: Option<usize>,
    task_plan: Option<usize>,
    completion: Option<usize>,
}

#[allow(clippy::too_many_arguments)]
fn synchronize_adaptive_context(
    request: &mut ChatModelRequest,
    plan: &AgentToolPlan,
    requested_tool_choice: &ChatToolChoice,
    adaptive: &crate::adaptive::AdaptiveExecution,
    scoped_instructions: &crate::workspace_context::ScopedInstructions,
    patch_recovery: &crate::patch_recovery::PatchRecovery,
    tool_archive: Option<&crate::tool_archive::ToolArchive>,
    long_horizon: Option<&LongHorizonState>,
    input_revision: usize,
    resources: bool,
    prior_task: bool,
    remote_resources: bool,
    history: bool,
    tool_discovery: bool,
    discovered_tools: &std::collections::BTreeSet<String>,
    slots: &mut AdaptiveContextSlots,
) -> Result<(), AgentEngineError> {
    configure_tools(
        request,
        plan,
        resources,
        prior_task,
        remote_resources,
        history,
        adaptive.task_ledger(),
        adaptive.tool_history(),
        tool_discovery,
        discovered_tools,
    )?;
    request.input.tool_choice = if request.input.tools.is_empty() {
        ChatToolChoice::None
    } else if matches!(requested_tool_choice, ChatToolChoice::None) {
        ChatToolChoice::Auto
    } else {
        requested_tool_choice.clone()
    };
    if adaptive.task_ledger() {
        let state = long_horizon.ok_or_else(|| {
            AgentEngineError::InvalidContract("active task ledger has no turn-local state".into())
        })?;
        upsert_instruction(
            &mut request.input.instructions,
            &mut slots.long_horizon_policy,
            crate::workflow::LONG_HORIZON_EXECUTION_INSTRUCTIONS.into(),
        );
        upsert_instruction(
            &mut request.input.instructions,
            &mut slots.task_plan,
            state.execution_plan.context()?,
        );
        upsert_instruction(
            &mut request.input.instructions,
            &mut slots.completion,
            state.completion.context(
                &state.execution_plan,
                &state.work_status,
                input_revision,
            )?,
        );
    }
    if scoped_instructions.has_context() || slots.scoped_instructions.is_some() {
        upsert_instruction(
            &mut request.input.instructions,
            &mut slots.scoped_instructions,
            scoped_instructions.context(),
        );
    }
    if patch_recovery.pending() || slots.patch_recovery.is_some() {
        upsert_instruction(
            &mut request.input.instructions,
            &mut slots.patch_recovery,
            patch_recovery.context(),
        );
    }
    if adaptive.tool_history() {
        let archive = tool_archive.ok_or_else(|| {
            AgentEngineError::InvalidContract("active tool history has no turn-local archive".into())
        })?;
        upsert_instruction(
            &mut request.input.instructions,
            &mut slots.tool_history,
            archive.context(),
        );
    }
    Ok(())
}

fn upsert_instruction(
    instructions: &mut Vec<String>,
    slot: &mut Option<usize>,
    value: String,
) {
    if let Some(index) = *slot {
        instructions[index] = value;
    } else {
        *slot = Some(instructions.len());
        instructions.push(value);
    }
}

#[allow(clippy::too_many_arguments)]
fn configure_tools(
    request: &mut ChatModelRequest,
    plan: &AgentToolPlan,
    resources: bool,
    prior_task: bool,
    remote_resources: bool,
    history: bool,
    task_ledger: bool,
    tool_history: bool,
    tool_discovery: bool,
    discovered_tools: &std::collections::BTreeSet<String>,
) -> Result<(), AgentEngineError> {
    // Retired capability-control names remain reserved so a host cannot
    // accidentally restore the old dynamic authority surface as ordinary tools.
    if [crate::planning::TOOL_NAME, crate::completion::TOOL_NAME, crate::context_resources::TOOL_NAME, crate::tool_discovery::TOOL_NAME, "search_capabilities", "activate_capability", crate::task_continuation::TOOL_NAME, crate::remote_resources::LIST, crate::remote_resources::READ, crate::remote_resources::TEMPLATES, crate::tool_archive::SEARCH, crate::tool_archive::READ, crate::tool_archive::LOAD]
        .iter().any(|name| plan.binding(name).is_some()) {
        return Err(AgentEngineError::InvalidContract("engine control tool names cannot be shadowed".into()));
    }
    request.input.tools = crate::tool_discovery::definitions(plan, discovered_tools);
    if tool_discovery {
        request.input.tools.push(crate::tool_discovery::definition());
    }
    if task_ledger {
        request.input.tools.push(crate::planning::definition());
        request.input.tools.push(crate::completion::definition());
    }
    if tool_history {
        request.input.tools.extend(crate::tool_archive::definitions());
        if history { request.input.tools.push(crate::tool_archive::load_definition()); }
    }
    if prior_task { request.input.tools.push(crate::task_continuation::definition()); }
    if resources { request.input.tools.push(crate::context_resources::definition()); }
    if remote_resources { request.input.tools.extend(crate::remote_resources::definitions()); }
    Ok(())
}

async fn fail_turn(
    event_sink: &Arc<dyn AgentEventSink>,
    model_steps: u16,
    message: impl Into<String>,
) -> Result<AgentTurnResult, AgentEngineError> {
    let message = message.into();
    event_sink
        .emit(AgentEngineEvent::TurnFailed {
            model_steps,
            message: message.clone(),
        })
        .await?;
    Err(AgentEngineError::TurnFailed(message))
}

async fn cancelled_turn(
    event_sink: &Arc<dyn AgentEventSink>,
    agent_session_id: &AgentSessionId,
    turn_operation_id: &OperationId,
    model_steps: u16,
    output_text: &str,
    reasoning_text: &str,
    tool_call_count: u32,
    provider_round_id: Option<ProviderRoundId>,
) -> Result<AgentTurnResult, AgentEngineError> {
    event_sink
        .emit(AgentEngineEvent::TurnCancelled { model_steps })
        .await?;
    Ok(AgentTurnResult {
        agent_session_id: agent_session_id.clone(),
        turn_operation_id: turn_operation_id.clone(),
        model_steps,
        output_text: output_text.to_owned(),
        reasoning_text: reasoning_text.to_owned(),
        tool_call_count,
        provider_round_id,
        terminal: AgentTurnTerminal::Cancelled,
    })
}

async fn invoke_tool_calls(
    agent_session_id: &AgentSessionId,
    principal: &PrincipalRef,
    model_request: &ChatModelRequest,
    active_set_generation: &u64,
    plan: &AgentToolPlan,
    invoker: &dyn AgentToolInvoker,
    event_sink: &dyn AgentEventSink,
    step: &StepState,
    model_step: u16,
    cancellation: &CancellationToken,
    execution_plan: &mut crate::AgentPlan,
    completion: &mut crate::completion::CompletionTracker,
    work_status: &mut crate::AgentWorkStatus,
    accepted_inputs: &[ChatMessage],
    scoped_instructions: &mut crate::workspace_context::ScopedInstructions,
    patch_recovery: &mut crate::patch_recovery::PatchRecovery,
    context_resources: &BTreeMap<String, crate::AgentContextResource>,
    context_image_input: &bool,
    input_port: Option<&dyn crate::AgentInputPort>,
    prior_task: Option<&crate::AgentPriorTask>,
    resource_port: Option<&dyn nomifun_engine_core::EngineResourcePort>,
    tool_discovery_port: Option<&dyn crate::AgentToolDiscoveryPort>,
    discovered_tools: &mut std::collections::BTreeSet<String>,
    tool_archive: &mut crate::tool_archive::ToolArchive,
    history_port: Option<&dyn crate::AgentHistoryPort>,
    engine_binding: &EngineBinding,
) -> Result<Vec<(ToolCallId, AgentToolResult)>, AgentEngineError> {
    let completed = step.call_order.iter().map(|id| step.calls.get(id).and_then(|pending| pending.completed.clone())
        .ok_or_else(|| AgentEngineError::InvalidModelEvent("incomplete tool call".into())))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(results) = crate::tool::reject_unexposed_batch(&completed, &model_request.input.tools) {
        if cancellation.is_cancelled() { return Err(AgentEngineError::Cancelled); }
        // This precedes control handlers, instruction discovery and platform
        // admission. No internal control or workspace effect ran.
        // The ordinary result path records all call/result pairs and feeds the
        // error back on the next bounded model step, without transport replay.
        execution_plan.needs_replan = true;
        completion.invalidate();
        for call in &completed {
            if plan.binding(&call.name).is_none() {
                work_status.observe_deferred();
            }
        }
        return finish_tool_results(results, event_sink, model_step, cancellation).await;
    }
    if completed
        .iter()
        .any(|call| call.name == crate::tool_discovery::TOOL_NAME)
    {
        let mut results = Vec::with_capacity(completed.len());
        for call in &completed {
            let result = if call.name != crate::tool_discovery::TOOL_NAME || completed.len() != 1 {
                AgentToolResult::text(
                    call.call_id.clone(),
                    "No tools executed: ToolSearch requires one isolated call.",
                    true,
                )
            } else if let Some(port) = tool_discovery_port {
                crate::tool_discovery::execute(
                    call,
                    plan,
                    discovered_tools,
                    port,
                    &model_request.causality,
                    *active_set_generation,
                    cancellation.clone(),
                )
                .await?
            } else {
                AgentToolResult::text(
                    call.call_id.clone(),
                    "ToolSearch is unavailable for this frozen AgentSession.",
                    true,
                )
            };
            results.push((call.call_id.clone(), Ok(result)));
        }
        return finish_tool_results(results, event_sink, model_step, cancellation).await;
    }
    if completed.iter().any(|call| call.name == crate::tool_archive::LOAD) {
        let mut results = Vec::new();
        for call in &completed {
            let result = if let Some(port) = history_port.filter(|_| completed.len() == 1) {
                tool_archive.load(call, port, &model_request.causality, engine_binding, cancellation).await?
            } else {
                AgentToolResult::text(call.call_id.clone(), "No tools executed: historical loading requires an available platform history port and one call alone.", true)
            };
            results.push((call.call_id.clone(), Ok(result)));
        }
        return finish_tool_results(results, event_sink, model_step, cancellation).await;
    }
    if completed.iter().any(|call| matches!(call.name.as_str(), crate::tool_archive::SEARCH | crate::tool_archive::READ)) {
        let isolated = completed.len() == 1;
        let results = completed.iter().map(|call| {
            let result = if isolated { tool_archive.handle(call) } else {
                AgentToolResult::text(call.call_id.clone(), "No tools executed: submit one search/read history call alone before effects or other controls.", true)
            };
            (call.call_id.clone(), Ok(result))
        }).collect();
        return finish_tool_results(results, event_sink, model_step, cancellation).await;
    }
    if completed.iter().any(|call| matches!(call.name.as_str(), crate::remote_resources::LIST | crate::remote_resources::READ | crate::remote_resources::TEMPLATES)) {
        let mut results = Vec::new();
        for call in &completed {
            let result = if let Some(port) = resource_port.filter(|_| completed.len() == 1) {
                match crate::remote_resources::prepare(call, *context_image_input) {
                    Err(result) => result,
                    Ok(prepared) => {
                        if cancellation.is_cancelled() { return Err(AgentEngineError::Cancelled); }
                        // Resource sessions can start stdio/OAuth/remote work.
                        // This is a conservative observation fence, not proof
                        // of host admission, execution or a workspace mutation.
                        patch_recovery.invalidate_observations();
                        patch_recovery.persist(event_sink).await?;
                        scoped_instructions.invalidate();
                        work_status.before_resource_request();
                        completion.invalidate();
                        event_sink.emit(AgentEngineEvent::WorkStatus { status: work_status.clone() }).await?;
                        crate::remote_resources::execute(prepared, port, call, &model_request.causality,
                            *active_set_generation, cancellation).await?
                    }
                }
            } else {
                AgentToolResult::text(call.call_id.clone(), "No tools executed: MCP resources require an available host port and a single-call batch.", true)
            };
            results.push((call.call_id.clone(), Ok(result)));
        }
        return finish_tool_results(results, event_sink, model_step, cancellation).await;
    }
    if completed.iter().any(|call| call.name == crate::task_continuation::TOOL_NAME) {
        let mut results = Vec::new();
        for call in &completed {
            if cancellation.is_cancelled() { return Err(AgentEngineError::Cancelled); }
            let result = if completed.len() == 1 {
                crate::task_continuation::resume(prior_task, call, execution_plan, work_status, accepted_inputs, event_sink).await?
            } else {
                AgentToolResult::text(call.call_id.clone(), "No tools executed: resume_task requires a single-call batch before planning/effects.", true)
            };
            results.push((call.call_id.clone(), Ok(result)));
        }
        return finish_tool_results(results, event_sink, model_step, cancellation).await;
    }
    // Internal control updates are isolated from effects. No Kernel capability
    // is invented and no model-selected name can shadow the engine planner.
    if completed.iter().any(|call| call.name == crate::completion::TOOL_NAME) {
        let mut results = Vec::new();
        for call in &completed {
            let result = if completed.len() == 1 {
                completion.submit(call, execution_plan, work_status, accepted_inputs, event_sink).await?
            } else {
                AgentToolResult::text(call.call_id.clone(), "No tools executed: submit report_completion alone after the plan and all observations are settled.", true)
            };
            results.push((call.call_id.clone(), Ok(result)));
        }
        return finish_tool_results(results, event_sink, model_step, cancellation).await;
    }
    if completed.iter().any(|call| call.name == crate::planning::TOOL_NAME) {
        let mut results = Vec::new();
        for call in &completed {
            let result = if completed.len() == 1 {
                execution_plan.update(call, accepted_inputs, event_sink).await?
            } else {
                AgentToolResult::text(call.call_id.clone(), "No tools executed: submit update_plan alone, then submit execution calls in a subsequent batch.", true)
            };
            results.push((call.call_id.clone(), Ok(result)));
        }
        return finish_tool_results(results, event_sink, model_step, cancellation).await;
    }
    if completed.iter().any(|call| call.name == crate::context_resources::TOOL_NAME) {
        let isolated = completed.iter().all(|call| call.name == crate::context_resources::TOOL_NAME);
        let results = completed.iter().map(|call| {
            let result = if isolated { crate::context_resources::read(call, context_resources, *context_image_input, completed.len() == 1) }
                else { AgentToolResult::text(call.call_id.clone(), "No tools executed: read context resources in a separate batch before workspace calls.", true) };
            (call.call_id.clone(), Ok(result))
        }).collect();
        return finish_tool_results(results, event_sink, model_step, cancellation).await;
    }
    let workspace_image = completed.iter().any(|call|
        plan.binding(&call.name).is_some_and(|binding| binding.action_id.as_ref() == "workspace.files/read")
            && call.arguments.0.get("format").and_then(|value| value.as_str()) == Some("image"));
    if workspace_image && (completed.len() != 1 || !*context_image_input) {
        let results = completed.into_iter().map(|call| (call.call_id.clone(), Ok(AgentToolResult::text(
            call.call_id, "No tools executed: workspace image reads require a compatible exact model route and a single-call batch. Use read_file format=image without offset/limit only when image input is available; this turn cannot change model features.", true)))).collect();
        return finish_tool_results(results, event_sink, model_step, cancellation).await;
    }
    let gate = completed.iter().find_map(|call| {
        plan.binding(&call.name).and_then(|binding| {
            let cleanup = matches!(binding.action_id.as_ref(),
                "workspace.process/poll" | "workspace.process/cancel" | "workspace.process/close_stdin");
            if let Some(reason) = patch_recovery.gate(binding, call) { Some(reason) }
            else if !cleanup && !matches!(binding.effect_class, AgentEffectClass::ReadOnly) { execution_plan.effect_gate() }
            else { None }
        })
    });
    let deferred = if let Some(reason) = gate { Some(reason.to_owned()) }
        else { scoped_instructions.before_calls(&completed, invoker, event_sink, cancellation.clone()).await? };
    if let Some(reason) = deferred {
        let results = completed.into_iter().map(|call| (call.call_id.clone(), Ok(AgentToolResult::text(call.call_id, reason.clone(), true)))).collect();
        return finish_tool_results(results, event_sink, model_step, cancellation).await;
    }
    let mut calls = Vec::with_capacity(step.call_order.len());
    let mut can_parallelize = !step.call_order.is_empty();
    for call_id in &step.call_order {
        if cancellation.is_cancelled() {
            return Err(AgentEngineError::Cancelled);
        }
        let pending = step
            .calls
            .get(call_id)
            .ok_or_else(|| AgentEngineError::InvalidModelEvent("tool call order is corrupt".to_owned()))?;
        let call = pending
            .completed
            .clone()
            .ok_or_else(|| {
                AgentEngineError::InvalidModelEvent(format!(
                    "tool call {} was not completed before the model terminal event",
                    call_id.as_ref()
                ))
            })?;
        let binding = plan
            .binding(&call.name)
            .ok_or_else(|| AgentEngineError::ToolNotExposed(call.name.clone()))?;
        can_parallelize &= binding.parallel_safe
            && matches!(binding.effect_class, AgentEffectClass::ReadOnly);
        let invocation = invocation_for(
            agent_session_id.clone(),
            principal.clone(),
            model_request.causality.resolved_snapshot_ref.clone(),
            *active_set_generation,
            &model_request.causality.turn_operation_id,
            call,
            binding,
        );
        calls.push(invocation);
    }

    if can_parallelize {
        let invocations = calls.into_iter().enumerate().map(|(index, invocation)| async move {
            let call_id = invocation.call.call_id.clone();
            let result = async {
                if cancellation.is_cancelled() {
                    return Err(AgentEngineError::Cancelled);
                }
                if let Some(port) = input_port {
                    if port.has_pending(&model_request.causality).await? {
                        return Ok(steering_deferred(call_id.clone()));
                    }
                }
                if !record_tool_admission(event_sink, model_step, &invocation).await? {
                    return Ok(steering_deferred(call_id.clone()));
                }
                invoker.invoke(invocation, cancellation.clone()).await
            }.await;
            (index, call_id, result)
        });
        let mut pending = invocations.collect::<futures::stream::FuturesUnordered<_>>();
        let mut ordered = vec![None; completed.len()];
        // Owners retain admitted tasks independently of these waiter futures.
        // Record each returned read without waiting for an unrelated slow read;
        // scoped instruction postprocessing remains serial and precedes output.
        while !pending.is_empty() {
            let next = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(AgentEngineError::Cancelled),
                next = pending.next() => next,
            };
            let Some((index, call_id, result)) = next else { break; };
            let call = completed.get(index).ok_or_else(|| AgentEngineError::InvalidContract(
                "parallel result index is outside its frozen batch".into()))?;
            if call.call_id != call_id || ordered[index].is_some() {
                return Err(AgentEngineError::InvalidContract("parallel result identity is inconsistent".into()));
            }
            let result = match result {
                Ok(result) => {
                    result.validate_for(&call_id)?;
                    Ok(scoped_instructions.after_call(call, result, invoker, event_sink, cancellation.clone()).await?.0)
                }
                Err(error) => Err(error),
            };
            ordered[index] = Some(record_tool_result(call_id, result, event_sink, model_step).await?);
        }
        if cancellation.is_cancelled() { return Err(AgentEngineError::Cancelled); }
        let results = ordered.into_iter().map(|result| result.ok_or_else(||
            AgentEngineError::InvalidContract("parallel batch has no recorded result".into())))
            .collect::<Result<Vec<_>, _>>()?;
        // Live model input remains proposal-ordered. Persist that ordering so
        // replay does not accidentally use asynchronous completion timing.
        event_sink.emit(AgentEngineEvent::ToolResultsOrdered {
            step: model_step, call_ids: results.iter().map(|(id, _)| id.clone()).collect(),
        }).await?;
        return Ok(results);
    }

    let mut results = Vec::new();
    let mut defer_remaining: Option<String> = None;
    for invocation in calls {
        if cancellation.is_cancelled() {
            return Err(AgentEngineError::Cancelled);
        }
        let call_id = invocation.call.call_id.clone();
        if let Some(reason) = &defer_remaining {
            let result = AgentToolResult::text(call_id.clone(), reason.clone(), true);
            results.push(record_tool_result(call_id, Ok(result), event_sink, model_step).await?);
            continue;
        }
        // A preceding serial effect can change repository instructions. Do
        // not let later calls in the same batch use the pre-effect view.
        if !results.is_empty() {
            if let Some(reason) = scoped_instructions.before_calls(std::slice::from_ref(&invocation.call), invoker, event_sink, cancellation.clone()).await? {
                defer_remaining = Some(reason.clone());
                let result = AgentToolResult::text(call_id.clone(), reason, true);
                results.push(record_tool_result(call_id, Ok(result), event_sink, model_step).await?);
                continue;
            }
        }
        if let Some(port) = input_port {
            if port.has_pending(&model_request.causality).await? {
                let reason = "Not executed: new user input is waiting at the next model boundary; reconsider remaining calls.".to_owned();
                defer_remaining = Some(reason.clone());
                let result = AgentToolResult::text(call_id.clone(), reason, true);
                results.push(record_tool_result(call_id, Ok(result), event_sink, model_step).await?);
                continue;
            }
        }
        if let Some(reason) = patch_recovery.gate(&invocation.binding, &invocation.call) {
            defer_remaining = Some(reason.to_owned());
            let result = AgentToolResult::text(call_id.clone(), reason, true);
            results.push(record_tool_result(call_id, Ok(result), event_sink, model_step).await?);
            continue;
        }
        if !record_tool_admission(event_sink, model_step, &invocation).await? {
            let result = steering_deferred(call_id.clone());
            defer_remaining = Some(result.output_text());
            results.push(record_tool_result(call_id, Ok(result), event_sink, model_step).await?);
            continue;
        }
        let patch_call = (invocation.binding.action_id.as_ref() == "workspace.files/patch").then(|| invocation.call.clone());
        if let Some(call) = &patch_call {
            // Persist BEFORE the invoker owns the effect, including cancellation
            // and crash windows where no engine result can ever be observed.
            if let Err(error) = patch_recovery.arm(call) {
                let reason = format!("Not executed: {error}");
                defer_remaining = Some(reason.clone());
                let result = AgentToolResult::text(call_id.clone(), reason, true);
                results.push(record_tool_result(call_id, Ok(result), event_sink, model_step).await?);
                continue;
            }
            patch_recovery.persist(event_sink).await?;
        }
        if patch_call.is_none() && (invocation.binding.capability_id.as_ref() == "workspace.process"
            || !matches!(invocation.binding.effect_class, crate::AgentEffectClass::ReadOnly)) {
            patch_recovery.invalidate_observations();
            patch_recovery.persist(event_sink).await?;
        }
        let observed_call = invocation.call.clone();
        let result = tokio::select! {
            _ = cancellation.cancelled() => return Err(AgentEngineError::Cancelled),
            result = invoker.invoke(invocation, cancellation.clone()) => result,
        };
        let result = match result {
            Ok(result) => {
                // A malformed/misattributed response must stop this batch
                // before it can clear patch obligations or admit another effect.
                result.validate_for(&call_id)?;
                let (result, reconsider) = scoped_instructions.after_call(&observed_call, result, invoker, event_sink, cancellation.clone()).await?;
                if reconsider {
                    defer_remaining = Some("Not executed: an earlier search discovered changed repository instructions or withheld its snippets because required context was unavailable. Read the current context, reconsider remaining calls and submit new call identities.".into());
                }
                Ok(result)
            }
            Err(error) => Err(error),
        };
        // Persist every settled result before the next serial effect. A later
        // cancellation/failure cannot erase the already recorded prefix. This
        // is a tool observation, not owner cleanup or task-success proof.
        let recorded = record_tool_result(call_id, result, event_sink, model_step).await?;
        if recorded.1.is_error {
            if let Some(call) = patch_call { patch_recovery.failed(&call); }
            defer_remaining = Some("Not executed: an earlier serial call failed. Inspect its result and update_plan before retrying remaining effects.".into());
        } else if patch_call.is_some() {
            patch_recovery.published_successfully();
            patch_recovery.persist(event_sink).await?;
        }
        results.push(recorded);
        if cancellation.is_cancelled() {
            break;
        }
    }
    if cancellation.is_cancelled() {
        return Err(AgentEngineError::Cancelled);
    }
    Ok(results)
}

fn steering_deferred(call_id: ToolCallId) -> AgentToolResult {
    AgentToolResult::text(call_id, "Not executed: new user input is waiting at the next model boundary; reconsider remaining calls.", true)
}

async fn record_tool_admission(sink: &dyn AgentEventSink, step: u16, invocation: &crate::AgentToolInvocation) -> Result<bool, AgentEngineError> {
    sink.admit_tool(AgentEngineEvent::ToolStarted {
        step, call_id: invocation.call.call_id.clone(), capability_id: invocation.binding.capability_id.clone(),
        action_id: invocation.binding.action_id.clone(),
    }).await
}

async fn finish_tool_results(
    results: Vec<(
        ToolCallId,
        Result<AgentToolResult, AgentEngineError>,
    )>,
    event_sink: &dyn AgentEventSink,
    model_step: u16,
    cancellation: &CancellationToken,
) -> Result<Vec<(ToolCallId, AgentToolResult)>, AgentEngineError> {
    if cancellation.is_cancelled() {
        return Err(AgentEngineError::Cancelled);
    }
    let mut normalized = Vec::with_capacity(results.len());
    for (call_id, result) in results {
        normalized.push(record_tool_result(call_id, result, event_sink, model_step).await?);
    }
    Ok(normalized)
}

/// Do not discard a returned observation merely because its token changed.
/// Callers stop admitting more work at their next boundary. Outer driver drops
/// and storage failures still follow the host's durable-outcome contract;
/// cancellation without a returned result is never synthesized as success.
async fn record_tool_result(
    call_id: ToolCallId,
    result: Result<AgentToolResult, AgentEngineError>,
    event_sink: &dyn AgentEventSink,
    model_step: u16,
) -> Result<(ToolCallId, AgentToolResult), AgentEngineError> {
    let result = match result {
        Ok(result) => result,
        Err(AgentEngineError::Cancelled) => return Err(AgentEngineError::Cancelled),
        // Only an ordinary owner invocation failure is a model-observable tool
        // result. Structural/host contract failures must terminate the turn;
        // allowing the model to continue would hide a failed enforcement layer.
        Err(error @ AgentEngineError::ToolInvocation(_))
        | Err(error @ AgentEngineError::CapabilityKernel { .. }) => {
            AgentToolResult::text(call_id.clone(), error.to_string(), true)
        }
        Err(error) => return Err(error),
    };
    result.validate_for(&call_id)?;
    event_sink.emit(AgentEngineEvent::ToolCompleted {
        step: model_step, result: result.clone(),
    }).await?;
    Ok((call_id, result))
}

fn append_assistant_step(
    request: &mut ChatModelRequest,
    step: &StepState,
) -> Result<(), AgentEngineError> {
    if step.assistant_content.is_empty() {
        return Ok(());
    }
    request.input.messages.push(ChatMessage {
        role: ChatRole::Assistant,
        content: step.assistant_content.clone(),
        provider_round_id: step.provider_round_id.clone(),
    });
    Ok(())
}

#[derive(Default)]
struct StepState {
    assistant_content: Vec<ChatContentPart>,
    calls: BTreeMap<ToolCallId, PendingToolCall>,
    call_order: Vec<ToolCallId>,
    finish_reason: Option<ChatFinishReason>,
    provider_round_id: Option<ProviderRoundId>,
    pending_reasoning_signature: Option<String>,
}

impl StepState {
    fn append_text(&mut self, text: &str) {
        if let Some(ChatContentPart::Text { text: existing }) = self.assistant_content.last_mut()
        {
            existing.push_str(text);
        } else {
            self.assistant_content
                .push(ChatContentPart::Text { text: text.to_owned() });
        }
    }

    fn append_reasoning(&mut self, text: &str) {
        if let Some(ChatContentPart::Reasoning { text: existing, signature: None, encrypted_content: None }) =
            self.assistant_content.last_mut()
        {
            existing.push_str(text);
        } else {
            self.assistant_content.push(ChatContentPart::Reasoning {
                text: text.to_owned(),
                signature: self.pending_reasoning_signature.take(),
                encrypted_content: None,
            });
        }
    }

    fn set_reasoning_signature(
        &mut self,
        signature: String,
    ) -> Result<(), AgentEngineError> {
        if let Some(ChatContentPart::Reasoning {
            signature: existing,
            text,
            ..
        }) = self.assistant_content.last_mut()
        {
            if existing.is_some() {
                return Err(AgentEngineError::InvalidModelEvent(
                    "model emitted more than one reasoning signature".to_owned(),
                ));
            }
            if text.is_empty() {
                self.pending_reasoning_signature = Some(signature);
            } else {
                *existing = Some(signature);
            }
        } else {
            if self.pending_reasoning_signature.is_some() {
                return Err(AgentEngineError::InvalidModelEvent(
                    "model emitted more than one reasoning signature".to_owned(),
                ));
            }
            self.pending_reasoning_signature = Some(signature);
        }
        Ok(())
    }

    fn record_tool_delta(
        &mut self,
        call_id: &ToolCallId,
        name: &str,
        arguments_delta: &str,
    ) -> Result<(), AgentEngineError> {
        crate::stream_limits::identity(call_id, name)?;
        if !self.calls.contains_key(call_id) && self.calls.len() >= MAX_CALLS_PER_STEP {
            return Err(AgentEngineError::InvalidModelEvent("model exceeded the per-step tool-call limit".into()));
        }
        if call_id.as_ref().trim().is_empty() {
            return Err(AgentEngineError::InvalidModelEvent(
                "tool call delta has an empty call id".to_owned(),
            ));
        }
        let entry = self.calls.entry(call_id.clone()).or_insert_with(|| {
            self.call_order.push(call_id.clone());
            PendingToolCall {
                name: String::new(),
                arguments: String::new(),
                completed: None,
            }
        });
        if entry.completed.is_some() {
            return Err(AgentEngineError::InvalidModelEvent(format!(
                "tool call {} emitted a delta after completion",
                call_id.as_ref()
            )));
        }
        if !name.is_empty() {
            if !entry.name.is_empty() && entry.name != name {
                return Err(AgentEngineError::InvalidModelEvent(format!(
                    "tool call {} changed its name",
                    call_id.as_ref()
                )));
            }
            entry.name = name.to_owned();
        }
        entry.arguments.push_str(arguments_delta);
        validate_tool_argument_size(&entry.arguments)
    }

    fn record_tool_completed(&mut self, call: &ChatToolCall) -> Result<(), AgentEngineError> {
        crate::stream_limits::completed(call)?;
        if !self.calls.contains_key(&call.call_id) && self.calls.len() >= MAX_CALLS_PER_STEP {
            return Err(AgentEngineError::InvalidModelEvent("model exceeded the per-step tool-call limit".into()));
        }
        call.validate()
            .map_err(|error| AgentEngineError::InvalidModelEvent(error.to_string()))?;
        if !call.arguments.0.is_object() {
            return Err(AgentEngineError::InvalidModelEvent(format!(
                "tool call {} arguments must be a JSON object",
                call.call_id.as_ref()
            )));
        }
        let entry = self.calls.entry(call.call_id.clone()).or_insert_with(|| {
            self.call_order.push(call.call_id.clone());
            PendingToolCall {
                name: call.name.clone(),
                arguments: String::new(),
                completed: None,
            }
        });
        if !entry.name.is_empty() && entry.name != call.name {
            return Err(AgentEngineError::InvalidModelEvent(format!(
                "tool call {} changed its name before completion",
                call.call_id.as_ref()
            )));
        }
        if entry.completed.is_some() {
            return Err(AgentEngineError::InvalidModelEvent(format!(
                "tool call {} completed more than once",
                call.call_id.as_ref()
            )));
        }
        parse_completed_arguments(call)?;
        if !entry.arguments.is_empty() {
            let assembled = serde_json::from_str::<serde_json::Value>(&entry.arguments)
                .map_err(|error| {
                    AgentEngineError::InvalidModelEvent(format!(
                        "tool call {} emitted invalid assembled arguments: {error}",
                        call.call_id.as_ref()
                    ))
                })?;
            if assembled != call.arguments.0 {
                return Err(AgentEngineError::InvalidModelEvent(format!(
                    "tool call {} completed arguments differ from assembled deltas",
                    call.call_id.as_ref()
                )));
            }
        }
        entry.name = call.name.clone();
        entry.completed = Some(call.clone());
        self.assistant_content.push(ChatContentPart::ToolCall {
            call_id: call.call_id.clone(),
            name: call.name.clone(),
            arguments: call.arguments.clone(),
            provider_metadata: call.provider_metadata.clone(),
        });
        Ok(())
    }

    fn has_tool_calls(&self) -> bool {
        !self.call_order.is_empty()
    }

    fn finalize(&self) -> Result<(), AgentEngineError> {
        for call_id in &self.call_order {
            let call = self
                .calls
                .get(call_id)
                .ok_or_else(|| AgentEngineError::InvalidModelEvent("tool call order is corrupt".to_owned()))?;
            if call.completed.is_none() {
                return Err(AgentEngineError::InvalidModelEvent(format!(
                    "tool call {} was not completed before the model terminal event",
                    call_id.as_ref()
                )));
            }
        }
        if self.pending_reasoning_signature.is_some()
            || self.assistant_content.iter().any(|part| {
            matches!(
                part,
                ChatContentPart::Reasoning { text, encrypted_content, .. }
                    if text.is_empty() && encrypted_content.is_none()
            )
        })
        {
            return Err(AgentEngineError::InvalidModelEvent(
                "reasoning signature was emitted without reasoning text".to_owned(),
            ));
        }
        Ok(())
    }
}

struct PendingToolCall {
    name: String,
    arguments: String,
    completed: Option<ChatToolCall>,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;
    use crate::engine::{
        AgentEngine, AgentEngineBuild, EngineBuildId,
    };
    use crate::events::NoopAgentEventSink;
    use crate::model::AgentModelStream;
    use crate::tool::{
        AgentEffectClass, AgentToolBinding, AgentToolInvocation, AgentToolInvoker,
        AgentToolPlan, AgentToolResult,
    };
    use async_trait::async_trait;
    use futures::stream;
    use nomifun_agent_contracts::{
        ActionId, AgentSessionId, ChatRouteIdentity, DigestHex, EventId, ModelRouteId,
        OperationId, PrincipalRef, ResolvedSnapshotId, ResolvedSnapshotRef, VersionString,
    };
    use nomifun_chat_model_broker::{
        ChatCausality, ChatContentPart, ChatModelError, ChatModelEvent, ChatToolDefinition,
    };
    use serde_json::json;
    use tokio::sync::Notify;

    fn binding() -> EngineBinding {
        let engine = AgentEngine::new(AgentEngineBuild {
            build_id: EngineBuildId::from("coding-dev"),
            build_digest: DigestHex::from("a".repeat(64)),
        })
        .unwrap();
        engine
            .bind(
                AgentSessionId::from("session"),
                nomifun_agent_contracts::RuntimeBindingId::from("binding"),
                ResolvedSnapshotRef {
                    snapshot_id: ResolvedSnapshotId::from("snapshot"),
                    snapshot_digest: DigestHex::from("b".repeat(64)),
                },
            )
            .unwrap()
    }

    fn request() -> ChatModelRequest {
        ChatModelRequest {
            contract_version: VersionString::from(
                nomifun_chat_model_broker::CHAT_MODEL_CONTRACT_VERSION,
            ),
            route: ChatRouteIdentity::new(
                "preset@1",
                "agent_chat",
                ModelRouteId::from("route"),
                1,
            ),
            causality: ChatCausality {
                agent_session_id: AgentSessionId::from("session"),
                turn_operation_id: OperationId::from("turn"),
                causation_event_id: EventId::from("input"),
                resolved_snapshot_ref: binding().resolved_snapshot_ref().clone(),
                route_identity: ChatRouteIdentity::new(
                    "preset@1",
                    "agent_chat",
                    ModelRouteId::from("route"),
                    1,
                ),
                operation_id: OperationId::from("model"),
            },
            input: nomifun_chat_model_broker::ChatModelInput {
                instructions: vec!["coding".to_owned()],
                messages: vec![nomifun_chat_model_broker::ChatMessage {
                    role: ChatRole::User,
                    content: vec![ChatContentPart::Text {
                        text: "inspect".to_owned(),
                    }],
                    provider_round_id: None,
                }],
                tools: Vec::new(),
                tool_choice: ChatToolChoice::None,
                max_output_tokens: Some(100),
                reasoning: None,
                prompt_cache: nomifun_chat_model_broker::PromptCachePolicy::Disabled,
                response_format: nomifun_chat_model_broker::ChatResponseFormat::Text,
                requested_output_modalities: BTreeSet::new(),
                provider_round_parent: None,
                preserve_native_responses_items: false,
                metadata: Default::default(),
            },
        }
    }

    fn tool_plan() -> AgentToolPlan {
        AgentToolPlan::new([tool_binding(
            "read_file",
            "workspace.files",
            "workspace.files/read",
            AgentEffectClass::ReadOnly,
            true,
        )])
        .unwrap()
    }

    #[test]
    fn frozen_tool_surface_never_advertises_capability_activation() {
        let mut request = request();
        let plan = tool_plan();
        configure_tools(&mut request, &plan, true, true, true, true, false, false, false, &Default::default()).unwrap();
        assert!(request.input.tools.iter().any(|tool| tool.name == "read_file"));
        assert!(!request.input.tools.iter().any(|tool| tool.name == crate::planning::TOOL_NAME));
        assert!(!request.input.tools.iter().any(|tool| tool.name == crate::tool_archive::SEARCH));
        configure_tools(&mut request, &plan, true, true, true, true, true, true, false, &Default::default()).unwrap();
        assert!(request.input.tools.iter().any(|tool| tool.name == crate::planning::TOOL_NAME));
        assert!(request.input.tools.iter().any(|tool| tool.name == crate::completion::TOOL_NAME));
        assert!(request.input.tools.iter().any(|tool| tool.name == crate::tool_archive::SEARCH));
        assert!(!request.input.tools.iter().any(|tool|
            matches!(tool.name.as_str(), "activate_capability" | "search_capabilities")));
        for name in ["activate_capability", "search_capabilities"] {
            let calls = [ChatToolCall {
                call_id: ToolCallId::from("retired-control"),
                name: name.into(),
                arguments: nomifun_agent_contracts::StrictJsonValue(json!({"capability_id":"workspace.files"})),
                provider_metadata: None,
            }];
            let rejected = crate::tool::reject_unexposed_batch(&calls, &request.input.tools).unwrap();
            assert_eq!(rejected.len(), 1);
            assert!(rejected[0].1.as_ref().unwrap().is_error);
            let shadow = AgentToolPlan::new([tool_binding(
                name, "workspace.files", "workspace.files/read", AgentEffectClass::ReadOnly, true,
            )]).unwrap();
            assert!(configure_tools(&mut request, &shadow, false, false, false, false, false, false, false, &Default::default()).is_err());
        }
    }

    fn tool_binding(
        model_name: &str,
        capability_id: &str,
        action_id: &str,
        effect_class: AgentEffectClass,
        parallel_safe: bool,
    ) -> AgentToolBinding {
        let definition = ChatToolDefinition {
            name: model_name.to_owned(),
            description: "Agent Runtime tool".to_owned(),
            input_schema: nomifun_agent_contracts::StrictJsonValue(json!({
                "type": "object",
                "properties": {"path": {"type": "string"}}
            })),
            deferred: false,
        };
        AgentToolBinding {
            model_name: model_name.to_owned(),
            schema_digest: crate::tool::input_schema_digest(&definition.input_schema).unwrap(),
            canonical_input_schema_ref: nomifun_agent_contracts::CanonicalSchemaRef::from(
                format!("schema://{capability_id}/input"),
            ),
            capability_contract_digest: DigestHex::from("c".repeat(64)),
            definition,
            capability_id: nomifun_agent_contracts::CapabilityId::from(capability_id),
            action_id: ActionId::from(action_id),
            resource_binding_ids: BTreeSet::new(),
            effect_class,
            parallel_safe,
        }
    }

    fn two_tool_plan(effect_class: AgentEffectClass, parallel_safe: bool) -> AgentToolPlan {
        if effect_class == AgentEffectClass::ManagedEffect {
            return AgentToolPlan::new([
                tool_binding("read_file", "workspace.files", "workspace.files/read", AgentEffectClass::ReadOnly, true),
                tool_binding("write_file", "workspace.files", "workspace.files/write", effect_class, parallel_safe),
                tool_binding("write_other_file", "workspace.files", "workspace.files/write", effect_class, parallel_safe),
            ]).unwrap();
        }
        AgentToolPlan::new([
            tool_binding(
                "read_file",
                "workspace.files",
                "workspace.files/read",
                effect_class,
                parallel_safe,
            ),
            tool_binding(
                "search_files",
                "workspace.files",
                "workspace.files/search",
                effect_class,
                parallel_safe,
            ),
        ])
        .unwrap()
    }

    struct ScriptedModel {
        steps: std::sync::Mutex<Vec<Vec<Result<ChatModelEvent, ChatModelError>>>>,
    }

    #[async_trait]
    impl AgentModelPort for ScriptedModel {
        async fn open_stream(
            &self,
            _request: ChatModelRequest,
            _cancellation: CancellationToken,
        ) -> Result<AgentModelStream, ChatModelError> {
            let events = self.steps.lock().unwrap().remove(0);
            Ok(Box::pin(stream::iter(events)))
        }
    }

    struct PendingModel {
        started: tokio::sync::mpsc::UnboundedSender<CancellationToken>,
    }

    #[async_trait]
    impl AgentModelPort for PendingModel {
        async fn open_stream(
            &self,
            _request: ChatModelRequest,
            cancellation: CancellationToken,
        ) -> Result<AgentModelStream, ChatModelError> {
            self.started.send(cancellation).unwrap();
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn dropping_turn_cancels_broker_and_releases_admission_without_cancelling_parent() {
        let (started, mut observed) = tokio::sync::mpsc::unbounded_channel();
        let session = Arc::new(open_session(Arc::new(PendingModel { started }), Arc::new(EchoTool)));
        let parent = CancellationToken::new();
        let task_session = Arc::clone(&session);
        let task_parent = parent.clone();
        let task = tokio::spawn(async move {
            task_session.run_turn_cancellable(
                AgentTurnRequest::new(request(), AgentToolPlan::default(), principal(), 1),
                task_parent,
            ).await
        });
        let broker_token = tokio::time::timeout(Duration::from_secs(2), observed.recv()).await.unwrap().unwrap();
        assert!(session.is_turn_active().await);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(broker_token.is_cancelled());
        assert!(!parent.is_cancelled());
        assert!(!session.is_turn_active().await);

        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let result = session.run_turn_cancellable(
            AgentTurnRequest::new(request(), AgentToolPlan::default(), principal(), 1),
            cancelled,
        ).await.unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Cancelled));
        assert!(observed.try_recv().is_err(), "a pre-cancelled turn must not invoke the model");
    }

    #[tokio::test]
    async fn host_cancellation_during_model_open_returns_cancelled_and_allows_next_turn() {
        let (started, mut observed) = tokio::sync::mpsc::unbounded_channel();
        let session = Arc::new(open_session(Arc::new(PendingModel { started }), Arc::new(EchoTool)));
        let parent = CancellationToken::new();
        let task_session = Arc::clone(&session);
        let task_parent = parent.clone();
        let task = tokio::spawn(async move {
            task_session.run_turn_cancellable(
                AgentTurnRequest::new(request(), AgentToolPlan::default(), principal(), 1), task_parent,
            ).await
        });
        tokio::time::timeout(Duration::from_secs(2), observed.recv()).await.unwrap().unwrap();
        parent.cancel();
        let result = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Cancelled));
        assert!(!session.is_turn_active().await);
    }

    struct ObservingModel {
        steps: std::sync::Mutex<Vec<Vec<Result<ChatModelEvent, ChatModelError>>>>,
        requests: std::sync::Mutex<Vec<ChatModelRequest>>,
    }

    #[async_trait]
    impl AgentModelPort for ObservingModel {
        async fn open_stream(
            &self,
            request: ChatModelRequest,
            _cancellation: CancellationToken,
        ) -> Result<AgentModelStream, ChatModelError> {
            self.requests.lock().unwrap().push(request);
            let events = self.steps.lock().unwrap().remove(0);
            Ok(Box::pin(stream::iter(events)))
        }
    }

    struct EchoTool;

    // Internal instruction observations are real workspace.files/read envelopes, not model
    // tool calls. Keep them out of dispatch/concurrency/cancellation counters.
    fn instruction_result(invocation: &AgentToolInvocation) -> Option<AgentToolResult> {
        if invocation.binding.action_id.as_ref() != "workspace.files/read" {
            return None;
        }
        let args = &invocation.call.arguments.0;
        let path = args["path"].as_str().expect("fixture read requires a path");
        let value = if args["missing_ok"] == true {
            assert!(matches!(path, "AGENTS.md" | "AGENTS.override.md"));
            json!({"kind":"workspace_file_absent", "path":path})
        } else if args["format"] == "instruction_scope" {
            assert!(matches!(path, "README.md" | "a" | "b"));
            json!({"path":path, "canonical_path":path, "kind":"file",
                "recursive":args["recursive"], "directories":[""],
                "complete":true, "incomplete_reasons":[]})
        } else {
            return None;
        };
        Some(AgentToolResult::text(invocation.call.call_id.clone(), value.to_string(), false))
    }

    fn workspace_result(invocation: AgentToolInvocation) -> AgentToolResult {
        let args = &invocation.call.arguments.0;
        let value = match invocation.binding.action_id.as_ref() {
            "workspace.files/read" => {
                let content = "file contents";
                json!({"path":args["path"], "content":content, "offset":0,
                    "total_bytes":content.len(), "eof":true, "next_offset":null,
                    "sha256":nomifun_agent_contracts::digest_bytes(content.as_bytes())})
            }
            "workspace.files/search" => json!({"query":args["query"], "matches":[], "truncated":false,
                "incomplete_reasons":[], "files_scanned":1, "files_skipped":0,
                "source_bytes_read":13, "notice":"No matches in the fixture file."}),
            "workspace.files/write" => json!({"path":args["path"], "written":true}),
            other => panic!("unexpected fixture capability: {other}"),
        };
        AgentToolResult::text(invocation.call.call_id, value.to_string(), false)
    }

    fn text_step(text: &str) -> Vec<Result<ChatModelEvent, ChatModelError>> {
        vec![
            Ok(ChatModelEvent::OutputTextDelta { text: text.into() }),
            Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::Completed }),
        ]
    }

    fn control_step(id: &str, name: &str, arguments: serde_json::Value) -> Vec<Result<ChatModelEvent, ChatModelError>> {
        vec![
            Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall {
                call_id: id.into(), name: name.into(),
                arguments: nomifun_agent_contracts::StrictJsonValue(arguments), provider_metadata: None,
            }}),
            Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::ToolCalls }),
        ]
    }

    fn plan_step(status: &str, quote: &str) -> Vec<Result<ChatModelEvent, ChatModelError>> {
        control_step(&format!("plan-{status}"), crate::planning::TOOL_NAME, json!({
            "explanation":"Account for the requested workspace operations.",
            "plan":[{"step":"requested operations", "status":status}],
            "requirements":[{"id":"request", "description":quote,
                "source":{"input":0, "quote":quote}}]
        }))
    }

    fn completion_steps(quote: &str, evidence: &[&str], supported: bool) -> Vec<Vec<Result<ChatModelEvent, ChatModelError>>> {
        vec![
            plan_step("completed", quote),
            control_step("completion", crate::completion::TOOL_NAME, json!({
                "summary":"Requested operations returned.",
                "criteria":[{"step":"requested operations", "requirement_ids":["request"],
                    "disposition":if supported { "supported" } else { "unverified" },
                    "evidence_call_ids":evidence,
                    "rationale":if supported { "The requested reads returned successfully." }
                        else { "Writes returned, but their resulting contents were not independently verified." }}]
            })),
            text_step("done"),
        ]
    }

    #[async_trait]
    impl AgentToolInvoker for EchoTool {
        async fn invoke(
            &self,
            invocation: AgentToolInvocation,
            _cancellation: CancellationToken,
        ) -> Result<AgentToolResult, AgentEngineError> {
            Ok(instruction_result(&invocation).unwrap_or_else(|| workspace_result(invocation)))
        }
    }

    struct BlockingTool {
        started: Arc<Notify>,
    }

    struct ConcurrencyTool {
        active: AtomicUsize,
        max_active: AtomicUsize,
        order: std::sync::Mutex<Vec<String>>,
        delay: Duration,
    }

    #[derive(Debug)]
    struct OneSteer {
        input: std::sync::Mutex<Option<crate::AgentSteeringInput>>,
    }

    #[async_trait]
    impl crate::AgentInputPort for OneSteer {
        async fn take(
            &self,
            _causality: &ChatCausality,
            _close_if_empty: bool,
        ) -> Result<Vec<crate::AgentSteeringInput>, AgentEngineError> {
            Ok(self.input.lock().unwrap().take().into_iter().collect())
        }

        async fn has_pending(
            &self,
            _causality: &ChatCausality,
        ) -> Result<bool, AgentEngineError> {
            Ok(self.input.lock().unwrap().is_some())
        }
    }

    #[async_trait]
    impl AgentToolInvoker for ConcurrencyTool {
        async fn invoke(
            &self,
            invocation: AgentToolInvocation,
            _cancellation: CancellationToken,
        ) -> Result<AgentToolResult, AgentEngineError> {
            if let Some(result) = instruction_result(&invocation) {
                return Ok(result);
            }
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            let delay = if invocation.call.call_id.as_ref() == "call-1" { self.delay * 2 } else { self.delay };
            tokio::time::sleep(delay).await;
            self.order
                .lock()
                .unwrap()
                .push(invocation.call.call_id.as_ref().to_owned());
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(workspace_result(invocation))
        }
    }

    #[async_trait]
    impl AgentToolInvoker for BlockingTool {
        async fn invoke(
            &self,
            invocation: AgentToolInvocation,
            cancellation: CancellationToken,
        ) -> Result<AgentToolResult, AgentEngineError> {
            if let Some(result) = instruction_result(&invocation) {
                return Ok(result);
            }
            assert!(matches!(invocation.call.call_id.as_ref(), "call-cancel" | "call-active"));
            self.started.notify_one();
            cancellation.cancelled().await;
            Err(AgentEngineError::Cancelled)
        }
    }

    fn open_session(
        model: Arc<dyn AgentModelPort>,
        tools: Arc<dyn AgentToolInvoker>,
    ) -> crate::engine::AgentEngineSession {
        open_session_with_budget(model, tools, AgentContextBudget::default())
    }

    fn open_session_with_budget(
        model: Arc<dyn AgentModelPort>,
        tools: Arc<dyn AgentToolInvoker>,
        budget: AgentContextBudget,
    ) -> crate::engine::AgentEngineSession {
        let engine = AgentEngine::new(AgentEngineBuild {
            build_id: EngineBuildId::from("coding-dev"),
            build_digest: DigestHex::from("a".repeat(64)),
        })
        .unwrap().with_context_budget(budget).unwrap();
        engine
            .open_session(
                binding(),
                model,
                tools,
                Some(Arc::new(NoopAgentEventSink)),
            )
            .unwrap()
    }

    fn principal() -> PrincipalRef {
        PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "user-1".to_owned(),
        }
    }

    #[tokio::test]
    async fn engine_bounds_initial_history_before_calling_the_model() {
        let mut summary = text_step("Earlier user inputs: history-0 through history-7. Current request: inspect.");
        summary.insert(0, Ok(ChatModelEvent::ReasoningDelta { text: "private summarization reasoning".repeat(100) }));
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![
                summary,
                text_step("done"),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        // Isolate message-count pressure; mandatory engine instructions and
        // tool schemas must fit before historical data can be summarized.
        let budget = AgentContextBudget { max_history_messages: 4, max_context_bytes: 64 * 1024 };
        let session = open_session_with_budget(model.clone(), Arc::new(EchoTool),
            budget);
        let mut request = request();
        request.input.max_output_tokens = Some(3000);
        let current = request.input.messages[0].clone();
        request.input.messages = (0..8).map(|index| ChatMessage {
            role: ChatRole::User,
            content: vec![ChatContentPart::Text { text: format!("history-{index}") }],
            provider_round_id: None,
        }).chain([current.clone()]).collect();
        let result = session.run_turn(AgentTurnRequest::new(request, AgentToolPlan::default(), principal(), 0)).await.unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].input.max_output_tokens, Some(3000),
            "compaction uses the frozen total generation budget, not a visible-text byte/token guess");
        assert_eq!(requests[1].input.max_output_tokens, Some(3000));
        assert!(!serde_json::to_string(&requests[1].input).unwrap().contains("private summarization reasoning"));
        assert!(requests[0].input.tools.is_empty(), "history is summarized without tool authority");
        assert_eq!(requests[0].input.metadata.get("nomifun_task").map(String::as_str), Some("agent_compaction"));
        let source = serde_json::to_string(&requests[0].input.messages).unwrap();
        for index in 0..8 {
            assert!(source.contains(&format!("history-{index}")), "history must not be silently discarded");
        }
        assert_eq!(requests[1].input.messages.len(), 2);
        assert_eq!(requests[1].input.messages.last(), Some(&current));
        assert!(matches!(&requests[1].input.messages[0].content[0], ChatContentPart::Text { text }
            if text.contains("Derived summary") && text.contains("history-0 through history-7")));
        assert!(serde_json::to_vec(&requests[1].input).unwrap().len() <= budget.max_context_bytes);
    }

    #[tokio::test]
    async fn one_turn_can_compact_repeatedly_without_losing_the_accepted_requirement() {
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![
                text_step("summary-one"),
                text_step("summary-two"),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let mut request = request();
        request.input.max_output_tokens = Some(4096);
        let requirement = request.input.messages[0].clone();
        request.input.messages.insert(0, crate::context_lifecycle::text_message(
            ChatRole::Assistant,
            "older context one ".repeat(256),
        ));
        request.input.messages.insert(0, crate::context_lifecycle::text_message(
            ChatRole::User,
            "older context two ".repeat(256),
        ));
        let budget = AgentContextBudget {
            max_history_messages: 2,
            max_context_bytes: 64 * 1024,
        };
        let mut lifecycle = crate::context_lifecycle::ContextLifecycle::new(
            crate::AgentModelBudget::default(),
            budget,
        )
        .unwrap();
        let sink = NoopAgentEventSink;
        lifecycle
            .prepare(
                &mut request,
                std::slice::from_ref(&requirement),
                &binding(),
                model.clone(),
                &sink,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        request.input.messages.push(crate::context_lifecycle::text_message(
            ChatRole::Assistant,
            "new tool-loop context ".repeat(256),
        ));
        request.input.messages.push(crate::context_lifecycle::text_message(
            ChatRole::User,
            "new observation ".repeat(256),
        ));
        lifecycle
            .prepare(
                &mut request,
                std::slice::from_ref(&requirement),
                &binding(),
                model.clone(),
                &sink,
                CancellationToken::new(),
            )
            .await
            .unwrap();

        let compactions = model.requests.lock().unwrap();
        assert_eq!(compactions.len(), 2);
        assert!(compactions[0]
            .causality
            .operation_id
            .as_ref()
            .ends_with(":compact:1"));
        assert!(compactions[1]
            .causality
            .operation_id
            .as_ref()
            .ends_with(":compact:2"));
        assert_eq!(request.input.messages.last(), Some(&requirement));
        assert!(serde_json::to_string(&request.input.messages)
            .unwrap()
            .contains("summary-two"));
    }

    #[tokio::test]
    async fn growing_tool_cycle_stops_before_over_budget_model_call() {
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![vec![
                Ok(ChatModelEvent::OutputTextDelta { text: "x".repeat(4096) }),
                Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall {
                    call_id: "read-1".into(), name: "read_file".into(),
                    arguments: nomifun_agent_contracts::StrictJsonValue(json!({"path":"README.md"})),
                    provider_metadata: None,
                }}),
                Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::ToolCalls }),
            ], text_step(&"nonshrinking summary ".repeat(390))]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        // Initial mandatory state fits. The read creates a three-message
        // exchange; the two-message ceiling now requires compaction. A bad
        // summary that grows context must not trigger a second execution call.
        let budget = AgentContextBudget { max_history_messages: 2, max_context_bytes: 64 * 1024 };
        let session = open_session_with_budget(model.clone(), Arc::new(EchoTool),
            budget);
        let mut request = request();
        request.input.max_output_tokens = Some(4096);
        let error = session.run_turn(AgentTurnRequest::new(request, tool_plan(), principal(), 0)).await.unwrap_err();
        assert!(matches!(&error, AgentEngineError::Compaction(message)
            if message.contains("compaction cannot fit")), "{error}");
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(!requests[0].input.tools.is_empty());
        assert!(requests[1].input.tools.is_empty());
        assert_eq!(requests[1].input.metadata.get("nomifun_task").map(String::as_str), Some("agent_compaction"));
        assert!(requests.iter().all(|request|
            serde_json::to_vec(&request.input).unwrap().len() <= budget.max_context_bytes));
        drop(requests);
        assert!(!session.is_turn_active().await);
    }

    #[tokio::test]
    async fn mandatory_context_over_budget_is_rejected_without_a_model_call() {
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(Vec::new()),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let session = open_session_with_budget(model.clone(), Arc::new(EchoTool),
            AgentContextBudget { max_history_messages: 4, max_context_bytes: 2048 });
        let mut oversized = request();
        oversized.input.instructions.push("x".repeat(4096));
        let error = session.run_turn(AgentTurnRequest::new(
            oversized, AgentToolPlan::default(), principal(), 0,
        )).await.unwrap_err();
        assert!(matches!(&error, AgentEngineError::Compaction(message)
            if message.contains("Mandatory instructions/task state/accepted inputs")), "{error}");
        assert!(model.requests.lock().unwrap().is_empty());
        assert!(!session.is_turn_active().await);
    }

    #[test]
    fn reasoning_signature_can_arrive_before_reasoning_text() {
        let mut step = StepState::default();
        step.set_reasoning_signature("signature".to_owned()).unwrap();
        step.append_reasoning("thinking");
        assert!(matches!(
            step.assistant_content.as_slice(),
            [ChatContentPart::Reasoning {
                text,
                signature: Some(signature),
                ..
            }] if text == "thinking" && signature == "signature"
        ));
        assert!(step.finalize().is_ok());
    }

    #[tokio::test]
    async fn tool_call_continues_into_a_second_model_step() {
        let call_id = ToolCallId::from("call-1");
        let mut steps = vec![vec![
            Ok(ChatModelEvent::ToolCallDelta {
                call_id: call_id.clone(), name: "read_file".into(),
                arguments_delta: r#"{"path":"README.md"}"#.into(),
            }),
            Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall {
                call_id: call_id.clone(), name: "read_file".into(),
                arguments: nomifun_agent_contracts::StrictJsonValue(json!({"path":"README.md"})),
                provider_metadata: None,
            }}),
            Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::ToolCalls }),
        ]];
        // One read-only batch stays on the generic Tool loop. It does not pay
        // for a task ledger or completion-account exchange.
        steps.push(text_step("done"));
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(steps), requests: std::sync::Mutex::new(Vec::new()),
        });
        let session = open_session(model.clone(), Arc::new(EchoTool));
        let result = session.run_turn(AgentTurnRequest::new(
            request(), tool_plan(), principal(), 0,
        )).await.unwrap();
        assert_eq!(result.output_text, "done");
        assert_eq!(result.model_steps, 2);
        assert_eq!(result.tool_call_count, 1);
        assert!(matches!(result.terminal, AgentTurnTerminal::Completed {
            finish_reason: ChatFinishReason::Completed
        }));
        let requests = model.requests.lock().unwrap();
        assert!(requests[1].input.messages.iter().flat_map(|message| &message.content).any(|part|
            matches!(part, ChatContentPart::ToolResult { call_id: id, is_error: false, .. } if id == &call_id)));
        assert!(model.steps.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn read_only_tools_run_in_parallel_but_results_keep_call_order() {
        let first = ToolCallId::from("call-1");
        let second = ToolCallId::from("call-2");
        let mut steps = vec![vec![
            Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall {
                call_id: first.clone(), name: "read_file".into(),
                arguments: nomifun_agent_contracts::StrictJsonValue(json!({"path":"a"})),
                provider_metadata: None,
            }}),
            Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall {
                call_id: second.clone(), name: "search_files".into(),
                arguments: nomifun_agent_contracts::StrictJsonValue(json!({"path":"b", "query":"needle"})),
                provider_metadata: None,
            }}),
            Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::ToolCalls }),
        ]];
        steps.extend(completion_steps("inspect", &["call-1", "call-2"], true));
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(steps), requests: std::sync::Mutex::new(Vec::new()),
        });
        let tools = Arc::new(ConcurrencyTool {
            active: AtomicUsize::new(0), max_active: AtomicUsize::new(0),
            order: std::sync::Mutex::new(Vec::new()), delay: Duration::from_millis(20),
        });
        let session = open_session(model.clone(), tools.clone());
        let result = session.run_turn(AgentTurnRequest::new(
            request(), two_tool_plan(AgentEffectClass::ReadOnly, true), principal(), 0,
        )).await.unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
        assert_eq!(result.tool_call_count, 4); // two reads, closed plan, completion
        assert_eq!(tools.max_active.load(Ordering::SeqCst), 2);
        assert_eq!(*tools.order.lock().unwrap(), vec!["call-2", "call-1"]);
        let requests = model.requests.lock().unwrap();
        let results = requests[1].input.messages.iter()
            .flat_map(|message| message.content.iter())
            .filter_map(|part| match part {
                ChatContentPart::ToolResult { call_id, is_error, .. } => Some((call_id.clone(), *is_error)),
                _ => None,
            }).collect::<Vec<_>>();
        assert_eq!(results, vec![(first, false), (second, false)]);
        assert!(model.steps.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn effectful_tools_run_serially() {
        let quote = "Write a and b; do not run verification.";
        let proposed = vec![
            Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall {
                call_id: "call-1".into(), name: "write_file".into(),
                arguments: nomifun_agent_contracts::StrictJsonValue(json!({"path":"a", "content":"first"})),
                provider_metadata: None,
            }}),
            Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall {
                call_id: "call-2".into(), name: "write_other_file".into(),
                arguments: nomifun_agent_contracts::StrictJsonValue(json!({"path":"b", "content":"second"})),
                provider_metadata: None,
            }}),
            Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::ToolCalls }),
        ];
        // The first effect proposal only activates reliability state and is
        // deferred before the owner port. The model then records scope and
        // retries with fresh call identities.
        let activation = vec![
            Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall {
                call_id: "activate-1".into(), name: "write_file".into(),
                arguments: nomifun_agent_contracts::StrictJsonValue(json!({"path":"a", "content":"first"})),
                provider_metadata: None,
            }}),
            Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::ToolCalls }),
        ];
        let mut steps = vec![activation, plan_step("in_progress", quote), proposed];
        steps.extend(completion_steps(quote, &[], false));
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(steps), requests: std::sync::Mutex::new(Vec::new()),
        });
        let tools = Arc::new(ConcurrencyTool {
            active: AtomicUsize::new(0), max_active: AtomicUsize::new(0),
            order: std::sync::Mutex::new(Vec::new()), delay: Duration::from_millis(10),
        });
        let session = open_session(model.clone(), tools.clone());
        let mut request = request();
        request.input.messages[0].content = vec![ChatContentPart::Text { text: quote.into() }];
        let result = session.run_turn(AgentTurnRequest::new(
            request, two_tool_plan(AgentEffectClass::ManagedEffect, false), principal(), 0,
        )).await.unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
        assert!(result.output_text.contains("not independently verified"));
        assert_eq!(tools.max_active.load(Ordering::SeqCst), 1);
        assert_eq!(*tools.order.lock().unwrap(), vec!["call-1", "call-2"]);
        let requests = model.requests.lock().unwrap();
        assert!(!requests[0]
            .input
            .tools
            .iter()
            .any(|tool| tool.name == crate::planning::TOOL_NAME));
        assert!(requests[1]
            .input
            .tools
            .iter()
            .any(|tool| tool.name == crate::planning::TOOL_NAME));
        assert!(requests[1]
            .input
            .instructions
            .iter()
            .any(|instruction| instruction.contains("Long-horizon execution policy")));
        for id in ["call-1", "call-2"] {
            assert!(requests[3].input.messages.iter().flat_map(|message| &message.content).any(|part|
                matches!(part, ChatContentPart::ToolResult { call_id, is_error: false, .. } if call_id.as_ref() == id)));
        }
        assert!(model.steps.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn plain_text_turn_completes_without_tools() {
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![vec![
                Ok(ChatModelEvent::OutputTextDelta {
                    text: "hello".to_owned(),
                }),
                Ok(ChatModelEvent::Completed {
                    finish_reason: ChatFinishReason::Completed,
                }),
            ]]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let tools = Arc::new(ConcurrencyTool {
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            order: std::sync::Mutex::new(Vec::new()),
            delay: Duration::from_millis(1),
        });
        let session = open_session(model.clone(), tools.clone());

        let result = session
            .run_turn(AgentTurnRequest::new(
                request(),
                tool_plan(),
                principal(),
                1,
            ))
            .await
            .unwrap();

        assert_eq!(result.output_text, "hello");
        assert_eq!(result.model_steps, 1);
        assert!(tools.order.lock().unwrap().is_empty(), "direct answers must not pre-read workspace instructions");
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].input.tools.iter().any(|tool| tool.name == "read_file"));
        assert!(!requests[0].input.tools.iter().any(|tool| matches!(tool.name.as_str(),
            crate::planning::TOOL_NAME | crate::completion::TOOL_NAME | crate::tool_archive::SEARCH)));
        assert!(!requests[0].input.instructions.iter().any(|instruction|
            instruction.contains("Current engine plan") || instruction.contains("Completion accounting")));
        assert!(matches!(
            result.terminal,
            AgentTurnTerminal::Completed {
                finish_reason: ChatFinishReason::Completed
            }
        ));
    }

    #[tokio::test]
    async fn accepted_steering_activates_the_long_horizon_ledger() {
        let plan = control_step("plan-steered", crate::planning::TOOL_NAME, json!({
            "explanation":"Account for both accepted inputs.",
            "plan":[{"step":"response","status":"completed"}],
            "requirements":[
                {"id":"original","description":"Inspect","source":{"input":0,"quote":"inspect"}},
                {"id":"steer","description":"Explain too","source":{"input":1,"quote":"also explain"}}
            ]
        }));
        let completion = control_step("completion-steered", crate::completion::TOOL_NAME, json!({
            "summary":"Both accepted inputs were addressed without external effects.",
            "criteria":[{"step":"response","requirement_ids":["original","steer"],
                "disposition":"unverified","evidence_call_ids":[],
                "rationale":"No external observation was required."}]
        }));
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![plan, completion, text_step("steered")]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let port = Arc::new(OneSteer {
            input: std::sync::Mutex::new(Some(crate::AgentSteeringInput {
                receipt_operation_id: "steer-receipt".into(),
                message_id: "steer-message".into(),
                text: "also explain".into(),
                files: Vec::new(),
                inject_skills: Vec::new(),
                image_count: 0,
                prepared_images: Vec::new(),
            })),
        });
        let result = open_session(model.clone(), Arc::new(EchoTool))
            .run_turn(
                AgentTurnRequest::new(
                    request(),
                    AgentToolPlan::default(),
                    principal(),
                    0,
                )
                .with_input_port(port),
            )
            .await
            .unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
        let requests = model.requests.lock().unwrap();
        assert!(requests[0]
            .input
            .tools
            .iter()
            .any(|tool| tool.name == crate::planning::TOOL_NAME));
        assert!(requests[0]
            .input
            .instructions
            .iter()
            .any(|instruction| instruction.contains("Long-horizon execution policy")));
    }

    #[tokio::test]
    async fn request_must_match_the_session_binding() {
        let model = Arc::new(ScriptedModel {
            steps: std::sync::Mutex::new(vec![vec![
                Ok(ChatModelEvent::OutputTextDelta {
                    text: "should not run".to_owned(),
                }),
                Ok(ChatModelEvent::Completed {
                    finish_reason: ChatFinishReason::Completed,
                }),
            ]]),
        });
        let session = open_session(Arc::clone(&model) as Arc<dyn AgentModelPort>, Arc::new(EchoTool));
        let mut mismatched_request = request();
        mismatched_request.causality.agent_session_id = AgentSessionId::from("other-session");

        let error = session
            .run_turn(AgentTurnRequest::new(
                mismatched_request,
                AgentToolPlan::default(),
                principal(),
                1,
            ))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            AgentEngineError::TurnBindingMismatch {
                field: "agent_session_id"
            }
        ));
        assert_eq!(model.steps.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn cancelling_a_tool_turn_returns_a_cancelled_terminal() {
        let call_id = ToolCallId::from("call-cancel");
        let model = Arc::new(ScriptedModel {
            steps: std::sync::Mutex::new(vec![vec![
                Ok(ChatModelEvent::ToolCallCompleted {
                    call: ChatToolCall {
                        call_id: call_id.clone(),
                        name: "read_file".to_owned(),
                        arguments: nomifun_agent_contracts::StrictJsonValue(json!({
                            "path": "README.md"
                        })),
                        provider_metadata: None,
                    },
                }),
                Ok(ChatModelEvent::Completed {
                    finish_reason: ChatFinishReason::ToolCalls,
                }),
            ]]),
        });
        let started = Arc::new(Notify::new());
        let session = Arc::new(open_session(
            model.clone(),
            Arc::new(BlockingTool {
                started: Arc::clone(&started),
            }),
        ));
        let task_session = Arc::clone(&session);
        let task = tokio::spawn(async move {
            task_session
                .run_turn(AgentTurnRequest::new(
                    request(),
                    tool_plan(),
                    principal(),
                    0,
                ))
                .await
        });

        tokio::time::timeout(Duration::from_secs(2), started.notified()).await.unwrap();
        assert!(model.steps.lock().unwrap().is_empty(), "must cancel the model's tool call, not an instruction read");
        assert!(session.cancel().await);
        let result = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().unwrap();

        assert!(matches!(result.terminal, AgentTurnTerminal::Cancelled));
        assert_eq!(result.model_steps, 1);
        assert!(!session.is_turn_active().await);
        assert!(!session.cancel().await);
    }

    #[tokio::test]
    async fn only_one_turn_is_admitted_at_a_time() {
        let call_id = ToolCallId::from("call-active");
        let model = Arc::new(ScriptedModel {
            steps: std::sync::Mutex::new(vec![vec![
                Ok(ChatModelEvent::ToolCallCompleted {
                    call: ChatToolCall {
                        call_id: call_id.clone(),
                        name: "read_file".to_owned(),
                        arguments: nomifun_agent_contracts::StrictJsonValue(json!({
                            "path": "README.md"
                        })),
                        provider_metadata: None,
                    },
                }),
                Ok(ChatModelEvent::Completed {
                    finish_reason: ChatFinishReason::ToolCalls,
                }),
            ]]),
        });
        let started = Arc::new(Notify::new());
        let session = Arc::new(open_session(
            model.clone(),
            Arc::new(BlockingTool {
                started: Arc::clone(&started),
            }),
        ));
        let task_session = Arc::clone(&session);
        let task = tokio::spawn(async move {
            task_session
                .run_turn(AgentTurnRequest::new(
                    request(),
                    tool_plan(),
                    principal(),
                    0,
                ))
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), started.notified()).await.unwrap();
        assert!(model.steps.lock().unwrap().is_empty(), "first turn must reach its model-proposed tool");

        let second = session
            .run_turn(AgentTurnRequest::new(
                request(),
                tool_plan(),
                principal(),
                0,
            ))
            .await;
        assert!(matches!(second, Err(AgentEngineError::TurnAlreadyRunning)));

        assert!(session.cancel().await);
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().unwrap().terminal,
            AgentTurnTerminal::Cancelled
        ));
        assert!(!session.is_turn_active().await);
    }

    #[tokio::test]
    async fn dispose_is_idempotent_and_rejects_new_turns() {
        let model = Arc::new(ScriptedModel {
            steps: std::sync::Mutex::new(vec![vec![
                Ok(ChatModelEvent::OutputTextDelta {
                    text: "should not run".to_owned(),
                }),
                Ok(ChatModelEvent::Completed {
                    finish_reason: ChatFinishReason::Completed,
                }),
            ]]),
        });
        let session = open_session(
            Arc::clone(&model) as Arc<dyn AgentModelPort>,
            Arc::new(EchoTool),
        );

        session.dispose().await;
        session.dispose().await;
        assert!(session.is_disposed().await);
        assert!(matches!(
            session
                .run_turn(AgentTurnRequest::new(
                    request(),
                    AgentToolPlan::default(),
                    principal(),
                    1,
                ))
                .await,
            Err(AgentEngineError::SessionDisposed)
        ));
        assert_eq!(model.steps.lock().unwrap().len(), 1);
    }
}
