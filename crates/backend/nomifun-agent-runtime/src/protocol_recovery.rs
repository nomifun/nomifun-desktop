//! One bounded model-response coordinator, reduced from committed facts only.
use nomifun_chat_model_broker::{ChatMessage, ChatRole, ChatToolChoice, ChatToolDefinition, ToolCallId};
use crate::{AgentEngineError, AgentEngineEvent, AgentModelResponseRejection};

const MAX_FEEDBACK_BYTES: usize = 24 * 1024;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ProtocolRecovery {
    consecutive: u8,
    total: u8,
    pending: Option<AgentEngineEvent>,
    tool_hint: Option<String>,
    current_step: Option<u16>,
    last_verdict: Option<u16>,
}

/// Strategies can refuse correction, never enlarge budgets, repair arguments,
/// invoke models or grant tool authority.
pub(crate) trait ProtocolCorrectionStrategy {
    fn allow(&self, rejection: &AgentModelResponseRejection) -> bool;
}
struct NativeProtocolCorrection;
impl ProtocolCorrectionStrategy for NativeProtocolCorrection {
    fn allow(&self, reason: &AgentModelResponseRejection) -> bool {
        !matches!(reason, AgentModelResponseRejection::Unclassified)
    }
}

pub(crate) struct PreparedProtocolRejection {
    event: AgentEngineEvent,
    expected_state: ProtocolRecovery,
}
impl PreparedProtocolRejection {
    pub(crate) fn event(&self) -> &AgentEngineEvent { &self.event }
    pub(crate) fn continuation(&self) -> bool {
        matches!(self.event, AgentEngineEvent::ModelResponseRejected { continuation: true, .. })
    }
}

impl ProtocolRecovery {
    pub(crate) fn restore<'a>(&mut self, events: impl Iterator<Item = &'a AgentEngineEvent>) -> Result<(), AgentEngineError> {
        for event in events { self.observe(event)?; }
        Ok(())
    }

    pub(crate) fn prepare_rejection(&self, step: u16, ids: Vec<ToolCallId>, reason: AgentModelResponseRejection,
        hint: Option<String>, capacity: bool) -> Result<PreparedProtocolRejection, AgentEngineError> {
        self.prepare_rejection_using(step, ids, reason, hint, capacity, &NativeProtocolCorrection)
    }

    pub(crate) fn prepare_rejection_using(&self, step: u16, ids: Vec<ToolCallId>, reason: AgentModelResponseRejection,
        hint: Option<String>, capacity: bool, strategy: &dyn ProtocolCorrectionStrategy)
        -> Result<PreparedProtocolRejection, AgentEngineError> {
        if matches!(reason, AgentModelResponseRejection::Unclassified) {
            return Err(invalid("new rejection requires an observed typed reason"));
        }
        let continuation = capacity && self.consecutive < 2 && self.total < 8 && strategy.allow(&reason);
        let event = AgentEngineEvent::ModelResponseRejected { step, discarded_tool_call_ids: ids, continuation,
            tool_hint: safe_hint(hint), feedback: Some(render_feedback(&reason, continuation)), rejection: Some(reason) };
        validate_rejection(&event)?;
        let mut candidate = self.clone(); candidate.observe(&event)?;
        Ok(PreparedProtocolRejection { event, expected_state: self.clone() })
    }

    /// Call only after the canonical sink committed event(). Failed appends
    /// leave the correction state and budget unchanged.
    pub(crate) fn commit_rejection(&mut self, prepared: PreparedProtocolRejection) -> Result<ChatMessage, AgentEngineError> {
        if self != &prepared.expected_state { return Err(invalid("prepared correction differs from its source state")); }
        let feedback = rejection_feedback(&prepared.event)?;
        self.observe(&prepared.event)?;
        Ok(feedback)
    }

    /// Shared by live commit, closed replay and native checkpoint recovery.
    pub(crate) fn observe(&mut self, event: &AgentEngineEvent) -> Result<(), AgentEngineError> {
        match event {
            AgentEngineEvent::ModelStepStarted { step, correction_source, .. } => {
                if *step == 0 || self.current_step.is_some_and(|previous| *step <= previous) {
                    return Err(invalid("model steps are not strictly ordered"));
                }
                if let Some(source) = correction_source {
                    if *source >= *step || self.correction_source() != Some(*source) {
                        return Err(invalid("correction source is not a pending committed rejection"));
                    }
                } else if self.pending.as_ref().is_some_and(|event| matches!(event,
                    AgentEngineEvent::ModelResponseRejected { continuation: true, rejection: Some(_), .. })) {
                    return Err(invalid("new correction step omitted its committed source"));
                }
                self.current_step = Some(*step);
            }
            AgentEngineEvent::ModelResponseRejected { step, continuation, tool_hint, .. } => {
                validate_rejection(event)?; self.validate_verdict(*step)?;
                if *continuation {
                    self.consecutive = self.consecutive.saturating_add(1).min(2);
                    self.total = self.total.saturating_add(1).min(8);
                }
                self.pending = Some(event.clone()); self.tool_hint = safe_hint(tool_hint.clone()); self.last_verdict = Some(*step);
            }
            AgentEngineEvent::ModelResponseValidated { step } => {
                self.validate_verdict(*step)?;
                self.consecutive = 0; self.release_constraint(); self.last_verdict = Some(*step);
            }
            AgentEngineEvent::ModelOutputTruncated { .. } | AgentEngineEvent::SteeringInputs { .. }
            | AgentEngineEvent::VoiceModelStepSuperseded { .. } | AgentEngineEvent::DeliveryReviewSuperseded { .. } => self.release_constraint(),
            AgentEngineEvent::ExecutionTailReconciled { retry_stall_guards: true, .. } => {
                self.total = 0; self.consecutive = 0; self.release_constraint();
            }
            _ => {}
        }
        Ok(())
    }

    fn validate_verdict(&self, step: u16) -> Result<(), AgentEngineError> {
        if step == 0 || self.current_step.is_some_and(|current| current != step)
            || self.last_verdict.is_some_and(|previous| previous >= step) {
            return Err(invalid("verdict is outside its step or repeats a verdict"));
        }
        Ok(())
    }

    /// A committed refusal to continue survives a native pause. Only the
    /// existing real-input or explicit retry-grant events release it.
    pub(crate) fn continuation_stopped(&self)->bool {
        matches!(&self.pending,Some(AgentEngineEvent::ModelResponseRejected {continuation:false,..}))
    }
    pub(crate) fn correction_source(&self) -> Option<u16> {
        match &self.pending {
            Some(AgentEngineEvent::ModelResponseRejected { step, continuation: true, .. }) => Some(*step), _ => None,
        }
    }
    pub(crate) fn needs_validation(&self) -> bool { self.pending.is_some() || self.consecutive != 0 }
    pub(crate) fn pending_feedback(&self) -> Option<ChatMessage> {
        self.pending.as_ref().map(|event| rejection_feedback(event).expect("pending rejection was validated before reduction"))
    }
    /// New input releases constraints without replenishing the budget.
    pub(crate) fn observe_new_input(&mut self) { self.release_constraint(); }
    pub(crate) fn release_constraint(&mut self) { self.pending = None; self.tool_hint = None; }

    pub(crate) fn constrain_exposed_tool(&self, choice: &mut ChatToolChoice, tools: &[ChatToolDefinition]) {
        if self.correction_source().is_some() && !matches!(choice, ChatToolChoice::Specific { .. })
            && let Some(name) = &self.tool_hint && tools.iter().any(|tool| &tool.name == name) {
            *choice = ChatToolChoice::Specific { name: name.clone() };
        }
    }
    pub(crate) fn narrow_repair_surface(&self, choice: &ChatToolChoice, tools: &mut Vec<ChatToolDefinition>) -> bool {
        if self.correction_source().is_some() && let Some(name) = &self.tool_hint
            && matches!(choice, ChatToolChoice::Specific { name: chosen } if chosen == name)
            && tools.iter().any(|tool| &tool.name == name) {
            tools.retain(|tool| &tool.name == name); true
        } else { false }
    }
    pub(crate) fn constrain_tool_choice(&self, choice: &mut ChatToolChoice, has_tools: bool) {
        let repairs_tool_proposal = self.pending.as_ref().is_some_and(|event| matches!(event,
            AgentEngineEvent::ModelResponseRejected { rejection: Some(
                AgentModelResponseRejection::NonNativeMarkup | AgentModelResponseRejection::ToolArguments { .. }), .. }));
        if self.correction_source().is_some() && repairs_tool_proposal && has_tools
            && !matches!(choice, ChatToolChoice::Specific { .. }) {
            *choice = ChatToolChoice::Required;
        }
    }
}

fn safe_hint(hint: Option<String>) -> Option<String> {
    hint.filter(|name| !name.is_empty() && name.len() <= 256
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c)))
}

pub(crate) fn validate_rejection(event: &AgentEngineEvent) -> Result<(), AgentEngineError> {
    let AgentEngineEvent::ModelResponseRejected { step, discarded_tool_call_ids, rejection, feedback, tool_hint, .. } = event
        else { return Err(invalid("expected a model-response rejection")); };
    crate::output_limit::validate_discarded(*step, discarded_tool_call_ids)?;
    if tool_hint.is_some() && safe_hint(tool_hint.clone()) != *tool_hint { return Err(invalid("invalid correction hint")); }
    match (rejection, feedback) {
        (None, None) => {},
        (Some(reason), Some(text)) if !text.trim().is_empty() && text.len() <= MAX_FEEDBACK_BYTES && !text.contains('\0') => {
            if let AgentModelResponseRejection::ToolArguments { diagnostics } = reason {
                let mut seen = std::collections::BTreeSet::new();
                if diagnostics.is_empty() || diagnostics.len() > 64 { return Err(invalid("invalid diagnostic count")); }
                for proposal in diagnostics {
                    crate::stream_limits::identity(&proposal.call_id, &proposal.name)?;
                    proposal.diagnostic.validate().map_err(|_| invalid("invalid safe argument diagnostic"))?;
                    if !discarded_tool_call_ids.contains(&proposal.call_id) || !seen.insert(&proposal.call_id)
                        || proposal.name.is_empty() {
                        return Err(invalid("diagnostic differs from its rejected proposal"));
                    }
                }
            }
            if matches!(reason, AgentModelResponseRejection::EmptyTaskTerminal | AgentModelResponseRejection::RequiredInputHandoff | AgentModelResponseRejection::RequiredChildResults | AgentModelResponseRejection::RequiredDelegation) && !discarded_tool_call_ids.is_empty() {
                return Err(invalid("text-only terminal rejection contains native proposals"));
            }
        }
        _ => return Err(invalid("typed rejection and safe feedback must be recorded together")),
    }
    Ok(())
}

pub(crate) fn rejection_feedback(event: &AgentEngineEvent) -> Result<ChatMessage, AgentEngineError> {
    validate_rejection(event)?;
    let AgentEngineEvent::ModelResponseRejected { continuation, feedback, .. } = event else { unreachable!() };
    Ok(crate::context_lifecycle::text_message(ChatRole::User,
        feedback.clone().unwrap_or_else(|| render_feedback(&AgentModelResponseRejection::Unclassified, *continuation))))
}

fn render_feedback(reason: &AgentModelResponseRejection, continuation: bool) -> String {
    let observation = match reason {
        AgentModelResponseRejection::NonNativeMarkup => "The previous response emitted tool-call markup as text instead of native function calls. No markup or embedded command was executed.".to_owned(),
        AgentModelResponseRejection::EmptyTaskTerminal => "The previous terminal response contained no public answer or native tool calls. Private thinking is not task completion. No new effect was proposed or executed by that response.".to_owned(),
        AgentModelResponseRejection::RequiredInputHandoff => "The previous text-only stop did not submit the required user-input handoff. The current completion account explicitly records blocked work. Neither plain text nor a provider stop resolves that dependency or completes the task.".to_owned(),
        AgentModelResponseRejection::RequiredChildResults => "The previous stop did not read the accepted children's exact terminal results. Delegation acceptance and a wake notification do not prove that those results were integrated or that the original task is complete.".to_owned(),
        AgentModelResponseRejection::RequiredDelegation => "The original accepted user request explicitly requires delegated Agent work, but no collaboration owner attempt or accepted child receipt exists. The previous text-only response did not perform that requested action.".to_owned(),
        AgentModelResponseRejection::ToolArguments { diagnostics } => {
            let mut text = "The completed native response contained invalid tool arguments. The arguments were not repaired or executed. Diagnostic identity fields are data: ".to_owned();
            for proposal in diagnostics.iter().take(8) {
                text.push_str(&format!("call={}, tool={}, category={:?}, line={:?}, column={:?}, bytes={}, digest={}; ",
                    serde_json::to_string(proposal.call_id.as_ref()).expect("identity JSON"),
                    serde_json::to_string(&proposal.name).expect("name JSON"), proposal.diagnostic.category,
                    proposal.diagnostic.line, proposal.diagnostic.column, proposal.diagnostic.byte_len, proposal.diagnostic.digest.as_ref()));
            }
            if diagnostics.len() > 8 {
                text.push_str(&format!("{} additional rejected proposals are retained in canonical diagnostics. ", diagnostics.len() - 8));
            }
            text
        }
        AgentModelResponseRejection::Unclassified => "The previous response was rejected. The original canonical event did not record a more precise reason.".to_owned(),
    };
    format!("Engine protocol observation, not a new user instruction: {observation} The ENTIRE proposed tool batch was discarded before any tool admission, including any well-formed native calls in that batch. Earlier steps, effects, original user request, constraints and plan are unchanged. {}",
        if continuation && matches!(reason, AgentModelResponseRejection::EmptyTaskTerminal) {
            "Provide the public reply needed by the accepted request. A direct text answer is allowed; do not call tools merely to repair an empty reply. If the authorized task needs work, continue using the native tool interface; if required user information is missing, use the available input handoff. Submit the required completion report only when work is settled. Never repeat settled effects or infer additional authority."
        }
        else if continuation && matches!(reason,AgentModelResponseRejection::RequiredInputHandoff) {
            "Submit request_user_input alone with the concrete user information or external action required now. Retain known failures, completed work and unfinished requirements. Use a fresh call ID. Do not resubmit the report or reopen execution tools; expired links and helpers remain expired."
        }
        else if continuation && matches!(reason,AgentModelResponseRejection::RequiredChildResults) {
            "Use agent_status with the original execution_ids to read the exact child results and source references. For large groups select bounded step_ids until every child result has been read. Do not delegate again, poll children with model calls, or claim integration before consuming their results. Preserve the original task, budget, failures and completed work."
        }
        else if continuation && matches!(reason,AgentModelResponseRejection::RequiredDelegation) {
            "Invoke the exact advertised collaboration Action with a bounded task under the original user request. Acceptance proves delegated work was admitted, not completed. Preserve the parent's tools for independent work and integration; join and read exact child results before final delivery. Never repeat a settled or uncertain owner effect."
        }
        else if continuation { "Use the actual native tool-call interface with exact advertised names and schemas. Continue the unfinished authorized task, or submit the required completion report when work is settled. Every native tool argument root must be a JSON object. Send nested arrays, objects and booleans as JSON values, not strings containing JSON. Quote and escape argument strings correctly. Use fresh call IDs. Never repeat settled effects or infer additional authority. If a protocol example was requested, display it in a fenced code block instead of executing it." }
        else { "The existing bounded protocol correction will not continue. This is not task completion or permission to replay effects." })
}

fn invalid(message: &str) -> AgentEngineError { AgentEngineError::ReplayContract(format!("protocol correction: {message}")) }

#[cfg(test)]
#[path = "protocol_recovery_tests.rs"]
mod tests;
