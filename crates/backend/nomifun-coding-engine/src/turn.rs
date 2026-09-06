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
use crate::error::CodingEngineError;
use crate::events::{CodingEngineEvent, CodingEventSink};
use crate::model::CodingModelPort;
use crate::tool::{
    invocation_for, parse_completed_arguments, validate_tool_argument_size, CodingEffectClass,
    CodingToolInvoker, CodingToolPlan, CodingToolResult,
};

const DEFAULT_MAX_MODEL_STEPS: u16 = 32;

#[derive(Clone, Debug)]
pub struct CodingTurnRequest {
    pub model_request: ChatModelRequest,
    pub tool_plan: CodingToolPlan,
    pub principal: PrincipalRef,
    pub active_set_generation: u64,
    pub max_model_steps: u16,
}

impl CodingTurnRequest {
    pub fn new(
        model_request: ChatModelRequest,
        tool_plan: CodingToolPlan,
        principal: PrincipalRef,
        active_set_generation: u64,
    ) -> Self {
        Self {
            model_request,
            tool_plan,
            principal,
            active_set_generation,
            max_model_steps: DEFAULT_MAX_MODEL_STEPS,
        }
    }

    pub fn with_max_model_steps(mut self, max_model_steps: u16) -> Self {
        self.max_model_steps = max_model_steps;
        self
    }

    fn validate_for(
        &self,
        binding: &EngineBinding,
    ) -> Result<(), CodingEngineError> {
        if self.max_model_steps == 0 {
            return Err(CodingEngineError::InvalidContract(
                "max_model_steps must be greater than zero".to_owned(),
            ));
        }
        if self.model_request.causality.agent_session_id.as_ref().is_empty() {
            return Err(CodingEngineError::InvalidContract(
                "model request must identify an AgentSession".to_owned(),
            ));
        }
        if self.principal.principal_id.trim().is_empty()
            || self.principal.principal_kind.trim().is_empty()
        {
            return Err(CodingEngineError::InvalidContract(
                "coding turn principal must be complete".to_owned(),
            ));
        }
        if &self.model_request.causality.agent_session_id != binding.agent_session_id() {
            return Err(CodingEngineError::TurnBindingMismatch {
                field: "agent_session_id",
            });
        }
        if &self.model_request.causality.resolved_snapshot_ref != binding.resolved_snapshot_ref() {
            return Err(CodingEngineError::TurnBindingMismatch {
                field: "resolved_snapshot_ref",
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodingTurnTerminal {
    Completed { finish_reason: ChatFinishReason },
    Cancelled,
    Failed { message: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct CodingTurnResult {
    pub agent_session_id: AgentSessionId,
    pub turn_operation_id: OperationId,
    pub model_steps: u16,
    pub output_text: String,
    pub reasoning_text: String,
    pub tool_call_count: u32,
    pub provider_round_id: Option<ProviderRoundId>,
    pub terminal: CodingTurnTerminal,
}

pub(crate) async fn run_turn(
    binding: EngineBinding,
    model: Arc<dyn CodingModelPort>,
    tools: Arc<dyn CodingToolInvoker>,
    event_sink: Arc<dyn CodingEventSink>,
    request: CodingTurnRequest,
    cancellation: CancellationToken,
) -> Result<CodingTurnResult, CodingEngineError> {
    request.validate_for(&binding)?;

    let mut model_request = request.model_request;
    let requested_tool_choice = model_request.input.tool_choice.clone();
    model_request.input.tools = request.tool_plan.model_definitions();
    model_request.input.tool_choice = if request.tool_plan.is_empty() {
        ChatToolChoice::None
    } else if matches!(requested_tool_choice, ChatToolChoice::None) {
        ChatToolChoice::Auto
    } else {
        requested_tool_choice
    };
    model_request
        .validate()
        .map_err(|error| CodingEngineError::InvalidContract(error.to_string()))?;

    let agent_session_id = model_request.causality.agent_session_id.clone();
    let turn_operation_id = model_request.causality.turn_operation_id.clone();
    event_sink
        .emit(CodingEngineEvent::TurnStarted {
            binding: binding.clone(),
            turn_operation_id: turn_operation_id.clone(),
        })
        .await?;

    let mut output_text = String::new();
    let mut reasoning_text = String::new();
    let mut model_steps = 0_u16;
    let mut tool_call_count = 0_u32;
    let mut provider_round_id = None;

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

    while model_steps < request.max_model_steps {
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

        model_steps = model_steps.saturating_add(1);
        let model_operation_id =
            OperationId::from(format!("{}:model:{}", turn_operation_id.as_ref(), model_steps));
        model_request.causality.operation_id = model_operation_id.clone();
        event_sink
            .emit(CodingEngineEvent::ModelStepStarted {
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
            Err(error) => return Err(CodingEngineError::from_model_error(error)),
        };
        let mut step = StepState::default();
        let mut saw_terminal = false;

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
                Err(error) => return Err(CodingEngineError::from_model_error(error)),
            };
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
                        .emit(CodingEngineEvent::OutputTextDelta {
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
                        .emit(CodingEngineEvent::ReasoningDelta {
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
                ChatModelEvent::ToolCallDelta {
                    call_id,
                    name,
                    arguments_delta,
                } => {
                    step.record_tool_delta(&call_id, &name, &arguments_delta)?;
                    event_sink
                        .emit(CodingEngineEvent::ToolCallDelta {
                            step: model_steps,
                            call_id,
                            name,
                            arguments_delta,
                        })
                        .await?;
                }
                ChatModelEvent::ToolCallCompleted { call } => {
                    step.record_tool_completed(&call)?;
                    event_sink
                        .emit(CodingEngineEvent::ToolCallCompleted {
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
                    event_sink
                        .emit(CodingEngineEvent::Usage {
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
                            "native Responses item {item_type:?} is not yet representable in Coding history"
                        ),
                    )
                    .await;
                }
                ChatModelEvent::OutputAudioDelta { .. } => {
                    return fail_turn(
                        &event_sink,
                        model_steps,
                        "audio output is outside the Coding Engine core",
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
            return Err(CodingEngineError::ModelStreamEndedWithoutTerminal);
        }

        step.finalize()?;
        let finish_reason = step
            .finish_reason
            .ok_or(CodingEngineError::ModelStreamEndedWithoutTerminal)?;
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
        if !step.has_tool_calls() && matches!(finish_reason, ChatFinishReason::ToolCalls) {
            return fail_turn(
                &event_sink,
                model_steps,
                "model returned a tool-call terminal reason without Tool Calls",
            )
            .await;
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

            append_assistant_step(&mut model_request, &step)?;
            let results = match invoke_tool_calls(
                &agent_session_id,
                &request.principal,
                &model_request,
                request.active_set_generation,
                &request.tool_plan,
                tools.as_ref(),
                event_sink.as_ref(),
                &step,
                model_steps,
                &cancellation,
            )
            .await
            {
                Ok(results) => results,
                Err(CodingEngineError::Cancelled) => {
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
            if let Some(round_id) = step.provider_round_id {
                model_request.input.provider_round_parent = Some(round_id);
            }
            model_request
                .validate()
                .map_err(|error| CodingEngineError::InvalidContract(error.to_string()))?;
            continue;
        }

        append_assistant_step(&mut model_request, &step)?;
        model_request
            .validate()
            .map_err(|error| CodingEngineError::InvalidContract(error.to_string()))?;
        let result = CodingTurnResult {
            agent_session_id,
            turn_operation_id,
            model_steps,
            output_text,
            reasoning_text,
            tool_call_count,
            provider_round_id,
            terminal: CodingTurnTerminal::Completed { finish_reason },
        };
        event_sink
            .emit(CodingEngineEvent::TurnCompleted {
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

async fn fail_turn(
    event_sink: &Arc<dyn CodingEventSink>,
    model_steps: u16,
    message: impl Into<String>,
) -> Result<CodingTurnResult, CodingEngineError> {
    let message = message.into();
    event_sink
        .emit(CodingEngineEvent::TurnFailed {
            model_steps,
            message: message.clone(),
        })
        .await?;
    Err(CodingEngineError::TurnFailed(message))
}

async fn cancelled_turn(
    event_sink: &Arc<dyn CodingEventSink>,
    agent_session_id: &AgentSessionId,
    turn_operation_id: &OperationId,
    model_steps: u16,
    output_text: &str,
    reasoning_text: &str,
    tool_call_count: u32,
    provider_round_id: Option<ProviderRoundId>,
) -> Result<CodingTurnResult, CodingEngineError> {
    event_sink
        .emit(CodingEngineEvent::TurnCancelled { model_steps })
        .await?;
    Ok(CodingTurnResult {
        agent_session_id: agent_session_id.clone(),
        turn_operation_id: turn_operation_id.clone(),
        model_steps,
        output_text: output_text.to_owned(),
        reasoning_text: reasoning_text.to_owned(),
        tool_call_count,
        provider_round_id,
        terminal: CodingTurnTerminal::Cancelled,
    })
}

async fn invoke_tool_calls(
    agent_session_id: &AgentSessionId,
    principal: &PrincipalRef,
    model_request: &ChatModelRequest,
    active_set_generation: u64,
    plan: &CodingToolPlan,
    invoker: &dyn CodingToolInvoker,
    event_sink: &dyn CodingEventSink,
    step: &StepState,
    model_step: u16,
    cancellation: &CancellationToken,
) -> Result<Vec<(ToolCallId, CodingToolResult)>, CodingEngineError> {
    let mut calls = Vec::with_capacity(step.call_order.len());
    let mut can_parallelize = !step.call_order.is_empty();
    for call_id in &step.call_order {
        if cancellation.is_cancelled() {
            return Err(CodingEngineError::Cancelled);
        }
        let pending = step
            .calls
            .get(call_id)
            .ok_or_else(|| CodingEngineError::InvalidModelEvent("tool call order is corrupt".to_owned()))?;
        let call = pending
            .completed
            .clone()
            .ok_or_else(|| {
                CodingEngineError::InvalidModelEvent(format!(
                    "tool call {} was not completed before the model terminal event",
                    call_id.as_ref()
                ))
            })?;
        let binding = plan
            .binding(&call.name)
            .ok_or_else(|| CodingEngineError::ToolNotExposed(call.name.clone()))?;
        can_parallelize &= binding.parallel_safe
            && matches!(binding.effect_class, CodingEffectClass::ReadOnly);
        let invocation = invocation_for(
            agent_session_id.clone(),
            principal.clone(),
            model_request.causality.resolved_snapshot_ref.clone(),
            active_set_generation,
            &model_request.causality.turn_operation_id,
            call,
            binding,
        );
        event_sink
            .emit(CodingEngineEvent::ToolStarted {
                step: model_step,
                call_id: invocation.call.call_id.clone(),
                capability_id: invocation.binding.capability_id.clone(),
                action_id: invocation.binding.action_id.clone(),
            })
            .await?;
        calls.push(invocation);
    }

    if can_parallelize {
        let invocations = calls.into_iter().map(|invocation| async move {
            let call_id = invocation.call.call_id.clone();
            let result = invoker.invoke(invocation, cancellation.clone()).await;
            (call_id, result)
        });
        let results = tokio::select! {
            _ = cancellation.cancelled() => return Err(CodingEngineError::Cancelled),
            results = futures::future::join_all(invocations) => results,
        };
        return finish_tool_results(results, event_sink, model_step, cancellation).await;
    }

    let mut results = Vec::new();
    for invocation in calls {
        if cancellation.is_cancelled() {
            return Err(CodingEngineError::Cancelled);
        }
        let call_id = invocation.call.call_id.clone();
        let result = tokio::select! {
            _ = cancellation.cancelled() => return Err(CodingEngineError::Cancelled),
            result = invoker.invoke(invocation, cancellation.clone()) => result,
        };
        results.push((call_id, result));
        if cancellation.is_cancelled() {
            break;
        }
    }
    finish_tool_results(results, event_sink, model_step, cancellation).await
}

async fn finish_tool_results(
    results: Vec<(
        ToolCallId,
        Result<CodingToolResult, CodingEngineError>,
    )>,
    event_sink: &dyn CodingEventSink,
    model_step: u16,
    cancellation: &CancellationToken,
) -> Result<Vec<(ToolCallId, CodingToolResult)>, CodingEngineError> {
    if cancellation.is_cancelled() {
        return Err(CodingEngineError::Cancelled);
    }
    let mut normalized = Vec::with_capacity(results.len());
    for (call_id, result) in results {
        let result = match result {
            Ok(result) => result,
            Err(CodingEngineError::Cancelled) => return Err(CodingEngineError::Cancelled),
            Err(error) => CodingToolResult::text(call_id.clone(), error.to_string(), true),
        };
        result.validate_for(&call_id)?;
        event_sink
            .emit(CodingEngineEvent::ToolCompleted {
                step: model_step,
                result: result.clone(),
            })
            .await?;
        normalized.push((call_id, result));
    }
    Ok(normalized)
}

fn append_assistant_step(
    request: &mut ChatModelRequest,
    step: &StepState,
) -> Result<(), CodingEngineError> {
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
        if let Some(ChatContentPart::Reasoning { text: existing, .. }) =
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
    ) -> Result<(), CodingEngineError> {
        if let Some(ChatContentPart::Reasoning {
            signature: existing,
            text,
            ..
        }) = self.assistant_content.last_mut()
        {
            if existing.is_some() {
                return Err(CodingEngineError::InvalidModelEvent(
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
                return Err(CodingEngineError::InvalidModelEvent(
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
    ) -> Result<(), CodingEngineError> {
        if call_id.as_ref().trim().is_empty() {
            return Err(CodingEngineError::InvalidModelEvent(
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
            return Err(CodingEngineError::InvalidModelEvent(format!(
                "tool call {} emitted a delta after completion",
                call_id.as_ref()
            )));
        }
        if !name.is_empty() {
            if !entry.name.is_empty() && entry.name != name {
                return Err(CodingEngineError::InvalidModelEvent(format!(
                    "tool call {} changed its name",
                    call_id.as_ref()
                )));
            }
            entry.name = name.to_owned();
        }
        entry.arguments.push_str(arguments_delta);
        validate_tool_argument_size(&entry.arguments)
    }

    fn record_tool_completed(&mut self, call: &ChatToolCall) -> Result<(), CodingEngineError> {
        call.validate()
            .map_err(|error| CodingEngineError::InvalidModelEvent(error.to_string()))?;
        if !call.arguments.0.is_object() {
            return Err(CodingEngineError::InvalidModelEvent(format!(
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
            return Err(CodingEngineError::InvalidModelEvent(format!(
                "tool call {} changed its name before completion",
                call.call_id.as_ref()
            )));
        }
        if entry.completed.is_some() {
            return Err(CodingEngineError::InvalidModelEvent(format!(
                "tool call {} completed more than once",
                call.call_id.as_ref()
            )));
        }
        parse_completed_arguments(call)?;
        if !entry.arguments.is_empty() {
            let assembled = serde_json::from_str::<serde_json::Value>(&entry.arguments)
                .map_err(|error| {
                    CodingEngineError::InvalidModelEvent(format!(
                        "tool call {} emitted invalid assembled arguments: {error}",
                        call.call_id.as_ref()
                    ))
                })?;
            if assembled != call.arguments.0 {
                return Err(CodingEngineError::InvalidModelEvent(format!(
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

    fn finalize(&self) -> Result<(), CodingEngineError> {
        for call_id in &self.call_order {
            let call = self
                .calls
                .get(call_id)
                .ok_or_else(|| CodingEngineError::InvalidModelEvent("tool call order is corrupt".to_owned()))?;
            if call.completed.is_none() {
                return Err(CodingEngineError::InvalidModelEvent(format!(
                    "tool call {} was not completed before the model terminal event",
                    call_id.as_ref()
                )));
            }
        }
        if self.pending_reasoning_signature.is_some()
            || self.assistant_content.iter().any(|part| {
            matches!(
                part,
                ChatContentPart::Reasoning { text, .. } if text.is_empty()
            )
        })
        {
            return Err(CodingEngineError::InvalidModelEvent(
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

    use super::*;
    use crate::engine::{
        CodingEngine, CodingEngineBuild, CodingRuntimeProfile, EngineBuildId, EngineFamilyId,
    };
    use crate::events::NoopCodingEventSink;
    use crate::model::CodingModelStream;
    use crate::tool::{
        CodingEffectClass, CodingToolBinding, CodingToolInvocation, CodingToolInvoker,
        CodingToolPlan, CodingToolResult,
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
        let engine = CodingEngine::new(CodingEngineBuild {
            family_id: EngineFamilyId::from("nomifun.coding"),
            build_id: EngineBuildId::from("coding-dev"),
            build_digest: DigestHex::from("a".repeat(64)),
            display_name: "NomiFun Coding Engine".to_owned(),
            supported_profiles: vec![CodingRuntimeProfile::Coding],
        })
        .unwrap();
        engine
            .bind(
                AgentSessionId::from("session"),
                nomifun_agent_contracts::RuntimeBindingId::from("binding"),
                CodingRuntimeProfile::Coding,
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

    fn tool_plan() -> CodingToolPlan {
        let definition = ChatToolDefinition {
                name: "read_file".to_owned(),
                description: "read a file".to_owned(),
                input_schema: nomifun_agent_contracts::StrictJsonValue(json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}}
                })),
                deferred: false,
            };
        CodingToolPlan::new([CodingToolBinding {
            model_name: "read_file".to_owned(),
            schema_digest: crate::tool::input_schema_digest(&definition.input_schema).unwrap(),
            canonical_input_schema_ref: nomifun_agent_contracts::CanonicalSchemaRef::from(
                "schema://fs.read/input",
            ),
            capability_contract_digest: DigestHex::from("c".repeat(64)),
            definition,
            capability_id: nomifun_agent_contracts::CapabilityId::from("fs.read"),
            action_id: ActionId::from("read"),
            resource_binding_ids: BTreeSet::new(),
            effect_class: CodingEffectClass::ReadOnly,
            parallel_safe: true,
        }])
        .unwrap()
    }

    struct ScriptedModel {
        steps: std::sync::Mutex<Vec<Vec<Result<ChatModelEvent, ChatModelError>>>>,
    }

    #[async_trait]
    impl CodingModelPort for ScriptedModel {
        async fn open_stream(
            &self,
            _request: ChatModelRequest,
            _cancellation: CancellationToken,
        ) -> Result<CodingModelStream, ChatModelError> {
            let events = self.steps.lock().unwrap().remove(0);
            Ok(Box::pin(stream::iter(events)))
        }
    }

    struct EchoTool;

    #[async_trait]
    impl CodingToolInvoker for EchoTool {
        async fn invoke(
            &self,
            invocation: CodingToolInvocation,
            _cancellation: CancellationToken,
        ) -> Result<CodingToolResult, CodingEngineError> {
            Ok(CodingToolResult::text(
                invocation.call.call_id,
                "file contents",
                false,
            ))
        }
    }

    struct BlockingTool {
        started: Arc<Notify>,
    }

    #[async_trait]
    impl CodingToolInvoker for BlockingTool {
        async fn invoke(
            &self,
            _invocation: CodingToolInvocation,
            cancellation: CancellationToken,
        ) -> Result<CodingToolResult, CodingEngineError> {
            self.started.notify_one();
            cancellation.cancelled().await;
            Err(CodingEngineError::Cancelled)
        }
    }

    fn open_session(
        model: Arc<dyn CodingModelPort>,
        tools: Arc<dyn CodingToolInvoker>,
    ) -> crate::engine::CodingEngineSession {
        let engine = CodingEngine::new(CodingEngineBuild {
            family_id: EngineFamilyId::from("nomifun.coding"),
            build_id: EngineBuildId::from("coding-dev"),
            build_digest: DigestHex::from("a".repeat(64)),
            display_name: "NomiFun Coding Engine".to_owned(),
            supported_profiles: vec![CodingRuntimeProfile::Coding],
        })
        .unwrap();
        engine
            .open_session(
                binding(),
                model,
                tools,
                Some(Arc::new(NoopCodingEventSink)),
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
    async fn tool_call_continues_into_a_second_model_step() {
        let call_id = ToolCallId::from("call-1");
        let model = Arc::new(ScriptedModel {
            steps: std::sync::Mutex::new(vec![
                vec![
                    Ok(ChatModelEvent::ToolCallDelta {
                        call_id: call_id.clone(),
                        name: "read_file".to_owned(),
                        arguments_delta: r#"{"path":"README.md"}"#.to_owned(),
                    }),
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
                ],
                vec![
                    Ok(ChatModelEvent::OutputTextDelta {
                        text: "done".to_owned(),
                    }),
                    Ok(ChatModelEvent::Completed {
                        finish_reason: ChatFinishReason::Completed,
                    }),
                ],
            ]),
        });
        let plan = tool_plan();
        let session = open_session(model, Arc::new(EchoTool));
        let result = session
            .run_turn(CodingTurnRequest::new(
                request(),
                plan,
                principal(),
                1,
            ))
            .await
            .unwrap();
        assert_eq!(result.output_text, "done");
        assert_eq!(result.model_steps, 2);
        assert_eq!(result.tool_call_count, 1);
        assert!(matches!(
            result.terminal,
            CodingTurnTerminal::Completed {
                finish_reason: ChatFinishReason::Completed
            }
        ));
    }

    #[tokio::test]
    async fn plain_text_turn_completes_without_tools() {
        let model = Arc::new(ScriptedModel {
            steps: std::sync::Mutex::new(vec![vec![
                Ok(ChatModelEvent::OutputTextDelta {
                    text: "hello".to_owned(),
                }),
                Ok(ChatModelEvent::Completed {
                    finish_reason: ChatFinishReason::Completed,
                }),
            ]]),
        });
        let session = open_session(model, Arc::new(EchoTool));

        let result = session
            .run_turn(CodingTurnRequest::new(
                request(),
                CodingToolPlan::default(),
                principal(),
                1,
            ))
            .await
            .unwrap();

        assert_eq!(result.output_text, "hello");
        assert_eq!(result.model_steps, 1);
        assert!(matches!(
            result.terminal,
            CodingTurnTerminal::Completed {
                finish_reason: ChatFinishReason::Completed
            }
        ));
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
        let session = open_session(Arc::clone(&model) as Arc<dyn CodingModelPort>, Arc::new(EchoTool));
        let mut mismatched_request = request();
        mismatched_request.causality.agent_session_id = AgentSessionId::from("other-session");

        let error = session
            .run_turn(CodingTurnRequest::new(
                mismatched_request,
                CodingToolPlan::default(),
                principal(),
                1,
            ))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            CodingEngineError::TurnBindingMismatch {
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
            model,
            Arc::new(BlockingTool {
                started: Arc::clone(&started),
            }),
        ));
        let task_session = Arc::clone(&session);
        let task = tokio::spawn(async move {
            task_session
                .run_turn(CodingTurnRequest::new(
                    request(),
                    tool_plan(),
                    principal(),
                    1,
                ))
                .await
        });

        started.notified().await;
        assert!(session.cancel().await);
        let result = task.await.unwrap().unwrap();

        assert!(matches!(result.terminal, CodingTurnTerminal::Cancelled));
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
            model,
            Arc::new(BlockingTool {
                started: Arc::clone(&started),
            }),
        ));
        let task_session = Arc::clone(&session);
        let task = tokio::spawn(async move {
            task_session
                .run_turn(CodingTurnRequest::new(
                    request(),
                    tool_plan(),
                    principal(),
                    1,
                ))
                .await
        });
        started.notified().await;

        let second = session
            .run_turn(CodingTurnRequest::new(
                request(),
                tool_plan(),
                principal(),
                1,
            ))
            .await;
        assert!(matches!(second, Err(CodingEngineError::TurnAlreadyRunning)));

        session.cancel().await;
        assert!(matches!(
            task.await.unwrap().unwrap().terminal,
            CodingTurnTerminal::Cancelled
        ));
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
            Arc::clone(&model) as Arc<dyn CodingModelPort>,
            Arc::new(EchoTool),
        );

        session.dispose().await;
        session.dispose().await;
        assert!(session.is_disposed().await);
        assert!(matches!(
            session
                .run_turn(CodingTurnRequest::new(
                    request(),
                    CodingToolPlan::default(),
                    principal(),
                    1,
                ))
                .await,
            Err(CodingEngineError::SessionDisposed)
        ));
        assert_eq!(model.steps.lock().unwrap().len(), 1);
    }
}
