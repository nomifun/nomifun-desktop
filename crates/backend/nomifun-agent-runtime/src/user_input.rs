//! A typed input handoff is a reply boundary, never task-completion evidence,
//! an invented answer or authority to change the accepted task.
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{AgentUserInputRequest, StrictJsonValue};
use nomifun_chat_model_broker::{ChatCausality, ChatToolCall, ChatToolDefinition};
use serde::Deserialize;

use crate::AgentEngineError;

pub(crate) const TOOL_NAME: &str = "request_user_input";
pub(crate) const ACK: &str = "Input request recorded. This is a pending input handoff, not a user answer or task-completion receipt.";

#[async_trait]
pub trait AgentUserInputRequestPort: Send + Sync + std::fmt::Debug {
    fn enabled(&self) -> bool { true }
    /// None permits this exact handoff; a bounded explanation rejects it.
    /// This read-only policy check creates no domain decision or user answer.
    async fn check(&self, causality: &ChatCausality, request: &AgentUserInputRequest)
        -> Result<Option<String>, AgentEngineError>;
}

pub(crate) fn enabled(port: Option<&Arc<dyn AgentUserInputRequestPort>>) -> bool {
    port.is_some_and(|port| port.enabled())
}

pub(crate) fn definition() -> ChatToolDefinition {
    ChatToolDefinition {
        name: TOOL_NAME.into(), deferred: false,
        description: "Ask for user information or an external user action needed to continue the accepted task. The host ends this computational turn and hands input back to the user; Session context and owned background processes remain available. A process exit never answers the question or authorizes a new attempt. Submit alone with question and remaining_work. This communication control remains available after a known command failure and during completion review; it never reopens execution tools, erases failure evidence or authorizes replay. Preserve the complete request, known failures and completed effects. Use report_completion only for the real completion account. Do not duplicate the question in assistant text. Unsettled patch obligations still require their original recovery controls. Never cancel an owned process merely to ask for required authorization.".into(),
        input_schema: StrictJsonValue(serde_json::json!({"type":"object","additionalProperties":false,
            "properties":{"question":{"type":"string","minLength":1,"maxLength":16384},
                "remaining_work":{"type":"string","minLength":1,"maxLength":16384}},
            "required":["question","remaining_work"]})),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments { question: String, remaining_work: String }

pub(crate) fn from_call(call: &ChatToolCall) -> Result<AgentUserInputRequest, String> {
    if call.name != TOOL_NAME { return Err("input request has a different tool identity".into()); }
    let args: Arguments = serde_json::from_value(call.arguments.0.clone()).map_err(|_| "Submit only question and remaining_work as nonempty strings".to_owned())?;
    let request = AgentUserInputRequest { request_id: call.call_id.clone(), question: args.question, remaining_work: args.remaining_work };
    request.validate()?;
    Ok(request)
}

pub(crate) fn bounded_rejection(message: &str) -> &str {
    let mut end = message.len().min(4096);
    while !message.is_char_boundary(end) { end -= 1; }
    &message[..end]
}

/// Rebuild only committed native declarations. A proposal or tool result
/// alone cannot invent a handoff, and a later accepted input retires it.
pub fn recorded_pending(events: &[crate::AgentEngineEvent])
    -> Result<Option<AgentUserInputRequest>, AgentEngineError> {
    use crate::AgentEngineEvent as Event;
    let invalid = || AgentEngineError::ReplayContract("input handoff differs from its committed source question".into());
    let mut calls = std::collections::BTreeMap::new();
    let mut pending = None;
    let mut step = 0;
    let mut proposals = std::collections::BTreeSet::new();
    for event in events {
        match event {
            Event::ModelStepStarted { step: current, .. } => {
                if pending.is_some() { return Err(invalid()); }
                step = *current;
                proposals.clear();
            }
            Event::ToolCallDelta { step: current, call_id, .. } if *current > 0 => { proposals.insert(call_id.clone()); }
            Event::ToolCallCompleted { step, call } if call.name == TOOL_NAME => {
                proposals.insert(call.call_id.clone());
                calls.insert(call.call_id.clone(), (*step, call));
            }
            Event::ToolCallCompleted { step, call } if *step > 0 => { proposals.insert(call.call_id.clone()); }
            Event::UserInputRequested { step: current, request } => {
                request.validate().map_err(AgentEngineError::InvalidContract)?;
                let Some((source_step, call)) = calls.get(&request.request_id) else { return Err(invalid()); };
                if pending.is_some() || *current != step || source_step != current || proposals.len() != 1
                    || from_call(call).as_ref() != Ok(request) { return Err(invalid()); }
                pending = Some(request.clone());
            }
            Event::ToolCompleted { result, .. } | Event::ToolOutcomeReconciled { result, .. }
                if pending.as_ref().is_some_and(|request| request.request_id == result.call_id) && result.is_error => return Err(invalid()),
            Event::ToolStarted { .. } if pending.is_some() => return Err(invalid()),
            Event::SteeringInputs { inputs } if !inputs.is_empty() => pending = None,
            // Resource observation is independent of a human question. Only
            // real accepted input can retire the source question.
            Event::TurnInputRequired { request, .. } if pending.as_ref() != Some(request) => return Err(invalid()),
            _ => {}
        }
    }
    if pending.is_some() && matches!(events.last(), Some(Event::TurnCompleted { .. })) { return Err(invalid()); }
    Ok(pending)
}
