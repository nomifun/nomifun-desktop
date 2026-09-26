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
const CODING_MAX_MODEL_STEPS: u16 = 64;
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
    /// Optional host policy. Without it the legacy explicit step cap remains
    /// a hard Turn limit; automatic renewal requires a durable event sink.
    pub segment_policy: Option<crate::AgentSegmentPolicy>,
    pub model_budget: crate::AgentModelBudget,
    pub context_resources: Arc<BTreeMap<String, crate::AgentContextResource>>,
    /// Host-admitted for this turn; false unless both vision authority and the
    /// frozen primary model route support images (Skill/workspace readers).
    /// Broker revalidates on send.
    pub context_image_input: bool,
    /// The host must set this when tool middleware can mutate beyond the
    /// declared action. Such calls cannot preserve unrelated file evidence.
    pub unscoped_tool_hooks: bool,
    pub input_port: Option<Arc<dyn crate::AgentInputPort>>,
    pub live_context_port: Option<Arc<dyn crate::AgentLiveContextPort>>,
    pub resource_port: Option<Arc<dyn nomifun_engine_core::EngineResourcePort>>,
    pub tool_discovery_port: Option<Arc<dyn crate::AgentToolDiscoveryPort>>,
    pub history_port: Option<Arc<dyn crate::AgentHistoryPort>>,
    /// Canonical latest closed task, not a checkpoint or live execution state.
    pub prior_task: Option<crate::AgentPriorTask>,
    /// Host-loaded permanent engine state, independent of the history window.
    pub patch_recovery: crate::AgentPatchRecoveryState,
    pub recovery: Option<Arc<crate::AgentTurnRecovery>>,
}

impl AgentTurnRequest {
    pub fn new(
        model_request: ChatModelRequest,
        tool_plan: AgentToolPlan,
        principal: PrincipalRef,
        active_set_generation: u64,
    ) -> Self {
        let coding_workflow = tool_plan.model_name_for_action(
            "workspace.process", "workspace.process/exec",
        ).is_some() && (
            tool_plan.model_name_for_action("workspace.files", "workspace.files/write").is_some()
                || tool_plan.model_name_for_action("workspace.files", "workspace.files/patch").is_some()
        );
        Self {
            model_request,
            tool_plan,
            principal,
            active_set_generation,
            max_model_steps: if coding_workflow { CODING_MAX_MODEL_STEPS } else { DEFAULT_MAX_MODEL_STEPS },
            segment_policy: None,
            model_budget: crate::AgentModelBudget::default(),
            context_resources: Arc::default(),
            context_image_input: false,
            unscoped_tool_hooks: false,
            input_port: None,
            live_context_port: None,
            resource_port: None,
            tool_discovery_port: None,
            history_port: None,
            prior_task: None,
            patch_recovery: Default::default(),
            recovery: None,
        }
    }

    pub fn with_max_model_steps(mut self, max_model_steps: u16) -> Self {
        self.max_model_steps = max_model_steps;
        self
    }

    pub fn with_execution_segments(mut self, policy: crate::AgentSegmentPolicy) -> Self {
        self.segment_policy = Some(policy);
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

    pub fn with_recovery(mut self, recovery: crate::AgentTurnRecovery) -> Self {
        self.recovery = Some(Arc::new(recovery));
        self.prior_task = None;
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
        if let Some(policy) = self.segment_policy { policy.validate(self.max_model_steps)?; }
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
        if let Some(recovery) = &self.recovery {
            recovery.validate_for(binding, &self.model_request.causality.turn_operation_id, self.active_set_generation)?;
            if let Some(segments) = &recovery.checkpoint.segments {
                if self.segment_policy != Some(segments.policy) || self.max_model_steps != segments.model_steps_per_segment {
                    return Err(AgentEngineError::InvalidContract("recovery cannot change its admitted execution budget".into()));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentTurnTerminal {
    Completed { finish_reason: ChatFinishReason },
    Cancelled,
    Paused { reason: String },
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
    let recovery = request.recovery.take();
    if request.segment_policy.is_some() && !event_sink.supports_checkpoints() {
        return Err(AgentEngineError::InvalidContract("execution segments require durable checkpoints".into()));
    }
    let mut segments = match recovery.as_ref().and_then(|state| state.checkpoint.segments.clone()) {
        Some(state) => Some(state),
        None => request.segment_policy.map(|policy| crate::AgentExecutionSegmentState::new(request.max_model_steps, policy)).transpose()?,
    };
    let total_model_limit = segments.as_ref().map_or(request.max_model_steps, |state| state.total_model_limit());
    let requirement = request.model_request.input.messages.last().cloned()
        .ok_or_else(|| AgentEngineError::ContextAssembly("missing accepted requirement".into()))?;
    if let Some(recovery) = &recovery {
        request.model_request.input.messages.pop();
        crate::history::replay_checkpoint_prefix(&mut request.model_request.input.messages, requirement.clone(), recovery)?;
        request.model_request.input.messages.push(crate::recovery::notice());
        request.model_request.input.provider_round_parent = None;
        let mut targets = request.patch_recovery.targets.iter().cloned().collect::<std::collections::BTreeSet<_>>();
        targets.extend(recovery.checkpoint.patch_recovery.targets.iter().cloned());
        request.patch_recovery.target_budget_exceeded |= recovery.checkpoint.patch_recovery.target_budget_exceeded || targets.len() > 64;
        request.patch_recovery.targets = targets.into_iter().take(64).collect();
    }
    request.model_budget = request.model_budget.for_request(request.model_request.input.max_output_tokens)?;
    let mut context_lifecycle = crate::context_lifecycle::ContextLifecycle::new(request.model_budget, context_budget)?;
    if let Some(recovery) = &recovery {
        context_lifecycle.restore_accounting(recovery.prefix.iter().chain(&recovery.tail))?;
    }

    // Emit the root first so instruction reads have durable turn authority.
    if let Some(recovery) = &recovery {
        event_sink.emit(AgentEngineEvent::ExecutionResumed {
            checkpoint_revision: recovery.checkpoint_revision, checkpoint_step: recovery.checkpoint.model_steps,
            model_steps: recovery.last_model_step, execution_fence: recovery.execution_fence,
            discarded_tool_call_ids: recovery.discarded_call_ids.clone(),
        }).await?;
    } else {
        event_sink.emit(AgentEngineEvent::TurnStarted {
            binding: binding.clone(), turn_operation_id: request.model_request.causality.turn_operation_id.clone(),
        }).await?;
    }
    // Establish an empty/recovered boundary before preparation can issue
    // instruction reads or compaction. A claim followed by a setup crash
    // must not depend on eventually reaching the first model request.
    if event_sink.supports_checkpoints() && !cancellation.is_cancelled() {
        let mut initial = recovery.as_ref().map(|state| state.checkpoint.clone()).unwrap_or_else(|| crate::AgentExecutionCheckpoint {
            version: 1, binding: binding.clone(), turn_operation_id: request.model_request.causality.turn_operation_id.clone(),
            active_set_generation: request.active_set_generation, model_steps: 0, tool_call_count: 0,
            accepted_input_count: 1, applied_steering_receipts: Vec::new(), plan: Default::default(), work: Default::default(),
            patch_recovery: request.patch_recovery.clone(), segments: segments.clone(), control_rejections: Default::default(),
        });
        if let Some(recovery) = &recovery { initial.model_steps = recovery.last_model_step; }
        initial.validate()?;
        event_sink.save_checkpoint(initial).await?;
    }
    event_sink.emit(AgentEngineEvent::ExecutionBudgetPrepared {
        context_window_tokens: request.model_budget.context_window_tokens,
        max_output_tokens: request.model_budget.max_output_tokens,
        max_model_steps: total_model_limit,
    }).await?;
    let mut adaptive = recovery.as_ref().map(|recovery| crate::adaptive::AdaptiveExecution::restore(&recovery.prefix, &request.tool_plan)).unwrap_or_default();
    if recovery.as_ref().is_some_and(|recovery| recovery.checkpoint.tool_call_count > 0 || recovery.checkpoint.plan.revision > 0) {
        adaptive.activate(crate::adaptive::LONG_HORIZON_MODULES, crate::AgentRuntimeActivationReason::CheckpointRecovery, event_sink.as_ref()).await?;
    }
    let mut scoped_instructions = crate::workspace_context::ScopedInstructions::new(&request);
    if let Some(recovery) = &recovery { scoped_instructions.restore_sequence(recovery.next_instruction_sequence); }
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
    model_request.input.instructions.push(request.model_budget.execution_context(total_model_limit));
    let segment_context_slot = segments.as_ref().map(|state| {
        let slot = model_request.input.instructions.len();
        model_request.input.instructions.push(state.context(recovery.as_ref().map_or(0, |state| state.last_model_step)));
        slot
    });
    let requested_tool_choice = model_request.input.tool_choice.clone();
    if let Some(prior) = &request.prior_task {
        model_request.input.instructions.push(prior.context()?);
    }
    if !request.context_resources.is_empty() {
        model_request.input.instructions.push(crate::context_resources::index(&request.context_resources, request.context_image_input)?);
    }
    let agent_session_id = model_request.causality.agent_session_id.clone();
    let turn_operation_id = model_request.causality.turn_operation_id.clone();
    let tool_archive_scope = serde_json::json!([agent_session_id, turn_operation_id, recovery.as_ref().map(|state| state.execution_fence)]).to_string();
    let mut tool_archive = adaptive
        .tool_history()
        .then(|| crate::tool_archive::ToolArchive::new(tool_archive_scope.clone()));
    let mut discovered_tools = std::collections::BTreeSet::new();

    // The host supplies canonical facts; the engine selects its model context.
    // Do this only at turn entry: trimming individual messages inside an active
    // tool cycle would break call/result pairing or discard the accepted input.
    let mut retained_inputs = vec![requirement.clone()];
    let mut steering_receipts = std::collections::BTreeSet::new();
    let mut steering_receipt_order = Vec::new();
    // Compact before the resource assembler would discard entire old turns.
    // Repository instructions are retained independently, never summarized away.
    let mut long_horizon = adaptive.task_ledger().then(LongHorizonState::default);
    if let Some(recovery) = &recovery {
        retained_inputs.extend(recovery.applied_inputs.iter().cloned());
        steering_receipt_order = recovery.checkpoint.applied_steering_receipts.iter().map(|id| id.as_ref().to_owned()).collect();
        steering_receipts.extend(steering_receipt_order.iter().cloned());
        let mut work = recovery.checkpoint.work.clone();
        work.workspace_observation_epoch = work.workspace_observation_epoch.checked_add(1)
            .ok_or_else(|| AgentEngineError::InvalidContract("recovered workspace epoch exhausted".into()))?;
        work.recent_commands.clear(); work.observed_processes.clear();
        work.command_observed_after_latest_mutation = false;
        let mut plan = recovery.checkpoint.plan.clone();
        plan.needs_replan = adaptive.task_ledger();
        long_horizon = Some(LongHorizonState { execution_plan: plan, work_status: work, ..Default::default() });
        let mut calls = BTreeMap::new();
        if let Some(archive) = tool_archive.as_mut() {
            for event in &recovery.prefix {
                match event {
                    AgentEngineEvent::ToolCallCompleted { call, step } if *step > 0 => { calls.insert(call.call_id.clone(), call); }
                    AgentEngineEvent::ToolCompleted { result, step } | AgentEngineEvent::ToolOutcomeReconciled { result, step, .. } if *step > 0 => {
                        if let Some(call) = calls.get(&result.call_id) {
                            archive.record(&call.name, &call.call_id, &call.arguments, &result.output, result.is_error, None, None)?;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
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
    if recovery.is_none() {
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
    }

    let mut output_text = String::new();
    let mut reasoning_text = String::new();
    let mut model_steps = recovery.as_ref().map_or(0, |state| state.last_model_step);
    let mut tool_call_count = recovery.as_ref().map_or(0, |state| state.checkpoint.tool_call_count);
    let mut provider_round_id = None;
    let mut completion_review_used = false;
    let mut control_rejections: ControlRejections = recovery.as_ref().map(|state| state.checkpoint.control_rejections.clone()).unwrap_or_default();
    let mut admitted_call_ids = recovery.as_ref().map(|state| state.reserved_call_ids.clone()).unwrap_or_default();
    let mut stream_budget = crate::stream_limits::StreamBudget::default();
    let mut output_limit_recovery = crate::output_limit::OutputLimitRecovery::default();
    let mut protocol_recovery = crate::protocol_recovery::ProtocolRecovery::default();
    if let Some(recovery) = &recovery {
        output_limit_recovery.restore(recovery.prefix.iter().chain(&recovery.tail));
        protocol_recovery.restore(recovery.prefix.iter().chain(&recovery.tail));
    }

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

    'model_steps: loop {
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

        if let Some(current) = segments.as_ref() {
            let pressure = event_sink.execution_pressure().await?;
            let due = model_steps >= current.window_end() || pressure.renew_window;
            let mut stop = pressure.stop;
            let mut next = None;
            if stop.is_none() && due {
                if long_horizon.as_ref().is_some_and(|state| !state.work_status.running_processes.is_empty()) {
                    stop = Some(crate::AgentExecutionStopReason::NonQuiescentBoundary);
                } else {
                    match current.renewed(model_steps) { Ok(state) => next = Some(state), Err(reason) => stop = Some(reason) }
                }
            }
            if due || stop.is_some() {
                let receipt = persist_execution_checkpoint(event_sink.as_ref(), &binding, &turn_operation_id,
                    request.active_set_generation, model_steps, tool_call_count, retained_inputs.len(),
                    &steering_receipt_order, long_horizon.as_ref(), &patch_recovery, next.as_ref().or(Some(current)), &control_rejections).await?;
                if stop.is_none() && receipt.is_none() { stop = Some(crate::AgentExecutionStopReason::CheckpointUnavailable); }
                if let Some(reason) = stop {
                    event_sink.emit(AgentEngineEvent::ExecutionBudgetExhausted {
                        model_steps, segment: current.segment, checkpoint_revision: receipt.map(|receipt| receipt.revision), reason,
                    }).await?;
                    let reason = reason.code().to_owned();
                    event_sink.emit(AgentEngineEvent::TurnPaused { model_steps, reason: reason.clone() }).await?;
                    return Ok(AgentTurnResult { agent_session_id, turn_operation_id, model_steps, output_text, reasoning_text,
                        tool_call_count, provider_round_id, terminal: AgentTurnTerminal::Paused { reason } });
                }
                let next = next.expect("window renewal has a candidate");
                event_sink.emit(AgentEngineEvent::ExecutionSegmentRenewed {
                    segment: next.segment, model_steps, checkpoint_revision: receipt.expect("renewal requires durable acknowledgement").revision,
                    reason: if pressure.renew_window { crate::AgentSegmentReason::JournalWindow } else { crate::AgentSegmentReason::ModelWindow },
                }).await?;
                context_lifecycle.begin_segment();
                stream_budget = Default::default();
                model_request.input.provider_round_parent = None;
                segments = Some(next);
            }
        }
        if model_steps >= total_model_limit {
            return fail_turn(&event_sink, model_steps, format!("model step limit of {} exceeded", total_model_limit)).await;
        }
        if let (Some(slot), Some(state)) = (segment_context_slot, segments.as_ref()) {
            model_request.input.instructions[slot] = state.context(model_steps);
        }

        // This boundary is reached only after the previous batch has settled.
        // Never compact a stream with incomplete tool calls/results.
        if let Some(port) = &request.input_port {
            let inputs = port.take(&model_request.causality, false).await?;
            if crate::steering::incorporate(inputs, &mut model_request, &mut retained_inputs, &mut steering_receipts, &mut steering_receipt_order)? {
                completion_review_used = false;
                protocol_recovery.observe_new_input();
                control_rejections.reset();
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
        protocol_recovery.constrain_tool_choice(
            &mut model_request.input.tool_choice, !model_request.input.tools.is_empty(),
        );
        protocol_recovery.constrain_exposed_tool(&mut model_request.input.tool_choice, &model_request.input.tools);
        protocol_recovery.narrow_repair_surface(&model_request.input.tool_choice, &mut model_request.input.tools);
        context_lifecycle.prepare(&mut model_request, &retained_inputs, &binding, model.clone(), event_sink.as_ref(), cancellation.clone()).await?;
        let context_bytes = serde_json::to_vec(&model_request.input)
            .map_err(|error| AgentEngineError::ContextAssembly(error.to_string()))?.len();
        if context_bytes > context_budget.max_context_bytes {
            return fail_turn(&event_sink, model_steps,
                format!("active turn context still exceeds the Nomi byte budget after compaction ({} > {})",
                    context_bytes, context_budget.max_context_bytes)).await;
        }
        // Preparation/compaction can itself fill a journal window. Re-enter
        // the same settled boundary to checkpoint/rotate before spending a
        // further model request; no model or tool step is replayed here.
        if segments.is_some() {
            let pressure = event_sink.execution_pressure().await?;
            if pressure.renew_window || pressure.stop.is_some() { continue 'model_steps; }
        }
        // The preceding batch and context updates have settled. Persist only
        // quiescent progress; live process handles are never checkpointed as
        // if they could survive a process restart.
        persist_execution_checkpoint(event_sink.as_ref(), &binding, &turn_operation_id,
            request.active_set_generation, model_steps, tool_call_count, retained_inputs.len(),
            &steering_receipt_order, long_horizon.as_ref(), &patch_recovery, segments.as_ref(), &control_rejections).await?;
        if cancellation.is_cancelled() {
            return cancelled_turn(&event_sink, &agent_session_id, &turn_operation_id,
                model_steps, &output_text, &reasoning_text, tool_call_count, provider_round_id.clone()).await;
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
                if model_steps < total_model_limit
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
        let mut public_output = crate::public_output::PublicOutputGuard::default();
        let mut saw_terminal = false;
        let mut protocol_violation = false;
        let mut protocol_tool_hint = None;
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
                    if model_steps < total_model_limit
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
                    // Some compatible providers spill a truncated internal
                    // tool representation into content before emitting length.
                    // Withhold it, but drain the bounded stream to distinguish
                    // output exhaustion from a completed protocol violation.
                    if text.is_empty() {
                        return fail_turn(
                            &event_sink,
                            model_steps,
                            "model emitted an empty output text delta",
                        )
                        .await;
                    }
                    if protocol_violation { continue; }
                    let visible = public_output.push(&text);
                    if !visible.text.is_empty() {
                        output_text.push_str(&visible.text);
                        step.append_text(&visible.text);
                        event_sink.emit(AgentEngineEvent::OutputTextDelta {
                            step: model_steps, text: visible.text,
                        }).await?;
                    }
                    if visible.invalid_tool_call {
                        protocol_violation = true;
                        protocol_tool_hint = visible.tool_hint;
                    }
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
                    if public_output.has_pending_native_call() {
                        protocol_violation = true;
                    }
                    let remaining = public_output.finish();
                    if !protocol_violation && !remaining.is_empty() {
                        output_text.push_str(&remaining);
                        step.append_text(&remaining);
                        event_sink.emit(AgentEngineEvent::OutputTextDelta {
                            step: model_steps, text: remaining,
                        }).await?;
                    }
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

        if protocol_violation && step.finish_reason != Some(ChatFinishReason::MaxOutputTokens) {
            drop(stream);
            if cancellation.is_cancelled() {
                return cancelled_turn(&event_sink, &agent_session_id, &turn_operation_id,
                    model_steps, &output_text, &reasoning_text, tool_call_count, provider_round_id.clone()).await;
            }
            let discarded_tool_call_ids = step.call_order.clone();
            crate::output_limit::validate_discarded(model_steps, &discarded_tool_call_ids)?;
            // Never force a later text proposal ahead of a discarded native
            // prefix. Recheck the name against this exact advertised surface.
            let tool_hint = if discarded_tool_call_ids.is_empty() {
                protocol_tool_hint.filter(|name| model_request.input.tools.iter().any(|tool| &tool.name == name))
            } else { None };
            let continuation = protocol_recovery.admit(model_steps < total_model_limit);
            protocol_recovery.set_tool_hint(tool_hint.clone());
            event_sink.emit(AgentEngineEvent::ModelResponseRejected {
                step: model_steps, discarded_tool_call_ids: discarded_tool_call_ids.clone(), continuation, tool_hint,
            }).await?;
            admitted_call_ids.extend(discarded_tool_call_ids);
            crate::output_limit::retain_partial_text(&mut model_request, &step.assistant_content);
            model_request.input.messages.push(crate::protocol_recovery::notice(continuation));
            provider_round_id = None;
            if !continuation {
                return fail_turn(&event_sink, model_steps,
                    "model emitted tool-call markup as text after the bounded protocol-correction budget; no tool from the rejected responses was executed").await;
            }
            continue 'model_steps;
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
            let continuation = output_limit_recovery.admit(model_steps < total_model_limit);
            event_sink.emit(AgentEngineEvent::ModelOutputTruncated {
                step: model_steps, discarded_tool_call_ids: discarded_tool_call_ids.clone(), continuation,
            }).await?;
            let effectful_workspace_proposal = protocol_tool_hint.as_deref().into_iter()
                .chain(step.calls.values().map(|pending| pending.name.as_str()))
                .filter_map(|name| request.tool_plan.binding(name))
                .any(|binding| crate::execution_policy::requires_task_ledger(binding)
                    && !matches!(binding.effect_class, AgentEffectClass::ReadOnly));
            if continuation && effectful_workspace_proposal {
                adaptive.activate(crate::adaptive::LEDGER_MODULES,
                    crate::AgentRuntimeActivationReason::OutputLimitRecovery,event_sink.as_ref()).await?;
                long_horizon.get_or_insert_with(LongHorizonState::default);
            }
            // Size recovery may require a different file/patch strategy. A
            // prior protocol hint must not force the oversized function again.
            protocol_recovery.release_constraint();
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
        output_limit_recovery.observe_complete_step();
        step.finalize()?;
        protocol_recovery.observe_valid_step();
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
                crate::steering::incorporate(inputs, &mut model_request, &mut retained_inputs, &mut steering_receipts, &mut steering_receipt_order)?;
                control_rejections.reset();
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

            let mut long_horizon_calls = 0usize;
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
                    if crate::execution_policy::requires_task_ledger(tool) {
                        long_horizon_calls = long_horizon_calls.saturating_add(1);
                    }
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
            let multi_step = adaptive.observe_external_batch(long_horizon_calls);
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
            let plan_revision_before = state.execution_plan.revision;
            let had_current_report = state.completion.current(
                &state.execution_plan, &state.work_status, retained_inputs.len(),
            ).is_some();
            let results = match invoke_tool_calls(
                &agent_session_id,
                &request.principal,
                &model_request,
                &request.active_set_generation,
                &request.tool_plan,
                &mut state.tool_arguments,
                &dispatch,
                event_sink.as_ref(),
                &step,
                model_steps,
                &cancellation,
                &mut state.execution_plan,
                adaptive.task_ledger(),
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
            let single_call_batch = step.call_order.len() == 1;
            let mut terminal_collaboration_accepted = false;
            let mut terminal_completion_requested = false;
            let mut repeated_control_rejection = None;
            for (expected_call_id, result) in results {
                result.validate_for(&expected_call_id)?;
                if let Some(call) = step.calls.get(&expected_call_id).and_then(|pending| pending.completed.as_ref()) {
                    terminal_completion_requested |= call.name == crate::completion::TOOL_NAME && !result.is_error;
                    let made_progress = match call.name.as_str() {
                        crate::planning::TOOL_NAME => state.execution_plan.revision != plan_revision_before,
                        crate::completion::TOOL_NAME => !had_current_report,
                        _ => true,
                    };
                    if let Some(reason) = control_rejections.observe(&call.name, &result, made_progress) {
                        repeated_control_rejection = Some(reason);
                    }
                }
                if let Some(call) = step.calls.get(&expected_call_id).and_then(|pending| pending.completed.as_ref()) {
                    if let Some(binding) = request.tool_plan.binding(&call.name) {
                        let attempted = dispatch.attempted(&expected_call_id)?;
                        let failed_process = attempted
                            && crate::execution_policy::failed_process_observation(binding, &result);
                        terminal_collaboration_accepted |= single_call_batch
                            && attempted
                            && !result.is_error
                            && crate::execution_policy::completes_turn_on_success(binding);
                        if attempted {
                            if let Some(segments) = segments.as_mut() { segments.observe(call, &result)?; }
                            state.work_status.observe(
                                binding,
                                call,
                                &result,
                                &mut state.command_tracker,
                            );
                            if request.unscoped_tool_hooks {
                                state.work_status.before_resource_request();
                            }
                        } else {
                            if !result.is_error {
                                return Err(AgentEngineError::InvalidContract("unattempted platform tool returned success".into()));
                            }
                            state.work_status.observe_deferred();
                        }
                        if failed_process {
                            adaptive.activate(
                                crate::adaptive::LONG_HORIZON_MODULES,
                                crate::AgentRuntimeActivationReason::EffectfulToolCall,
                                event_sink.as_ref(),
                            ).await?;
                            // The first failed command activates a plan. Once
                            // a source-anchored plan is in progress, an
                            // expected failing test remains an observation
                            // within that plan, not an automatic demand to
                            // replan before the next repair. Later serial
                            // effects in this same batch were already held.
                            if state.execution_plan.revision == 0 {
                                state.execution_plan.needs_replan = true;
                            }
                            state.completion.invalidate();
                        }
                        if attempted && binding.action_id.as_ref() == "workspace.files/read" && state.work_status.running_processes.is_empty() {
                            patch_recovery.observe_read(call, &result);
                        }
                        if (attempted && (request.unscoped_tool_hooks || (crate::execution_policy::affects_workspace(binding)
                            && !crate::execution_policy::process_did_not_start(binding, &result))))
                            || !state.work_status.running_processes.is_empty()
                        {
                            // Failed calls may have partial effects too.
                            scoped_instructions.invalidate();
                        }
                        let observation = state.completion.observe_with_effect_scope(
                            &state.work_status,
                            binding,
                            call,
                            &result,
                            attempted,
                            !request.unscoped_tool_hooks,
                        );
                        if adaptive.task_ledger() {
                            event_sink.emit(AgentEngineEvent::CompletionObservation { observation }).await?;
                        }
                        // New observations require a fresh completion account;
                        // the model-step bound limits repeated work/review.
                        completion_review_used = false;
                        if crate::execution_policy::requires_replanning_after_result(
                            binding, &result, attempted, state.execution_plan.revision,
                        ) || (request.unscoped_tool_hooks && attempted && result.is_error) {
                            state.execution_plan.needs_replan = true;
                            // Expose the recovery control before asking the
                            // model for another expensive proposed effect.
                            if crate::execution_policy::requires_task_ledger(binding) {
                                adaptive.activate(crate::adaptive::LEDGER_MODULES,
                                    crate::AgentRuntimeActivationReason::EffectfulToolCall,
                                    event_sink.as_ref()).await?;
                            }
                        }
                    } else if matches!(call.name.as_str(), crate::planning::TOOL_NAME | crate::task_continuation::TOOL_NAME) {
                        // Idempotent/rejected proposals are not state changes.
                        // Only a committed plan transition invalidates the
                        // current report; do not create a plan/report loop.
                        if state.execution_plan.revision != plan_revision_before
                            && state.completion.current(&state.execution_plan, &state.work_status, retained_inputs.len()).is_none() {
                            state.completion.invalidate();
                            completion_review_used = false;
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
            let terminal_report = terminal_completion_requested.then(|| state.completion.current(
                &state.execution_plan,&state.work_status,retained_inputs.len()).cloned()).flatten();
            let terminal_handoff = terminal_collaboration_accepted
                && (!adaptive.task_ledger()
                    || (state.execution_plan.revision == 0
                        && !state.execution_plan.needs_replan
                        && state.work_status.successful_workspace_mutations == 0
                        && state.work_status.successful_commands == 0
                        && retained_inputs.len() == 1));
            if (terminal_handoff || terminal_report.is_some())
                && state.work_status.running_processes.is_empty()
                && !patch_recovery.pending()
            {
                // `take(..., true)` is the terminal fence: when it returns
                // empty, the host atomically closes steering for this turn.
                // A separate `has_pending` check would leave a race in which
                // a newly accepted steer is acknowledged after completion and
                // then never incorporated.
                let terminal_inputs = match &request.input_port {
                    Some(port) => port.take(&model_request.causality, true).await?,
                    None => Vec::new(),
                };
                if terminal_inputs.is_empty() {
                    if cancellation.is_cancelled() {
                        return cancelled_turn(&event_sink,&agent_session_id,&turn_operation_id,
                            model_steps,&output_text,&reasoning_text,tool_call_count,provider_round_id.clone()).await;
                    }
                    if let Some(report) = terminal_report {
                        let mut delivery = if output_text.is_empty() { report.summary.clone() }
                            else { format!("\n\n{}",report.summary) };
                        if let Some(disclosure)=report.unverified_disclosure() { delivery.push_str(&disclosure); }
                        output_text.push_str(&delivery);
                        event_sink.emit(AgentEngineEvent::CompletionDelivered { step:model_steps,text:delivery }).await?;
                        if report.is_blocked() {
                            return fail_turn(&event_sink,model_steps,"completion account contains blocked work; this turn cannot be published as task completion").await;
                        }
                    }
                    let finish_reason = ChatFinishReason::Completed;
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
                crate::steering::incorporate(
                    terminal_inputs,
                    &mut model_request,
                    &mut retained_inputs,
                    &mut steering_receipts,
                    &mut steering_receipt_order,
                )?;
                control_rejections.reset();
                completion_review_used = false;
                adaptive
                    .activate(
                        crate::adaptive::LEDGER_MODULES,
                        crate::AgentRuntimeActivationReason::Steering,
                        event_sink.as_ref(),
                    )
                    .await?;
                state.execution_plan.needs_replan = true;
                state.completion.invalidate();
            }
            if let Some(reason) = repeated_control_rejection {
                return fail_turn(&event_sink, model_steps,
                    format!("engine control made no progress; {reason}")).await;
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
                && model_steps < total_model_limit
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
            if crate::steering::incorporate(inputs, &mut model_request, &mut retained_inputs, &mut steering_receipts, &mut steering_receipt_order)? {
                completion_review_used = false;
                control_rejections.reset();
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

}

#[allow(clippy::too_many_arguments)]
async fn persist_execution_checkpoint(
    sink: &dyn AgentEventSink, binding: &EngineBinding, operation: &OperationId,
    generation: u64, model_steps: u16, tool_call_count: u32, input_count: usize,
    steering_receipts: &[String], state: Option<&LongHorizonState>,
    patch_recovery: &crate::patch_recovery::PatchRecovery,
    segments: Option<&crate::AgentExecutionSegmentState>,
    control_rejections: &AgentControlRejectionState,
) -> Result<Option<crate::AgentCheckpointReceipt>, AgentEngineError> {
    if !sink.supports_checkpoints() { return Ok(None); }
    let work = state.map(|state| state.work_status.clone()).unwrap_or_default();
    if !work.running_processes.is_empty() { return Ok(None); }
    let checkpoint = crate::AgentExecutionCheckpoint {
        version: 1, binding: binding.clone(), turn_operation_id: operation.clone(),
        active_set_generation: generation, model_steps, tool_call_count, accepted_input_count: input_count,
        applied_steering_receipts: steering_receipts.iter().map(|id| OperationId::from(id.clone())).collect(),
        plan: state.map(|state| state.execution_plan.clone()).unwrap_or_default(),
        work, patch_recovery: patch_recovery.snapshot(), segments: segments.cloned(),
        control_rejections: control_rejections.clone(),
    };
    checkpoint.validate()?;
    sink.save_checkpoint(checkpoint).await
}

#[derive(Default)]
struct LongHorizonState {
    tool_arguments: crate::tool_validation::ToolArgumentValidators,
    execution_plan: crate::AgentPlan,
    work_status: crate::AgentWorkStatus,
    completion: crate::completion::CompletionTracker,
    command_tracker: crate::workflow::CommandTracker,
}

/// A weak tool-calling model can keep resubmitting malformed plan/completion
/// controls. Bound that retry loop before it consumes the context window; the
/// fourth failure still has its durable result before the turn stops.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentControlRejectionState {
    name: String,
    consecutive: u8,
    total: u8,
    no_progress: u8,
}

type ControlRejections = AgentControlRejectionState;

impl AgentControlRejectionState {
    pub(crate) fn validate(&self) -> Result<(), AgentEngineError> {
        if !matches!(self.name.as_str(), "" | crate::planning::TOOL_NAME | crate::completion::TOOL_NAME)
            || self.consecutive > 4 || self.total > 8 || self.no_progress > 4 {
            return Err(AgentEngineError::InvalidContract("invalid persisted control correction budget".into()));
        }
        Ok(())
    }

    pub(crate) fn reset(&mut self) {
        self.name.clear();
        self.consecutive = 0;
        self.total = 0;
        self.no_progress = 0;
    }

    pub(crate) fn observe(&mut self, name: &str, result: &AgentToolResult, made_progress: bool) -> Option<String> {
        if result.is_error && matches!(name, crate::planning::TOOL_NAME | crate::completion::TOOL_NAME) {
            self.consecutive = if self.name == name { self.consecutive.saturating_add(1) } else { 1 };
            self.total = self.total.saturating_add(1);
            self.name = name.to_owned();
            if self.consecutive >= 4 {
                return Some(format!("{name} failed four consecutive times"));
            }
            if self.total >= 8 {
                return Some("update_plan/report_completion failed eight times without a successful intervening tool; stop the control loop and surface the last rejection".into());
            }
            return None;
        }
        if result.is_error {
            self.name.clear();
            self.consecutive = 0;
            return None;
        }
        if !made_progress && matches!(name, crate::planning::TOOL_NAME | crate::completion::TOOL_NAME) {
            self.name.clear();
            self.consecutive = 0;
            self.no_progress = self.no_progress.saturating_add(1);
            if self.no_progress >= 4 {
                return Some("plan/completion controls succeeded idempotently four times without changing task state; no progress was made".into());
            }
            return None;
        }
        self.reset();
        None
    }
}

#[derive(Default)]
struct AdaptiveContextSlots {
    long_horizon_policy: Option<usize>,
    scoped_instructions: Option<usize>,
    patch_recovery: Option<usize>,
    tool_history: Option<usize>,
    task_plan: Option<usize>,
    completion: Option<usize>,
    discovery_catalog: Option<usize>,
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
    if tool_discovery {
        if let Some(catalog) = crate::tool_discovery::catalog(plan, discovered_tools) {
            upsert_instruction(&mut request.input.instructions, &mut slots.discovery_catalog, catalog);
        } else if let Some(slot) = slots.discovery_catalog {
            request.input.instructions[slot] = "All previously deferred tool schemas are now visible.".into();
        }
    }
    if adaptive.task_ledger() {
        let state = long_horizon.ok_or_else(|| {
            AgentEngineError::InvalidContract("active task ledger has no turn-local state".into())
        })?;
        // Temporary workflow gates do not revoke tools from the frozen
        // capability surface. Removing schemas made repairable command errors
        // look like lost shell permission and forced needless tool discovery.
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
    argument_validators: &mut crate::tool_validation::ToolArgumentValidators,
    invoker: &dyn AgentToolInvoker,
    event_sink: &dyn AgentEventSink,
    step: &StepState,
    model_step: u16,
    cancellation: &CancellationToken,
    execution_plan: &mut crate::AgentPlan,
    task_ledger_active: bool,
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
        completion.invalidate();
        for call in &completed {
            if plan.binding(&call.name).is_none() {
                work_status.observe_deferred();
            }
        }
        return finish_tool_results(results, event_sink, model_step, cancellation).await;
    }
    if let Some(results) = argument_validators.reject_invalid_batch(&completed, plan, &model_request.input.tools)? {
        // This precedes instruction discovery, Kernel admission and controls.
        // All calls receive paired, non-executed results; no successful prefix
        // can be accidentally repeated when the model repairs the batch.
        if cancellation.is_cancelled() { return Err(AgentEngineError::Cancelled); }
        if completed.iter().any(|call| call.name == crate::completion::TOOL_NAME) {
            completion.invalidate();
        }
        return finish_tool_results(results, event_sink, model_step, cancellation).await;
    }
    if completed
        .iter()
        .any(|call| call.name == crate::tool_discovery::TOOL_NAME)
    {
        let discovery_only = completed.iter().all(|call| call.name == crate::tool_discovery::TOOL_NAME);
        let mut results = Vec::with_capacity(completed.len());
        for call in &completed {
            let result = if !discovery_only {
                AgentToolResult::text(
                    call.call_id.clone(),
                    "No tools executed: batch ToolSearch calls only with other ToolSearch calls; use the discovered schemas in a later execution batch.",
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
    if completed.len() == 2 && completed[0].name == crate::planning::TOOL_NAME
        && completed[1].name == crate::completion::TOOL_NAME {
        let planned = execution_plan.update(&completed[0], accepted_inputs, event_sink).await?;
        let report = if planned.is_error {
            AgentToolResult::text(completed[1].call_id.clone(), "Completion was not applied because the preceding plan update failed. The prior plan and effects remain unchanged.", true)
        } else if patch_recovery.pending() {
            completion.invalidate();
            AgentToolResult::text(completed[1].call_id.clone(),"Completion is not ready: re-observe failed patch targets before finalizing.",true)
        } else {
            completion.submit(&completed[1], execution_plan, work_status, accepted_inputs, event_sink).await?
        };
        return finish_tool_results(vec![
            (completed[0].call_id.clone(), Ok(planned)),
            (completed[1].call_id.clone(), Ok(report)),
        ], event_sink, model_step, cancellation).await;
    }
    if completed.iter().any(|call| call.name == crate::completion::TOOL_NAME) {
        let mut results = Vec::new();
        for call in &completed {
            let result = if patch_recovery.pending() {
                completion.invalidate();
                AgentToolResult::text(call.call_id.clone(),"Completion is not ready: re-observe failed patch targets before finalizing.",true)
            } else if completed.len() == 1 {
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
            else if task_ledger_active
                && !cleanup
                && crate::execution_policy::requires_task_ledger(binding)
                && !matches!(binding.effect_class, AgentEffectClass::ReadOnly)
            {
                execution_plan.effect_gate()
            }
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
        if patch_call.is_none()
            && crate::execution_policy::affects_workspace(&invocation.binding)
        {
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
        if recorded.1.is_error || plan.binding(&observed_call.name).is_some_and(|binding|
            crate::execution_policy::failed_process_observation(binding, &recorded.1)
        ) {
            if let Some(call) = patch_call { patch_recovery.failed(&call); }
            defer_remaining = Some("Not executed: an earlier serial call or command failed. Inspect its result and correct the call before proposing remaining effects again. Replan if scope changed or an attempted effect has an uncertain outcome; read/argument errors alone do not require a new plan.".into());
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
    fn coding_workflow_gets_a_larger_bounded_model_step_budget() {
        let coding = AgentToolPlan::new([
            tool_binding("write_file", "workspace.files", "workspace.files/write", AgentEffectClass::ManagedEffect, false),
            tool_binding("exec_command", "workspace.process", "workspace.process/exec", AgentEffectClass::ExternalUncertainEffect, false),
        ]).unwrap();
        assert_eq!(AgentTurnRequest::new(request(), coding, principal(), 0).max_model_steps,
            CODING_MAX_MODEL_STEPS);
        assert_eq!(AgentTurnRequest::new(request(), tool_plan(), principal(), 0).max_model_steps,
            DEFAULT_MAX_MODEL_STEPS);
    }

    #[test]
    fn alternating_control_rejections_are_bounded_without_effect_replay() {
        let mut rejections = ControlRejections::default();
        let failed = AgentToolResult::text("control".into(), "rejected", true);
        for index in 0..7 {
            let name = if index % 2 == 0 { crate::planning::TOOL_NAME } else { crate::completion::TOOL_NAME };
            assert!(rejections.observe(name, &failed, false).is_none());
            if index == 3 {
                assert!(rejections.observe("exec_command", &failed, false).is_none());
                assert_eq!(rejections.total, 4);
            }
        }
        assert!(rejections.observe(crate::completion::TOOL_NAME, &failed, false)
            .is_some_and(|reason| reason.contains("eight times")));
        let accepted = AgentToolResult::text("control".into(), "accepted", false);
        assert!(rejections.observe(crate::planning::TOOL_NAME, &accepted, true).is_none());
        assert_eq!(rejections.total, 0);
    }

    fn atomic_media_plan() -> AgentToolPlan {
        AgentToolPlan::new([tool_binding(
            "generate_image",
            "creation.media",
            "creation.media/image",
            AgentEffectClass::ExternalUncertainEffect,
            false,
        )])
        .unwrap()
    }

    fn collaboration_plan(include_fork: bool) -> AgentToolPlan {
        let delegate = tool_binding(
            "delegate",
            "agent.collaboration",
            "agent/delegate",
            AgentEffectClass::ManagedEffect,
            false,
        );
        if include_fork {
            AgentToolPlan::new([
                delegate,
                tool_binding(
                    "fork",
                    "agent.collaboration",
                    "agent/fork",
                    AgentEffectClass::ManagedEffect,
                    false,
                ),
            ])
            .unwrap()
        } else {
            AgentToolPlan::new([delegate]).unwrap()
        }
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

    struct CountingInstructionTool {
        instruction_reads: AtomicUsize,
    }

    #[async_trait]
    impl AgentToolInvoker for CountingInstructionTool {
        async fn invoke(
            &self,
            invocation: AgentToolInvocation,
            _cancellation: CancellationToken,
        ) -> Result<AgentToolResult, AgentEngineError> {
            if invocation.call.arguments.0.get("missing_ok") == Some(&json!(true)) {
                self.instruction_reads.fetch_add(1, Ordering::SeqCst);
            }
            Ok(instruction_result(&invocation).unwrap_or_else(|| workspace_result(invocation)))
        }
    }

    struct FailedProcessTool;

    struct ProcessThenWriteTool {
        writes: AtomicUsize,
    }

    #[async_trait]
    impl AgentToolInvoker for ProcessThenWriteTool {
        async fn invoke(
            &self,
            invocation: AgentToolInvocation,
            _cancellation: CancellationToken,
        ) -> Result<AgentToolResult, AgentEngineError> {
            if let Some(result) = instruction_result(&invocation) {
                return Ok(result);
            }
            if invocation.binding.action_id.as_ref() == "workspace.files/write" {
                self.writes.fetch_add(1, Ordering::SeqCst);
                return Ok(workspace_result(invocation));
            }
            assert_eq!(invocation.binding.action_id.as_ref(), "workspace.process/exec");
            Ok(AgentToolResult::text(invocation.call.call_id,
                json!({"process_id":"failed-process","state":"exited",
                    "exit_code":1,"cleanup":{"reaped":true},"success":false}).to_string(), true))
        }
    }

    #[async_trait]
    impl AgentToolInvoker for FailedProcessTool {
        async fn invoke(
            &self,
            invocation: AgentToolInvocation,
            _cancellation: CancellationToken,
        ) -> Result<AgentToolResult, AgentEngineError> {
            if let Some(result) = instruction_result(&invocation) {
                return Ok(result);
            }
            assert_eq!(invocation.binding.action_id.as_ref(), "workspace.process/exec");
            Ok(AgentToolResult::text(
                invocation.call.call_id,
                json!({"process_id":"failed-process","state":"exited",
                    "exit_code":1,"cleanup":{"reaped":true},"success":false}).to_string(),
                false,
            ))
        }
    }

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
            assert!(matches!(path, "." | "README.md" | "a" | "b"));
            json!({"path":path, "canonical_path":if path == "." { "" } else { path },
                "kind":if path == "." { "directory" } else { "file" },
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
            "creation.media/image" => json!({
                "creation_task_id":"0190f5fe-7c00-7a00-8000-000000000001",
                "status":"queued",
                "result_asset_ids":[]
            }),
            "agent/delegate" | "agent/fork" => json!({
                "execution_id":"0190f5fe-7c00-7a00-8000-000000000002",
                "status":"planning"
            }),
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
                "summary":"done",
                "criteria":[{"step":"requested operations", "requirement_ids":["request"],
                    "disposition":if supported { "supported" } else { "unverified" },
                    "evidence_call_ids":evidence,
                    "rationale":if supported { "The requested reads returned successfully." }
                        else { "Writes returned, but their resulting contents were not independently verified." }}]
            })),
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

    struct FailingCollaborationTool;

    #[async_trait]
    impl AgentToolInvoker for FailingCollaborationTool {
        async fn invoke(
            &self,
            invocation: AgentToolInvocation,
            _cancellation: CancellationToken,
        ) -> Result<AgentToolResult, AgentEngineError> {
            Ok(AgentToolResult::text(
                invocation.call.call_id,
                "delegation rejected",
                true,
            ))
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

    #[derive(Debug)]
    struct TerminalFenceSteer {
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
    impl crate::AgentInputPort for TerminalFenceSteer {
        async fn take(
            &self,
            _causality: &ChatCausality,
            close_if_empty: bool,
        ) -> Result<Vec<crate::AgentSteeringInput>, AgentEngineError> {
            if close_if_empty {
                Ok(self.input.lock().unwrap().take().into_iter().collect())
            } else {
                Ok(Vec::new())
            }
        }

        async fn has_pending(
            &self,
            _causality: &ChatCausality,
        ) -> Result<bool, AgentEngineError> {
            // Simulate a steer accepted after the dispatch gate but before
            // the terminal `take(..., true)` fence.
            Ok(false)
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
        assert!(requests[0].input.instructions[0]
            .contains("at most 1125 UTF-8 bytes"),
            "the summary target scales with this route's frozen output budget");
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
    async fn compaction_summarizes_large_older_prefix_once_and_keeps_headroom() {
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![text_step("Earlier work summarized; current file receipt remains in the retained exchange.")]),
            requests: Default::default(),
        });
        let mut request = request();
        request.input.max_output_tokens = Some(4096);
        request.input.instructions = vec!["Stable workspace instructions. ".repeat(650)];
        let original = request.input.messages[0].clone();
        let fresh_code = "RECENT_FILE_BYTES".repeat(300);
        let call = ChatContentPart::ToolCall {
            call_id: "recent-file".into(), name: "write_file".into(),
            arguments: nomifun_agent_contracts::StrictJsonValue(json!({"path":"gomoku.html","content":fresh_code})),
            provider_metadata: None,
        };
        request.input.messages.extend([
            crate::context_lifecycle::text_message(ChatRole::Assistant, "OLDER_HISTORY ".repeat(3300)),
            ChatMessage { role: ChatRole::Assistant, content: vec![call.clone()], provider_round_id: None },
            ChatMessage { role: ChatRole::Tool, content: vec![ChatContentPart::ToolResult {
                call_id: "recent-file".into(), is_error: false,
                output: vec![nomifun_chat_model_broker::ChatToolResultPart::Text { text: "written:true".into() }],
            }], provider_round_id: None },
        ]);
        let before = serde_json::to_vec(&request.input).unwrap().len();
        let mut lifecycle = crate::context_lifecycle::ContextLifecycle::new(
            crate::AgentModelBudget::default(), AgentContextBudget { max_context_bytes: 256 * 1024, max_history_messages: 256 },
        ).unwrap();
        lifecycle.prepare(&mut request, &[original.clone()], &binding(), model.clone(), &NoopAgentEventSink,
            CancellationToken::new()).await.unwrap();
        let after = serde_json::to_vec(&request.input).unwrap().len();
        assert!(after < before * 3 / 4, "compaction must leave useful headroom: {before} -> {after}");
        assert!(request.input.messages.iter().any(|message| message == &original));
        assert!(request.input.messages.iter().flat_map(|message| &message.content).any(|part| part == &call));
        let captured = model.requests.lock().unwrap();
        assert_eq!(captured.len(), 1, "a 4k output envelope must not force tiny serial summary fragments");
        let source = serde_json::to_string(&captured[0].input.messages).unwrap();
        assert!(source.contains("OLDER_HISTORY"));
        assert!(!source.contains("RECENT_FILE_BYTES"), "do not summarize the suffix retained verbatim");
        drop(captured);
        request.input.messages.push(crate::context_lifecycle::text_message(ChatRole::Assistant, "small follow-up".repeat(100)));
        lifecycle.prepare(&mut request, &[original], &binding(), model.clone(), &NoopAgentEventSink,
            CancellationToken::new()).await.unwrap();
        assert_eq!(model.requests.lock().unwrap().len(), 1, "no repeat compaction after a small observation");
    }

    #[tokio::test]
    async fn compaction_output_limit_retries_once_without_replaying_tools() {
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![
                vec![
                    Ok(ChatModelEvent::OutputTextDelta { text: "partial summary".into() }),
                    Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::MaxOutputTokens }),
                ],
                text_step("Concise retained context."),
                text_step("done"),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let budget = AgentContextBudget { max_history_messages: 4, max_context_bytes: 64 * 1024 };
        let session = open_session_with_budget(model.clone(), Arc::new(EchoTool), budget);
        let mut request = request();
        let current = request.input.messages.pop().unwrap();
        request.input.messages = (0..6).map(|index| ChatMessage {
            role: ChatRole::User,
            content: vec![ChatContentPart::Text { text: format!("old context {index}") }],
            provider_round_id: None,
        }).chain([current]).collect();
        let result = session.run_turn(AgentTurnRequest::new(
            request, AgentToolPlan::default(), principal(), 0,
        )).await.unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
        assert_eq!(result.tool_call_count, 0);
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[0].causality.operation_id.as_ref().ends_with(":compact:1"));
        assert!(requests[1].causality.operation_id.as_ref().ends_with(":compact:2"));
        assert!(requests[..2].iter().all(|request| request.input.tools.is_empty()
            && request.input.metadata.get("nomifun_task").map(String::as_str) == Some("agent_compaction")));
        assert!(requests[1].input.instructions[0].contains("at most 192 UTF-8 bytes"));
        assert!(requests[2].input.messages.iter().any(|message| format!("{message:?}").contains("Concise retained context.")));
    }

    #[tokio::test]
    async fn compaction_byte_overrun_retries_once_with_a_smaller_summary() {
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![
                text_step(&"x".repeat(9000)),
                text_step(&"S".repeat(260)),
                text_step("done"),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let budget = AgentContextBudget { max_history_messages: 4, max_context_bytes: 64 * 1024 };
        let session = open_session_with_budget(model.clone(), Arc::new(EchoTool), budget);
        let mut request = request();
        let current = request.input.messages.pop().unwrap();
        request.input.messages = (0..6).map(|index| ChatMessage {
            role: ChatRole::User,
            content: vec![ChatContentPart::Text { text: format!("old context {index} ").repeat(20) }],
            provider_round_id: None,
        }).chain([current]).collect();
        let result = session.run_turn(AgentTurnRequest::new(
            request, AgentToolPlan::default(), principal(), 0,
        )).await.unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[0].causality.operation_id.as_ref().ends_with(":compact:1"));
        assert!(requests[1].causality.operation_id.as_ref().ends_with(":compact:2"));
        assert!(requests[1].input.instructions[0].contains("at most 192 UTF-8 bytes"));
        assert!(requests[2].input.messages.iter().any(|message|
            format!("{message:?}").contains(&"S".repeat(260))));
    }

    #[tokio::test]
    async fn repeatedly_truncated_compaction_splits_source_without_losing_the_task() {
        let truncated = vec![
            Ok(ChatModelEvent::OutputTextDelta { text: "partial".into() }),
            Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::MaxOutputTokens }),
        ];
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![
                truncated.clone(), truncated,
                text_step("First half retained."),
                text_step("Both halves retained."),
                text_step("done"),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let budget = AgentContextBudget { max_history_messages: 4, max_context_bytes: 64 * 1024 };
        let session = open_session_with_budget(model.clone(), Arc::new(EchoTool), budget);
        let mut request = request();
        let current = request.input.messages.pop().unwrap();
        request.input.messages = (0..6).map(|index| ChatMessage {
            role: ChatRole::User,
            content: vec![ChatContentPart::Text { text: format!("old context {index} ").repeat(20) }],
            provider_round_id: None,
        }).chain([current.clone()]).collect();
        let result = session.run_turn(AgentTurnRequest::new(
            request, AgentToolPlan::default(), principal(), 0,
        )).await.unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
        assert_eq!(result.tool_call_count, 0);
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 5);
        for (index, compact) in requests.iter().take(4).enumerate() {
            assert!(compact.causality.operation_id.as_ref().ends_with(&format!(":compact:{}", index + 1)));
            assert!(compact.input.tools.is_empty());
        }
        assert_eq!(requests[4].input.messages.last(), Some(&current));
        assert!(format!("{:?}", requests[4].input.messages).contains("Both halves retained."));
    }

    #[tokio::test]
    async fn exhausted_compaction_output_marks_missing_history_and_keeps_accepted_input() {
        let truncated = vec![
            Ok(ChatModelEvent::OutputTextDelta { text: "partial".into() }),
            Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::MaxOutputTokens }),
        ];
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new((0..64).map(|_| truncated.clone()).collect()),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let budget = AgentContextBudget { max_history_messages: 4, max_context_bytes: 64 * 1024 };
        let mut request = request();
        request.input.max_output_tokens = Some(4096);
        let current = request.input.messages.pop().unwrap();
        request.input.messages = (0..6).map(|index| ChatMessage {
            role: ChatRole::User,
            content: vec![ChatContentPart::Text { text: format!("old context {index} ").repeat(80) }],
            provider_round_id: None,
        }).chain([current.clone()]).collect();
        let mut lifecycle = crate::context_lifecycle::ContextLifecycle::new(
            crate::AgentModelBudget::default(), budget,
        ).unwrap();
        lifecycle.prepare(&mut request, std::slice::from_ref(&current), &binding(),
            model.clone(), &NoopAgentEventSink, CancellationToken::new()).await.unwrap();
        assert_eq!(request.input.messages.last(), Some(&current));
        let summary = format!("{:?}", request.input.messages[0]);
        assert!(summary.contains("Automatic summary incomplete"));
        assert!(summary.contains("Re-read relevant files and rerun checks"));
        assert!(!summary.contains("partial"), "a truncated model draft must not be committed");
        let requests = model.requests.lock().unwrap();
        assert!(!requests.is_empty() && requests.len() <= 16);
        assert!(requests.iter().all(|request| request.input.tools.is_empty()));
    }

    #[tokio::test]
    async fn one_turn_can_compact_repeatedly_without_losing_the_accepted_requirement() {
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new((0..16).map(|_| text_step("retained summary")).collect()),
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
        assert!(compactions.len() >= 2);
        for (index, compact) in compactions.iter().enumerate() {
            assert!(compact.causality.operation_id.as_ref()
                .ends_with(&format!(":compact:{}", index + 1)));
        }
        assert_eq!(request.input.messages.last(), Some(&requirement));
        assert!(serde_json::to_string(&request.input.messages)
            .unwrap()
            .contains("retained summary"));
    }

    #[tokio::test]
    async fn oversized_compaction_summary_degrades_explicitly_without_committing_its_draft() {
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new((0..64).map(|_| text_step(&"nonshrinking summary ".repeat(500))).collect()),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let budget = AgentContextBudget { max_history_messages: 4, max_context_bytes: 64 * 1024 };
        let mut request = request();
        request.input.max_output_tokens = Some(4096);
        let current = request.input.messages.pop().unwrap();
        request.input.messages = (0..6).map(|index| ChatMessage {
            role: ChatRole::User,
            content: vec![ChatContentPart::Text { text: format!("old context {index} ").repeat(80) }],
            provider_round_id: None,
        }).chain([current.clone()]).collect();
        let mut lifecycle = crate::context_lifecycle::ContextLifecycle::new(
            crate::AgentModelBudget::default(), budget,
        ).unwrap();
        lifecycle.prepare(&mut request, std::slice::from_ref(&current), &binding(),
            model.clone(), &NoopAgentEventSink, CancellationToken::new()).await.unwrap();
        assert_eq!(request.input.messages.last(), Some(&current));
        let summary = format!("{:?}", request.input.messages[0]);
        assert!(summary.contains("Automatic summary incomplete"));
        assert!(!summary.contains("nonshrinking summary"));
        let requests = model.requests.lock().unwrap();
        assert!(!requests.is_empty() && requests.len() <= 16);
        for (index, compact) in requests.iter().enumerate() {
            assert!(compact.input.tools.is_empty());
            assert_eq!(compact.input.metadata.get("nomifun_task").map(String::as_str), Some("agent_compaction"));
            assert!(compact.causality.operation_id.as_ref().ends_with(&format!(":compact:{}", index + 1)));
        }
        assert!(requests.iter().all(|request|
            serde_json::to_vec(&request.input).unwrap().len() <= budget.max_context_bytes));
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
    async fn repeated_read_only_calls_reuse_instructions_until_an_effect_invalidates_them() {
        let request = AgentTurnRequest::new(request(), tool_plan(), principal(), 0);
        let mut instructions = crate::workspace_context::ScopedInstructions::new(&request);
        let tool = CountingInstructionTool { instruction_reads: AtomicUsize::new(0) };
        let sink = NoopAgentEventSink;
        let call = ChatToolCall {
            call_id: "read-repeat".into(),
            name: "read_file".into(),
            arguments: nomifun_agent_contracts::StrictJsonValue(json!({"path":"README.md"})),
            provider_metadata: None,
        };
        instructions.before_calls(std::slice::from_ref(&call), &tool, &sink,
            CancellationToken::new()).await.unwrap();
        let loaded = tool.instruction_reads.load(Ordering::SeqCst);
        assert!(loaded > 0, "the first read must load applicable instructions");
        instructions.before_calls(std::slice::from_ref(&call), &tool, &sink,
            CancellationToken::new()).await.unwrap();
        assert_eq!(tool.instruction_reads.load(Ordering::SeqCst), loaded,
            "a repeated read-only target must not reload unchanged ancestors");
        instructions.invalidate();
        instructions.before_model(&tool, &sink, CancellationToken::new()).await.unwrap();
        assert!(tool.instruction_reads.load(Ordering::SeqCst) > loaded,
            "effects must force a fresh instruction observation");
    }

    #[tokio::test]
    async fn failed_process_exit_activates_replanning_before_a_terminal_reply() {
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![
                control_step("failed-exec", "exec_command", json!({"command":"bun","args":["test"]})),
                text_step("All done"),
                text_step("Still done"),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let plan = AgentToolPlan::new([
            tool_binding("read_file", "workspace.files", "workspace.files/read", AgentEffectClass::ReadOnly, true),
            tool_binding("exec_command", "workspace.process", "workspace.process/exec", AgentEffectClass::ExternalUncertainEffect, false),
        ]).unwrap();
        let result = open_session(model.clone(), Arc::new(FailedProcessTool))
            .run_turn(AgentTurnRequest::new(request(), plan, principal(), 0))
            .await;
        assert!(matches!(result, Err(AgentEngineError::TurnFailed(_))));
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 3, "the first attempted final reply must receive a completion review");
        assert!(requests[1].input.tools.iter().any(|tool| tool.name == crate::planning::TOOL_NAME));
        assert!(requests[1].input.tools.iter().any(|tool| tool.name == crate::completion::TOOL_NAME));
    }

    #[tokio::test]
    async fn failed_test_command_does_not_block_the_next_repair_in_an_active_plan() {
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![
                control_step("failed-first", "exec_command", json!({"command":"bun","args":["test"]})),
                plan_step("in_progress", "inspect"),
                control_step("failed-second", "exec_command", json!({"command":"bun","args":["test"]})),
                control_step("repair", "write_file", json!({"path":"README.md","content":"fixed"})),
                text_step("done"),
                text_step("still done"),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let plan = AgentToolPlan::new([
            tool_binding("read_file", "workspace.files", "workspace.files/read", AgentEffectClass::ReadOnly, true),
            tool_binding("exec_command", "workspace.process", "workspace.process/exec", AgentEffectClass::ExternalUncertainEffect, false),
            tool_binding("write_file", "workspace.files", "workspace.files/write", AgentEffectClass::ManagedEffect, false),
        ]).unwrap();
        let tools = Arc::new(ProcessThenWriteTool { writes: AtomicUsize::new(0) });
        let result = open_session(model.clone(), tools.clone())
            .run_turn(AgentTurnRequest::new(request(), plan, principal(), 0)).await;
        assert!(matches!(result, Err(AgentEngineError::TurnFailed(_))));
        assert_eq!(tools.writes.load(Ordering::SeqCst), 1,
            "the failed baseline test must not fence a later repair in the recorded plan");
        assert!(model.requests.lock().unwrap()[4].input.messages.iter()
            .flat_map(|message| &message.content)
            .any(|part| matches!(part, ChatContentPart::ToolResult { call_id, is_error: false, .. }
                if call_id.as_ref() == "repair")));
    }

    #[tokio::test]
    async fn successful_single_collaboration_handoff_completes_without_another_model_step() {
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![
                control_step(
                    "delegate-1",
                    "delegate",
                    json!({"strategy":"parallel","tasks":[{"name":"a","prompt":"A"}]}),
                ),
                text_step("must not be generated after the handoff"),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let result = open_session(model.clone(), Arc::new(EchoTool))
            .run_turn(AgentTurnRequest::new(
                request(),
                collaboration_plan(false),
                principal(),
                0,
            ))
            .await
            .unwrap();

        assert_eq!(result.model_steps, 1);
        assert_eq!(result.tool_call_count, 1);
        assert_eq!(result.output_text, "");
        assert!(matches!(
            result.terminal,
            AgentTurnTerminal::Completed {
                finish_reason: ChatFinishReason::Completed
            }
        ));
        assert_eq!(model.requests.lock().unwrap().len(), 1);
        assert_eq!(model.steps.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn accepted_steer_is_incorporated_before_collaboration_handoff_closes_the_turn() {
        let plan = control_step(
            "plan-after-delegate",
            crate::planning::TOOL_NAME,
            json!({
                "explanation":"Account for the accepted correction.",
                "plan":[{"step":"response","status":"completed"}],
                "requirements":[
                    {"id":"original","description":"Inspect","source":{"input":0,"quote":"inspect"}},
                    {"id":"steer","description":"Explain too","source":{"input":1,"quote":"also explain"}}
                ]
            }),
        );
        let completion = control_step(
            "completion-after-delegate",
            crate::completion::TOOL_NAME,
            json!({
                "summary":"The accepted correction was incorporated.",
                "criteria":[{"step":"response","requirement_ids":["original","steer"],
                    "disposition":"unverified","evidence_call_ids":[],
                    "rationale":"No further external observation was required."}]
            }),
        );
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![
                control_step(
                    "delegate-before-steer",
                    "delegate",
                    json!({"strategy":"planned","goal":"work"}),
                ),
                plan,
                completion,
                text_step("steered after handoff"),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let port = Arc::new(TerminalFenceSteer {
            input: std::sync::Mutex::new(Some(crate::AgentSteeringInput {
                receipt_operation_id: "steer-receipt-after-delegate".into(),
                message_id: "steer-message-after-delegate".into(),
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
                    collaboration_plan(false),
                    principal(),
                    0,
                )
                .with_input_port(port),
            )
            .await
            .unwrap();

        assert_eq!(result.model_steps, 3);
        assert!(result.output_text.contains("The accepted correction was incorporated."));
        assert_eq!(model.requests.lock().unwrap().len(), 3);
        assert_eq!(model.steps.lock().unwrap().len(), 1, "no further model request after the accepted report");
    }

    #[tokio::test]
    async fn collaboration_handoff_cannot_bypass_an_active_completion_ledger() {
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![
                control_step(
                    "delegate-with-ledger",
                    "delegate",
                    json!({"strategy":"planned","goal":"work"}),
                ),
                control_step(
                    "plan-with-ledger",
                    crate::planning::TOOL_NAME,
                    json!({
                        "explanation":"Account for both accepted inputs.",
                        "plan":[{"step":"response","status":"completed"}],
                        "requirements":[
                            {"id":"original","description":"Inspect","source":{"input":0,"quote":"inspect"}},
                            {"id":"steer","description":"Explain too","source":{"input":1,"quote":"also explain"}}
                        ]
                    }),
                ),
                control_step(
                    "completion-with-ledger",
                    crate::completion::TOOL_NAME,
                    json!({
                        "summary":"Both accepted inputs were accounted for.",
                        "criteria":[{"step":"response","requirement_ids":["original","steer"],
                            "disposition":"unverified","evidence_call_ids":[],
                            "rationale":"The delegated result remains owned by AgentExecution."}]
                    }),
                ),
                text_step("ledger closed"),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let port = Arc::new(OneSteer {
            input: std::sync::Mutex::new(Some(crate::AgentSteeringInput {
                receipt_operation_id: "steer-receipt-before-delegate".into(),
                message_id: "steer-message-before-delegate".into(),
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
                    collaboration_plan(false),
                    principal(),
                    0,
                )
                .with_input_port(port),
            )
            .await
            .unwrap();

        assert_eq!(result.model_steps, 3);
        assert!(result.output_text.contains("Both accepted inputs were accounted for."));
        assert_eq!(model.requests.lock().unwrap().len(), 3);
        assert_eq!(model.steps.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn failed_or_mixed_collaboration_batches_do_not_force_turn_completion() {
        let failed = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![
                control_step(
                    "delegate-failed",
                    "delegate",
                    json!({"strategy":"planned","goal":"work"}),
                ),
                text_step("recovered from rejection"),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let failed_result = open_session(failed.clone(), Arc::new(FailingCollaborationTool))
            .run_turn(AgentTurnRequest::new(
                request(),
                collaboration_plan(false),
                principal(),
                0,
            ))
            .await
            .unwrap();
        assert_eq!(failed_result.model_steps, 2);
        assert_eq!(failed_result.output_text, "recovered from rejection");
        assert_eq!(failed.requests.lock().unwrap().len(), 2);

        let mixed_step = vec![
            Ok(ChatModelEvent::ToolCallCompleted {
                call: ChatToolCall {
                    call_id: "delegate-mixed".into(),
                    name: "delegate".into(),
                    arguments: nomifun_agent_contracts::StrictJsonValue(
                        json!({"strategy":"planned","goal":"work"}),
                    ),
                    provider_metadata: None,
                },
            }),
            Ok(ChatModelEvent::ToolCallCompleted {
                call: ChatToolCall {
                    call_id: "fork-mixed".into(),
                    name: "fork".into(),
                    arguments: nomifun_agent_contracts::StrictJsonValue(
                        json!({"goal":"other work"}),
                    ),
                    provider_metadata: None,
                },
            }),
            Ok(ChatModelEvent::Completed {
                finish_reason: ChatFinishReason::ToolCalls,
            }),
        ];
        let mixed = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![mixed_step, text_step("mixed batch reviewed")]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let mixed_result = open_session(mixed.clone(), Arc::new(EchoTool))
            .run_turn(AgentTurnRequest::new(
                request(),
                collaboration_plan(true),
                principal(),
                0,
            ))
            .await
            .unwrap();
        assert_eq!(mixed_result.model_steps, 2);
        assert_eq!(mixed_result.output_text, "mixed batch reviewed");
        assert_eq!(mixed_result.tool_call_count, 2);
        assert_eq!(mixed.requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn atomic_media_submission_does_not_activate_coding_ledger() {
        let call_id = ToolCallId::from("create-image");
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![
                vec![
                    Ok(ChatModelEvent::ToolCallCompleted {
                        call: ChatToolCall {
                            call_id: call_id.clone(),
                            name: "generate_image".into(),
                            arguments: nomifun_agent_contracts::StrictJsonValue(json!({
                                "prompt":"a cat"
                            })),
                            provider_metadata: None,
                        },
                    }),
                    Ok(ChatModelEvent::Completed {
                        finish_reason: ChatFinishReason::ToolCalls,
                    }),
                ],
                text_step("submitted"),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let result = open_session(model.clone(), Arc::new(EchoTool))
            .run_turn(AgentTurnRequest::new(
                request(),
                atomic_media_plan(),
                principal(),
                0,
            ))
            .await
            .unwrap();

        assert_eq!(result.output_text, "submitted");
        assert_eq!(result.model_steps, 2);
        assert_eq!(result.tool_call_count, 1);
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        for request in requests.iter() {
            assert!(!request.input.tools.iter().any(|tool| {
                matches!(
                    tool.name.as_str(),
                    crate::planning::TOOL_NAME | crate::completion::TOOL_NAME
                )
            }));
            assert!(!request
                .input
                .instructions
                .iter()
                .any(|instruction| instruction.contains("Long-horizon execution policy")));
        }
        assert!(requests[1]
            .input
            .messages
            .iter()
            .flat_map(|message| &message.content)
            .any(|part| matches!(part,
                ChatContentPart::ToolResult { call_id: id, is_error: false, .. }
                    if id == &call_id)));
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
        // Automatic ledger activation does not reject the first valid batch
        // or force generation of the same file payload a second time.
        let mut steps = vec![proposed];
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
            assert!(requests[1].input.messages.iter().flat_map(|message| &message.content).any(|part|
                matches!(part, ChatContentPart::ToolResult { call_id, is_error: false, .. } if call_id.as_ref() == id)));
        }
        assert!(model.steps.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn discovery_batches_reveal_schemas_but_mixed_batches_have_no_effects() {
        #[derive(Debug)]
        struct ExactDiscovery;
        #[async_trait]
        impl crate::AgentToolDiscoveryPort for ExactDiscovery {
            async fn select(&self, _: &ChatCausality, _: u64, query: &str,
                candidates: &[crate::AgentToolDiscoveryCandidate], _: usize, _: CancellationToken,
            ) -> Result<Vec<String>, AgentEngineError> {
                Ok(candidates.iter().filter(|item| item.name == query).map(|item| item.name.clone()).collect())
            }
        }
        let mut first = tool_binding("browser_observe", "browser", "browser/observe", AgentEffectClass::ReadOnly, true);
        first.definition.deferred = true;
        let mut second = tool_binding("web_search", "web.research", "web.research/search", AgentEffectClass::ReadOnly, true);
        second.definition.deferred = true;
        let plan = AgentToolPlan::new([first, second,
            tool_binding("read_file", "workspace.files", "workspace.files/read", AgentEffectClass::ReadOnly, true)]).unwrap();
        let mut mixed = control_step("mixed-search", "ToolSearch", json!({"query":"browser_observe"}));
        mixed.pop();
        mixed.extend(control_step("mixed-read", "read_file", json!({"path":"a"})));
        let mut searches = control_step("search-browser", "ToolSearch", json!({"query":"browser_observe"}));
        searches.pop();
        searches.extend(control_step("search-web", "ToolSearch", json!({"query":"web_search"})));
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![mixed, searches, text_step("Schemas ready")]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let tools = Arc::new(ConcurrencyTool { active: AtomicUsize::new(0), max_active: AtomicUsize::new(0),
            order: std::sync::Mutex::new(Vec::new()), delay: Duration::ZERO });
        let result = open_session(model.clone(), tools.clone()).run_turn(
            AgentTurnRequest::new(request(), plan, principal(), 0).with_tool_discovery_port(Arc::new(ExactDiscovery)),
        ).await.unwrap();
        assert_eq!(result.model_steps, 3);
        assert!(tools.order.lock().unwrap().is_empty());
        let requests = model.requests.lock().unwrap();
        for name in ["browser_observe", "web_search"] {
            assert!(!requests[1].input.tools.iter().any(|tool| tool.name == name), "mixed batch cannot reveal schemas");
            assert!(requests[2].input.tools.iter().any(|tool| tool.name == name));
        }
        for id in ["search-browser", "search-web"] {
            assert!(requests[2].input.messages.iter().flat_map(|message| &message.content).any(|part|
                matches!(part, ChatContentPart::ToolResult { call_id, is_error: false, .. } if call_id.as_ref() == id)));
        }
    }

    #[tokio::test]
    async fn inspection_then_handoff_does_not_require_a_second_completion_owner() {
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![
                control_step("inspect-1", "read_file", json!({"path":"a"})),
                control_step("inspect-2", "read_file", json!({"path":"b"})),
                control_step("delegate", "delegate", json!({"strategy":"planned","goal":"work"})),
            ]), requests: std::sync::Mutex::new(Vec::new()),
        });
        let plan = AgentToolPlan::new([
            tool_binding("read_file", "workspace.files", "workspace.files/read", AgentEffectClass::ReadOnly, true),
            tool_binding("delegate", "agent.collaboration", "agent/delegate", AgentEffectClass::ManagedEffect, false),
        ]).unwrap();
        let result = open_session(model.clone(), Arc::new(EchoTool)).run_turn(
            AgentTurnRequest::new(request(), plan, principal(), 0),
        ).await.unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
        assert_eq!(result.model_steps, 3);
        assert!(model.requests.lock().unwrap()[2].input.tools.iter().any(|tool| tool.name == "update_plan"));
    }

    #[tokio::test]
    async fn ordered_plan_and_completion_batch_records_both_without_invalidating_the_report() {
        #[derive(Default)]
        struct Sink(std::sync::Mutex<Vec<AgentEngineEvent>>);
        #[async_trait]
        impl AgentEventSink for Sink {
            async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError> {
                self.0.lock().unwrap().push(event); Ok(())
            }
        }
        let mut controls = control_step("close-plan", "update_plan", json!({
            "plan":[{"step":"Original internal planning label","status":"completed"}]
        }));
        controls.pop();
        controls.extend(control_step("account", "report_completion", json!({"summary":"Inspected file",
            "criteria":[{"step":"Delivered source inspection","disposition":"supported","evidence_paths":["b"],"rationale":"Read the requested file"}]
        })));
        let model = Arc::new(ObservingModel { steps:std::sync::Mutex::new(vec![
            control_step("read-a","read_file",json!({"path":"a"})),
            control_step("read-b","read_file",json!({"path":"b"})),controls,text_step("Inspection delivered"),
        ]),requests:std::sync::Mutex::new(Vec::new()) });
        let sink = Arc::new(Sink::default());
        let result = run_turn(binding(), model.clone(), Arc::new(EchoTool), sink.clone(),
            AgentTurnRequest::new(request(),tool_plan(),principal(),0),
            AgentContextBudget::default(), CancellationToken::new(),
        ).await.unwrap();
        assert_eq!(result.model_steps,3);
        assert_eq!(result.output_text,"Inspected file");
        assert_eq!(model.requests.lock().unwrap().len(),3);
        assert_eq!(model.steps.lock().unwrap().len(),1);
        assert!(matches!(result.terminal,AgentTurnTerminal::Completed{..}));
        let events = sink.0.lock().unwrap();
        for expected in ["close-plan","account"] {
            assert!(events.iter().any(|event| matches!(event, AgentEngineEvent::ToolCompleted { result, .. }
                if result.call_id.as_ref()==expected && !result.is_error)));
        }
    }

    #[tokio::test]
    async fn output_usage_is_not_charged_again_when_calibrating_the_next_input() {
        let mut first = control_step("read-once","read_file",json!({"path":"a"}));
        first.insert(first.len()-1,Ok(ChatModelEvent::Usage { usage:nomifun_chat_model_broker::ChatUsage {
            input_tokens:17_500, output_tokens:4_096, ..Default::default()
        }}));
        let model = Arc::new(ObservingModel { steps:std::sync::Mutex::new(vec![first,text_step("Read complete")]),
            requests:std::sync::Mutex::new(Vec::new()) });
        let mut input = request(); input.input.max_output_tokens=Some(4096);
        let result = open_session(model.clone(),Arc::new(EchoTool)).run_turn(
            AgentTurnRequest::new(input,tool_plan(),principal(),0),
        ).await.unwrap();
        assert_eq!(result.model_steps,2);
        let requests=model.requests.lock().unwrap();
        assert_eq!(requests.len(),2,"completed output must not cause an unnecessary paid compaction request");
        assert!(requests.iter().all(|request| !request.input.tools.is_empty()),"no summary request was needed");
    }

    #[tokio::test]
    async fn provider_textual_tool_spill_with_length_uses_size_recovery_without_executing_or_replaying_it() {
        let mut oversized = text_step("<tool_call><function=read_file><parameter=path>PRIVATE_DISCARDED_ARGUMENTS");
        *oversized.last_mut().unwrap()=Ok(ChatModelEvent::Completed { finish_reason:ChatFinishReason::MaxOutputTokens });
        let model=Arc::new(ObservingModel { steps:std::sync::Mutex::new(vec![oversized,
            control_step("small-native","read_file",json!({"path":"a"})),text_step("Read complete")]),requests:Default::default() });
        let result=open_session(model.clone(),Arc::new(EchoTool)).run_turn(
            AgentTurnRequest::new(request(),tool_plan(),principal(),0),
        ).await.unwrap();
        assert_eq!(result.tool_call_count,1,"text is never executed");
        let requests=model.requests.lock().unwrap();
        let repaired=serde_json::to_string(&requests[1].input).unwrap();
        assert!(repaired.contains("oversized payload"));
        assert!(!repaired.contains("Engine protocol observation"),"length is not a completed malformed-tool response");
        assert!(!repaired.contains("PRIVATE_DISCARDED_ARGUMENTS"));
        assert_eq!(requests[1].input.tool_choice,ChatToolChoice::Auto,"do not force the same oversized tool again");
    }

    #[tokio::test]
    async fn a_small_scaffold_after_truncation_cannot_skip_the_task_completion_account() {
        let mut oversized=text_step("<tool_call><function=write_file><parameter=content>discarded");
        *oversized.last_mut().unwrap()=Ok(ChatModelEvent::Completed { finish_reason:ChatFinishReason::MaxOutputTokens });
        let model=Arc::new(ObservingModel { steps:std::sync::Mutex::new(vec![oversized,
            control_step("scaffold","write_file",json!({"path":"a","content":"initial structure"})),
            text_step("Done"),text_step("Done")]),requests:Default::default() });
        let plan=AgentToolPlan::new([tool_binding("write_file","workspace.files","workspace.files/write",AgentEffectClass::ManagedEffect,false)]).unwrap();
        let error=open_session(model.clone(),Arc::new(EchoTool)).run_turn(
            AgentTurnRequest::new(request(),plan,principal(),0),
        ).await.unwrap_err();
        assert!(matches!(&error,AgentEngineError::TurnFailed(message) if message.contains("completion account")),"{error:?}");
        assert!(model.requests.lock().unwrap()[1].input.tools.iter().any(|tool|tool.name=="report_completion"));
    }

    #[tokio::test]
    async fn single_workspace_write_completes_without_an_unavailable_plan_tool() {
        let call_id = ToolCallId::from("single-write");
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![
                control_step("single-write", "write_file", json!({
                    "path":"a", "content":"<html><canvas></canvas></html>"
                })),
                text_step("Saved the file."),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let tools = Arc::new(ConcurrencyTool {
            active: AtomicUsize::new(0), max_active: AtomicUsize::new(0),
            order: std::sync::Mutex::new(Vec::new()), delay: Duration::from_millis(1),
        });
        let result = open_session(model.clone(), tools.clone())
            .run_turn(AgentTurnRequest::new(
                request(), two_tool_plan(AgentEffectClass::ManagedEffect, false), principal(), 0,
            ))
            .await
            .unwrap();

        assert_eq!(result.output_text, "Saved the file.");
        assert_eq!(result.model_steps, 2);
        assert_eq!(*tools.order.lock().unwrap(), vec!["single-write"]);
        let requests = model.requests.lock().unwrap();
        assert!(requests[1].input.messages.iter().flat_map(|message| &message.content).any(|part|
            matches!(part, ChatContentPart::ToolResult { call_id: id, is_error: false, .. } if id == &call_id)));
        assert!(requests.iter().all(|request| !request.input.tools.iter().any(|tool|
            matches!(tool.name.as_str(), crate::planning::TOOL_NAME | crate::completion::TOOL_NAME))));
    }

    #[tokio::test]
    async fn accepted_completion_stops_before_the_model_can_reopen_the_task() {
        for invalid in [false, true] {
            let mut steps = vec![
                control_step("inspect-one", "read_file", json!({"path":"a"})),
                control_step("inspect-two", "read_file", json!({"path":"b"})),
            ];
            let mut closing = completion_steps("inspect", &["inspect-two"], true);
            steps.extend(closing.drain(..2));
            let mut update = json!({
                "explanation":"A repeated status update", "plan":[{"step":"requested operations","status":"completed"}]
            });
            if invalid {
                update["plan"] = json!([
                    {"step":"one","status":"in_progress"},
                    {"step":"two","status":"in_progress"},
                ]);
            }
            steps.push(control_step("repeated-plan", crate::planning::TOOL_NAME, update));
            steps.push(text_step("done"));
            let model = Arc::new(ObservingModel {
                steps: std::sync::Mutex::new(steps), requests: std::sync::Mutex::new(Vec::new()),
            });
            let result = open_session(model.clone(), Arc::new(EchoTool)).run_turn(
                AgentTurnRequest::new(request(), tool_plan(), principal(), 0),
            ).await.unwrap();
            assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
            assert_eq!(result.model_steps, 4);
            assert_eq!(result.output_text, "done");
            assert_eq!(model.requests.lock().unwrap().len(), 4);
            assert_eq!(model.steps.lock().unwrap().len(), 2);

        }
    }

    #[tokio::test]
    async fn accepted_steer_at_completion_fence_requires_a_new_account() {
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![
                control_step("read-a", "read_file", json!({"path":"a"})),
                control_step("read-b", "read_file", json!({"path":"b"})),
                control_step("old-account", "report_completion", json!({"summary":"Old delivery",
                    "criteria":[{"disposition":"supported","evidence_paths":["b"],"rationale":"Read observed"}]})),
                control_step("replan", "update_plan", json!({"plan":[{"step":"Include explanation","status":"completed"}]})),
                control_step("new-account", "report_completion", json!({"summary":"New delivery with explanation",
                    "criteria":[{"disposition":"unverified","rationale":"Explanation is not an external observation"}]})),
                text_step("must never request this"),
            ]), requests: Default::default(),
        });
        let port = Arc::new(TerminalFenceSteer { input: std::sync::Mutex::new(Some(crate::AgentSteeringInput {
            receipt_operation_id: "completion-steer".into(), message_id: "completion-steer-message".into(),
            text: "also explain".into(), files: vec![], inject_skills: vec![], image_count: 0, prepared_images: vec![],
        })) });
        let result = open_session(model.clone(), Arc::new(EchoTool)).run_turn(
            AgentTurnRequest::new(request(), tool_plan(), principal(), 0).with_input_port(port),
        ).await.unwrap();
        assert_eq!(result.model_steps, 5);
        assert!(result.output_text.contains("New delivery with explanation"));
        assert!(!result.output_text.contains("Old delivery"));
        assert_eq!(model.steps.lock().unwrap().len(), 1);
        assert!(model.requests.lock().unwrap()[3].input.messages.iter().any(|message|
            message.role == ChatRole::User && message.content.iter().any(|part|
                matches!(part, ChatContentPart::Text { text } if text == "also explain"))));
    }

    #[tokio::test]
    async fn a_blocked_completion_account_cannot_publish_success() {
        let model = Arc::new(ObservingModel { steps: std::sync::Mutex::new(vec![
            control_step("read-a", "read_file", json!({"path":"a"})),
            control_step("read-b", "read_file", json!({"path":"b"})),
            control_step("blocked", "report_completion", json!({"summary":"Cannot deliver",
                "criteria":[{"disposition":"blocked","rationale":"Required dependency is unavailable"}]})),
            text_step("must never publish success"),
        ]), requests: Default::default() });
        let result = open_session(model.clone(), Arc::new(EchoTool)).run_turn(
            AgentTurnRequest::new(request(), tool_plan(), principal(), 0),
        ).await;
        assert!(result.is_err());
        assert_eq!(model.requests.lock().unwrap().len(), 3);
        assert_eq!(model.steps.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn unchanged_plan_success_is_not_counted_as_task_progress_forever() {
        let mut steps = vec![
            control_step("inspect-one", "read_file", json!({"path":"a"})),
            control_step("inspect-two", "read_file", json!({"path":"b"})),
            plan_step("in_progress", "inspect"),
        ];
        for index in 0..4 {
            steps.push(control_step(&format!("noop-{index}"), crate::planning::TOOL_NAME, json!({
                "explanation":"No change", "plan":[{"step":"requested operations","status":"in_progress"}]
            })));
        }
        steps.push(text_step("must not be requested"));
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(steps), requests: std::sync::Mutex::new(Vec::new()),
        });
        let error = open_session(model.clone(), Arc::new(EchoTool)).run_turn(
            AgentTurnRequest::new(request(), tool_plan(), principal(), 0),
        ).await.unwrap_err();
        assert!(matches!(error, AgentEngineError::TurnFailed(message) if message.contains("succeeded idempotently")));
        assert_eq!(model.requests.lock().unwrap().len(), 7);
        assert_eq!(model.steps.lock().unwrap().len(), 1);
    }

    struct RecoverableReadTool { writes: AtomicUsize }

    #[async_trait]
    impl AgentToolInvoker for RecoverableReadTool {
        async fn invoke(&self, invocation: AgentToolInvocation, _: CancellationToken) -> Result<AgentToolResult, AgentEngineError> {
            if invocation.call.call_id.as_ref() == "recoverable-read" {
                return Ok(AgentToolResult::text(invocation.call.call_id, "The optional file was not found", true));
            }
            if invocation.binding.action_id.as_ref() == "workspace.files/write" {
                self.writes.fetch_add(1, Ordering::SeqCst);
            }
            Ok(instruction_result(&invocation).unwrap_or_else(|| workspace_result(invocation)))
        }
    }

    #[tokio::test]
    async fn a_failed_read_can_be_corrected_inside_the_existing_plan() {
        let mut steps = vec![
            control_step("inspect-one", "read_file", json!({"path":"a"})),
            control_step("inspect-two", "read_file", json!({"path":"b"})),
            plan_step("in_progress", "inspect"),
            control_step("recoverable-read", "read_file", json!({"path":"a"})),
            control_step("write", "write_file", json!({"path":"a", "content":"fixed"})),
            control_step("verify", "read_file", json!({"path":"a"})),
        ];
        steps.extend(completion_steps("inspect", &["verify"], true));
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(steps), requests: std::sync::Mutex::new(Vec::new()),
        });
        let tools = Arc::new(RecoverableReadTool { writes: AtomicUsize::new(0) });
        let result = open_session(model.clone(), tools.clone()).run_turn(
            AgentTurnRequest::new(request(), two_tool_plan(AgentEffectClass::ManagedEffect, false), principal(), 0),
        ).await.unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
        assert_eq!(tools.writes.load(Ordering::SeqCst), 1);
        assert_eq!(result.model_steps, 8);
    }

    #[tokio::test]
    async fn invalid_arguments_hold_the_whole_effect_batch_until_the_model_repairs_it() {
        let batch = |first: &str, second: &str, second_path: serde_json::Value| vec![
            Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall {
                call_id: first.into(), name: "write_file".into(), provider_metadata: None,
                arguments: nomifun_agent_contracts::StrictJsonValue(json!({"path":"a","content":"fixed"})),
            }}),
            Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall {
                call_id: second.into(), name: "write_file".into(), provider_metadata: None,
                arguments: nomifun_agent_contracts::StrictJsonValue(json!({"path":second_path,"content":"fixed"})),
            }}),
            Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::ToolCalls }),
        ];
        let mut steps = vec![
            control_step("inspect-one", "read_file", json!({"path":"a"})),
            control_step("inspect-two", "read_file", json!({"path":"b"})),
            plan_step("in_progress", "inspect"),
            batch("held-valid", "held-invalid", json!(42)),
            batch("corrected-one", "corrected-two", json!("b")),
            control_step("verify", "read_file", json!({"path":"a"})),
        ];
        steps.extend(completion_steps("inspect", &["verify"], true));
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(steps), requests: std::sync::Mutex::new(Vec::new()),
        });
        let tools = Arc::new(RecoverableReadTool { writes: AtomicUsize::new(0) });
        let result = open_session(model.clone(), tools.clone()).run_turn(
            AgentTurnRequest::new(request(), two_tool_plan(AgentEffectClass::ManagedEffect, false), principal(), 0),
        ).await.unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
        assert_eq!(tools.writes.load(Ordering::SeqCst), 2, "the valid prefix of the rejected batch must not execute");
        let requests = model.requests.lock().unwrap();
        for id in ["held-valid", "held-invalid"] {
            assert!(requests[4].input.messages.iter().flat_map(|message| &message.content).any(|part|
                matches!(part, ChatContentPart::ToolResult { call_id, is_error: true, .. } if call_id.as_ref() == id)));
        }
        assert_eq!(result.model_steps, 8, "argument repair must not require a spurious plan exchange");
    }

    #[tokio::test]
    async fn repeated_invalid_completion_reports_stop_before_compaction() {
        let quote = "inspect";
        let reads = vec![
            Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall {
                call_id: "read-1".into(), name: "read_file".into(),
                arguments: nomifun_agent_contracts::StrictJsonValue(json!({"path":"a"})),
                provider_metadata: None,
            }}),
            Ok(ChatModelEvent::ToolCallCompleted { call: ChatToolCall {
                call_id: "read-2".into(), name: "search_files".into(),
                arguments: nomifun_agent_contracts::StrictJsonValue(json!({"path":"b", "query":"needle"})),
                provider_metadata: None,
            }}),
            Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::ToolCalls }),
        ];
        let mut steps = vec![reads, plan_step("completed", quote)];
        for index in 0..4 {
            steps.push(control_step(&format!("bad-report-{index}"), crate::completion::TOOL_NAME,
                json!({"summary":"The reads returned.", "criteria":[{"disposition":"supported"}]})));
        }
        steps.push(text_step("must not be requested"));
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(steps), requests: std::sync::Mutex::new(Vec::new()),
        });
        let mut input = request();
        input.input.messages[0].content = vec![ChatContentPart::Text { text: quote.into() }];
        let error = open_session(model.clone(), Arc::new(EchoTool))
            .run_turn(AgentTurnRequest::new(
                input, two_tool_plan(AgentEffectClass::ReadOnly, true), principal(), 0,
            ))
            .await
            .unwrap_err();
        assert!(matches!(error, AgentEngineError::TurnFailed(message)
            if message.starts_with("engine control made no progress; report_completion")));
        assert_eq!(model.requests.lock().unwrap().len(), 6);
        assert_eq!(model.steps.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn recovered_turn_does_not_repeat_a_completed_write_or_execute_the_abandoned_model_batch() {
        #[derive(Default)]
        struct Journal {
            events: std::sync::Mutex<Vec<AgentEngineEvent>>,
            checkpoints: std::sync::Mutex<Vec<crate::AgentExecutionCheckpoint>>,
        }
        #[async_trait]
        impl AgentEventSink for Journal {
            async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError> {
                self.events.lock().unwrap().push(event); Ok(())
            }
            fn supports_checkpoints(&self) -> bool { true }
            async fn save_checkpoint(&self, state: crate::AgentExecutionCheckpoint) -> Result<Option<crate::AgentCheckpointReceipt>, AgentEngineError> {
                let mut checkpoints = self.checkpoints.lock().unwrap();
                checkpoints.push(state.clone());
                let revision = checkpoints.len() as u64;
                let digest = nomifun_agent_contracts::digest_payload(&state).unwrap();
                let mut events = self.events.lock().unwrap();
                events.push(AgentEngineEvent::ExecutionCheckpointSaved { step: state.model_steps, revision, digest: digest.clone() });
                Ok(Some(crate::AgentCheckpointReceipt { revision, through_seq: events.len() as u64, digest }))
            }
        }
        struct CrashModel { calls: AtomicUsize, pending: Arc<Notify> }
        #[async_trait]
        impl AgentModelPort for CrashModel {
            async fn open_stream(&self, _: ChatModelRequest, _: CancellationToken) -> Result<AgentModelStream, ChatModelError> {
                if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Ok(Box::pin(stream::iter(control_step("completed-write", "write_file", json!({"path":"a","content":"preserve"})))));
                }
                let mut partial = control_step("abandoned-write", "write_file", json!({"path":"b","content":"NEVER_EXECUTE"}));
                partial.pop();
                partial.push(Ok(ChatModelEvent::OutputTextDelta { text: "ABANDONED_MODEL_OUTPUT".into() }));
                let pending = self.pending.clone();
                Ok(Box::pin(stream::iter(partial).chain(stream::once(async move {
                    pending.notify_one(); std::future::pending::<Result<ChatModelEvent, ChatModelError>>().await
                }))))
            }
        }
        let pending = Arc::new(Notify::new());
        let journal = Arc::new(Journal::default());
        let tools = Arc::new(RecoverableReadTool { writes: AtomicUsize::new(0) });
        let mut initial = request();
        let original = initial.input.messages[0].clone();
        initial.input.max_output_tokens = Some(100);
        let request = AgentTurnRequest::new(initial, two_tool_plan(AgentEffectClass::ManagedEffect, false), principal(), 0);
        let original_request = request.clone();
        let task = tokio::spawn(run_turn(binding(), Arc::new(CrashModel { calls: AtomicUsize::new(0), pending: pending.clone() }),
            tools.clone(), journal.clone(), request, AgentContextBudget::default(), CancellationToken::new()));
        tokio::time::timeout(Duration::from_secs(2), pending.notified()).await.unwrap();
        task.abort(); assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(tools.writes.load(Ordering::SeqCst), 1);
        let snapshot = journal.checkpoints.lock().unwrap().last().cloned().unwrap();
        let snapshot: crate::AgentExecutionCheckpoint = serde_json::from_value(serde_json::to_value(snapshot).unwrap()).unwrap();
        let recorded = journal.events.lock().unwrap().clone();
        let through = recorded.iter().rposition(|event| matches!(event, AgentEngineEvent::ExecutionCheckpointSaved { .. })).unwrap() + 1;
        let revision = match &recorded[through - 1] { AgentEngineEvent::ExecutionCheckpointSaved { revision, .. } => *revision, _ => unreachable!() };
        let recovery = crate::AgentTurnRecovery::new(snapshot, revision, 1, recorded[..through].to_vec(), recorded[through..].to_vec(), vec![]).unwrap();
        let mut steps = vec![plan_step("in_progress", "inspect"), control_step("fresh-check", "read_file", json!({"path":"a"}))];
        steps.extend(completion_steps("inspect", &["fresh-check"], true));
        let model = Arc::new(ObservingModel { steps: std::sync::Mutex::new(steps), requests: Default::default() });
        let result = run_turn(binding(), model.clone(), tools.clone(), journal.clone(), original_request.with_recovery(recovery),
            AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
        assert_eq!(result.model_steps, 6);
        assert_eq!(tools.writes.load(Ordering::SeqCst), 1);
        assert!(model.requests.lock().unwrap()[0].input.messages.iter().any(|message| message == &original));
        assert!(!serde_json::to_string(&model.requests.lock().unwrap()[0].input).unwrap().contains("ABANDONED_MODEL_OUTPUT"));
        let mut history = Vec::new();
        crate::replay_closed_turn(&mut history, original, &journal.events.lock().unwrap()).unwrap();
        let history = serde_json::to_string(&history).unwrap();
        assert!(!history.contains("ABANDONED_MODEL_OUTPUT") && !history.contains("NEVER_EXECUTE"));
        assert!(history.contains("preserve"));
    }

    #[tokio::test]
    async fn execution_segments_cross_model_windows_only_after_checkpoint_acknowledgement() {
        #[derive(Default)]
        struct Journal {
            events: std::sync::Mutex<Vec<AgentEngineEvent>>,
            states: std::sync::Mutex<Vec<crate::AgentExecutionCheckpoint>>,
        }
        #[async_trait]
        impl AgentEventSink for Journal {
            fn supports_checkpoints(&self) -> bool { true }
            async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError> {
                self.events.lock().unwrap().push(event); Ok(())
            }
            async fn save_checkpoint(&self, state: crate::AgentExecutionCheckpoint) -> Result<Option<crate::AgentCheckpointReceipt>, AgentEngineError> {
                state.validate()?;
                let mut states = self.states.lock().unwrap();
                let revision = states.len() as u64 + 1;
                let digest = nomifun_agent_contracts::digest_payload(&serde_json::to_value(&state).unwrap()).unwrap();
                let mut events = self.events.lock().unwrap();
                events.push(AgentEngineEvent::ExecutionCheckpointSaved { step: state.model_steps, revision, digest: digest.clone() });
                states.push(state);
                Ok(Some(crate::AgentCheckpointReceipt { revision, digest, through_seq: events.len() as u64 }))
            }
        }
        let mut steps = vec![control_step("write-once", "write_file", json!({"path":"a","content":"preserve"})),
            plan_step("in_progress", "inspect"), control_step("fresh-read", "read_file", json!({"path":"a"}))];
        steps.extend(completion_steps("inspect", &["fresh-read"], true));
        let model = Arc::new(ObservingModel { steps: std::sync::Mutex::new(steps), requests: Default::default() });
        let tools = Arc::new(RecoverableReadTool { writes: AtomicUsize::new(0) });
        let journal = Arc::new(Journal::default());
        let result = run_turn(binding(), model.clone(), tools.clone(), journal.clone(),
            AgentTurnRequest::new(request(), two_tool_plan(AgentEffectClass::ManagedEffect, false), principal(), 0)
                .with_max_model_steps(2).with_execution_segments(crate::AgentSegmentPolicy { max_segments: 4, max_no_progress_segments: 2 }),
            AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
        assert_eq!(result.model_steps, 5);
        assert_eq!(tools.writes.load(Ordering::SeqCst), 1);
        let events = journal.events.lock().unwrap();
        let renewals = events.iter().enumerate().filter_map(|(index, event)| match event {
            AgentEngineEvent::ExecutionSegmentRenewed { segment, checkpoint_revision, model_steps, .. } => Some((index, *segment, *checkpoint_revision, *model_steps)), _ => None,
        }).collect::<Vec<_>>();
        assert_eq!(renewals.iter().map(|(_, segment, _, step)| (*segment, *step)).collect::<Vec<_>>(), vec![(2, 2), (3, 4)]);
        for (index, _, revision, step) in renewals {
            assert!(matches!(&events[index - 1], AgentEngineEvent::ExecutionCheckpointSaved { revision: saved, step: saved_step, .. } if *saved == revision && *saved_step == step));
        }
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests.iter().map(|request| request.causality.operation_id.as_ref()).collect::<std::collections::BTreeSet<_>>().len(), 5);
        assert!(journal.states.lock().unwrap().iter().all(|state| state.accepted_input_count == 1));
    }

    #[tokio::test]
    async fn checkpoint_input_indices_keep_the_actual_steering_application_order() {
        let mut request = request();
        let mut retained = request.input.messages.clone();
        let mut seen = BTreeSet::new();
        let mut applied = Vec::new();
        let steer = |id: &str, text: &str| crate::AgentSteeringInput {
            receipt_operation_id: id.into(), message_id: format!("message-{id}"), text: text.into(),
            files: vec![], inject_skills: vec![], image_count: 0, prepared_images: vec![],
        };
        crate::steering::incorporate(vec![steer("z", "Run the check"), steer("a", "Do not run the check")],
            &mut request, &mut retained, &mut seen, &mut applied).unwrap();
        assert_eq!(applied, vec!["z", "a"]);
        assert!(matches!(&retained[2].content[0], ChatContentPart::Text { text } if text == "Do not run the check"));
        assert!(crate::steering::incorporate(vec![steer("z", "must not replace prior input")],
            &mut request, &mut retained, &mut seen, &mut applied).is_err());
        assert_eq!(applied, vec!["z", "a"]);
        assert_eq!(retained.len(), 3);
    }

    #[tokio::test]
    async fn checkpoint_captures_quiescent_progress_without_tool_bodies_or_reasoning() {
        #[derive(Default)]
        struct Sink(std::sync::Mutex<Vec<crate::AgentExecutionCheckpoint>>);
        #[async_trait]
        impl AgentEventSink for Sink {
            async fn emit(&self, _: AgentEngineEvent) -> Result<(), AgentEngineError> { Ok(()) }
            fn supports_checkpoints(&self) -> bool { true }
            async fn save_checkpoint(&self, checkpoint: crate::AgentExecutionCheckpoint) -> Result<Option<crate::AgentCheckpointReceipt>, AgentEngineError> {
                self.0.lock().unwrap().push(checkpoint);
                Ok(None)
            }
        }
        let mut write = control_step("write", "write_file", json!({"path":"a","content":"PRIVATE_FILE_BODY"}));
        write.insert(0, Ok(ChatModelEvent::ReasoningDelta { text: "PRIVATE_MODEL_THOUGHT".into() }));
        let model = Arc::new(ObservingModel { steps: std::sync::Mutex::new(vec![write, text_step("done")]), requests: Default::default() });
        let sink = Arc::new(Sink::default());
        let result = run_turn(binding(), model, Arc::new(EchoTool), sink.clone(),
            AgentTurnRequest::new(request(), two_tool_plan(AgentEffectClass::ManagedEffect, false), principal(), 0),
            AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
        assert!(matches!(result.terminal, AgentTurnTerminal::Completed { .. }));
        let checkpoints = sink.0.lock().unwrap();
        assert_eq!(checkpoints.iter().map(|state| state.model_steps).collect::<Vec<_>>(), vec![0, 0, 1]);
        let latest = checkpoints.last().unwrap();
        assert_eq!(latest.tool_call_count, 1);
        assert_eq!(latest.work.successful_workspace_mutations, 1);
        let encoded = serde_json::to_string(&*checkpoints).unwrap();
        assert!(!encoded.contains("PRIVATE_FILE_BODY"));
        assert!(!encoded.contains("PRIVATE_MODEL_THOUGHT"));
    }

    #[tokio::test]
    async fn checkpoint_failure_and_cancellation_cannot_open_a_model_request() {
        struct Sink { cancellation: CancellationToken, fail: bool }
        #[async_trait]
        impl AgentEventSink for Sink {
            async fn emit(&self, _: AgentEngineEvent) -> Result<(), AgentEngineError> { Ok(()) }
            fn supports_checkpoints(&self) -> bool { true }
            async fn save_checkpoint(&self, _: crate::AgentExecutionCheckpoint) -> Result<Option<crate::AgentCheckpointReceipt>, AgentEngineError> {
                if self.fail { return Err(AgentEngineError::EventSink("checkpoint failed".into())); }
                self.cancellation.cancel();
                Ok(None)
            }
        }
        for fail in [false, true] {
            let model = Arc::new(ObservingModel { steps: Default::default(), requests: Default::default() });
            let cancellation = CancellationToken::new();
            let result = run_turn(binding(), model.clone(), Arc::new(EchoTool),
                Arc::new(Sink { cancellation: cancellation.clone(), fail }),
                AgentTurnRequest::new(request(), tool_plan(), principal(), 0),
                AgentContextBudget::default(), cancellation).await;
            if fail { assert!(matches!(result, Err(AgentEngineError::EventSink(_)))); }
            else { assert!(matches!(result.unwrap().terminal, AgentTurnTerminal::Cancelled)); }
            assert!(model.requests.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn pseudo_tool_output_is_corrected_without_executing_the_discarded_native_prefix() {
        #[derive(Default)]
        struct Sink(std::sync::Mutex<Vec<AgentEngineEvent>>);
        #[async_trait]
        impl AgentEventSink for Sink {
            async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError> {
                self.0.lock().unwrap().push(event);
                Ok(())
            }
        }
        let mut rejected = control_step("must-not-execute", "write_file", json!({"path":"a","content":"wrong"}));
        rejected.pop();
        rejected.push(Ok(ChatModelEvent::OutputTextDelta { text: "<tool_call><function=write_file>SECRET_REJECTED_BODY".into() }));
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![rejected,
                control_step("corrected", "write_file", json!({"path":"a","content":"right"})),
                text_step("done")]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let tools = Arc::new(RecoverableReadTool { writes: AtomicUsize::new(0) });
        let sink = Arc::new(Sink::default());
        let request = AgentTurnRequest::new(request(), two_tool_plan(AgentEffectClass::ManagedEffect, false), principal(), 0);
        let requirement = request.model_request.input.messages[0].clone();
        let result = run_turn(binding(), model.clone(), tools.clone(), sink.clone(), request,
            AgentContextBudget::default(), CancellationToken::new()).await.unwrap();
        assert_eq!(result.model_steps, 3);
        assert_eq!(tools.writes.load(Ordering::SeqCst), 1);
        assert_eq!(result.output_text, "done");
        let requests = model.requests.lock().unwrap();
        assert_eq!(requests[0].input.tool_choice, ChatToolChoice::Auto);
        assert_eq!(requests[1].input.tool_choice, ChatToolChoice::Required);
        assert_eq!(requests[2].input.tool_choice, ChatToolChoice::Auto);
        assert!(requests[1].input.tools.iter().any(|tool| tool.name == "write_file"));
        drop(requests);
        let events = sink.0.lock().unwrap().clone();
        assert!(events.iter().any(|event| matches!(event, AgentEngineEvent::ModelResponseRejected {
            step: 1, discarded_tool_call_ids, continuation: true, tool_hint: None,
        } if discarded_tool_call_ids == &[ToolCallId::from("must-not-execute")])));
        assert!(!events.iter().any(|event| matches!(event, AgentEngineEvent::ToolStarted { call_id, .. } if call_id.as_ref() == "must-not-execute")));
        let mut replayed = Vec::new();
        crate::replay_closed_turn(&mut replayed, requirement.clone(), &events).unwrap();
        assert!(!serde_json::to_string(&replayed).unwrap().contains("SECRET_REJECTED_BODY"));
        assert!(!replayed.iter().flat_map(|message| &message.content).any(|part| matches!(part,
            ChatContentPart::ToolCall { call_id, .. } if call_id.as_ref() == "must-not-execute")));
        let mut forged = events;
        let at = forged.iter().position(|event| matches!(event, AgentEngineEvent::ModelResponseRejected { .. })).unwrap();
        forged.insert(at, AgentEngineEvent::ToolStarted { step: 1, call_id: "must-not-execute".into(),
            capability_id: "workspace.files".into(), action_id: "workspace.files/write".into() });
        assert!(crate::replay_closed_turn(&mut Vec::new(), requirement, &forged).is_err(), "discard cannot conceal an admitted effect");
    }

    #[tokio::test]
    async fn repeated_pseudo_tools_exhaust_correction_budget_without_invocation() {
        let bad = || vec![Ok(ChatModelEvent::OutputTextDelta { text: "<tool_call><function=write_file>".into() })];
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![bad(), bad(), bad(), text_step("must not run")]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let tools = Arc::new(RecoverableReadTool { writes: AtomicUsize::new(0) });
        let result = open_session(model.clone(), tools.clone()).run_turn(
            AgentTurnRequest::new(request(), two_tool_plan(AgentEffectClass::ManagedEffect, false), principal(), 0),
        ).await;
        assert!(matches!(result, Err(AgentEngineError::TurnFailed(message)) if message.contains("protocol-correction budget")));
        assert_eq!(model.requests.lock().unwrap().len(), 3);
        assert_eq!(tools.writes.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn failed_protocol_rejection_persistence_cannot_start_a_retry() {
        struct FailedSink;
        impl FailedSink { fn model() -> Arc<ObservingModel> { Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![vec![Ok(ChatModelEvent::OutputTextDelta { text:"<tool_call><function=write_file>".into() })], text_step("must not run")]),
            requests: std::sync::Mutex::new(Vec::new()),
        }) } }
        #[async_trait]
        impl AgentEventSink for FailedSink {
            async fn emit(&self, event: AgentEngineEvent) -> Result<(), AgentEngineError> {
                if matches!(event, AgentEngineEvent::ModelResponseRejected { .. }) {
                    return Err(AgentEngineError::EventSink("injected failure".into()));
                }
                Ok(())
            }
        }
        let model = FailedSink::model();
        let result = run_turn(binding(), model.clone(), Arc::new(EchoTool), Arc::new(FailedSink),
            AgentTurnRequest::new(request(), tool_plan(), principal(), 0), AgentContextBudget::default(), CancellationToken::new()).await;
        assert!(matches!(result, Err(AgentEngineError::EventSink(_))));
        assert_eq!(model.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn malformed_tool_text_cannot_complete_a_turn_or_execute_a_tool() {
        let model = Arc::new(ObservingModel {
            steps: std::sync::Mutex::new(vec![vec![
                Ok(ChatModelEvent::OutputTextDelta { text: "I will write the file. <tool_".into() }),
                Ok(ChatModelEvent::OutputTextDelta { text: "call>\n<function=write_file>\n<parameter=content>".into() }),
                Ok(ChatModelEvent::OutputTextDelta { text: "SHOULD_NEVER_REACH_THE_UI".into() }),
                Ok(ChatModelEvent::Completed { finish_reason: ChatFinishReason::Completed }),
            ]]),
            requests: std::sync::Mutex::new(Vec::new()),
        });
        let tools = Arc::new(ConcurrencyTool {
            active: AtomicUsize::new(0), max_active: AtomicUsize::new(0),
            order: std::sync::Mutex::new(Vec::new()), delay: Duration::from_millis(1),
        });
        let result = open_session(model, tools.clone()).run_turn(AgentTurnRequest::new(
            request(), tool_plan(), principal(), 1,
        ).with_max_model_steps(1)).await;
        assert!(matches!(result, Err(AgentEngineError::TurnFailed(message))
            if message.contains("tool-call markup as text")));
        assert!(tools.order.lock().unwrap().is_empty());
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
        assert!(requests[0].input.instructions.iter().any(|instruction|
            instruction.contains("Do not emit chain-of-thought")
                && instruction.contains("give brief public progress updates")
                && instruction.contains("Use the provided function interface for tool actions")
                && !instruction.contains("<tool_call>")
                && instruction.contains("reserve the final response for the outcome")));
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
