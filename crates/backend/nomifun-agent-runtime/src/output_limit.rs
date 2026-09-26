//! An output ceiling is not transport failure or task completion. Continue
//! only with a new model operation and no execution of the truncated batch.
use crate::AgentEngineError;
use nomifun_chat_model_broker::{
    ChatContentPart, ChatMessage, ChatModelRequest, ChatRole, ToolCallId,
};

// Bound consecutive truncated model steps. A later complete step establishes
// a new safe boundary and must not consume this recovery budget forever.
const MAX_CONTINUATIONS: u8 = 2;

#[derive(Default)]
pub(crate) struct OutputLimitRecovery {
    continued: u8,
}

impl OutputLimitRecovery {
    pub(crate) fn restore<'a>(&mut self, events: impl Iterator<Item = &'a crate::AgentEngineEvent>) {
        let mut previous = None;
        for event in events {
            match event {
                crate::AgentEngineEvent::ExecutionTailReconciled { retry_stall_guards: true, .. } => { self.continued = 0; previous = None; }
                crate::AgentEngineEvent::ModelOutputTruncated { step, continuation: true, .. } => {
                    self.continued = if previous == step.checked_sub(1) { self.continued.saturating_add(1).min(MAX_CONTINUATIONS) } else { 1 };
                    previous = Some(*step);
                }
                crate::AgentEngineEvent::ToolResultsOrdered { .. } => { self.continued = 0; previous = None; }
                _ => {}
            }
        }
    }

    pub(crate) fn admit(&mut self, has_model_step: bool) -> bool {
        if !has_model_step || self.continued >= MAX_CONTINUATIONS {
            return false;
        }
        self.continued += 1;
        true
    }

    pub(crate) fn observe_complete_step(&mut self) {
        self.continued = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_later_truncation_gets_a_fresh_bounded_recovery_window() {
        let mut recovery = OutputLimitRecovery::default();
        assert!(recovery.admit(true));
        assert!(recovery.admit(true));
        assert!(!recovery.admit(true));
        recovery.observe_complete_step();
        assert!(recovery.admit(true));
        assert!(!recovery.admit(false));
    }
}

pub(crate) fn validate_discarded(step: u16, calls: &[ToolCallId]) -> Result<(), AgentEngineError> {
    let mut unique = std::collections::BTreeSet::new();
    if step == 0 || calls.len() > 64 {
        return Err(invalid());
    }
    for id in calls {
        crate::stream_limits::identity(id, "")?;
        if !unique.insert(id) {
            return Err(invalid());
        }
    }
    Ok(())
}

fn invalid() -> AgentEngineError {
    AgentEngineError::InvalidModelEvent("invalid output-limit discard record".into())
}

pub(crate) fn notice(continuation: bool) -> ChatMessage {
    crate::context_lifecycle::text_message(
        ChatRole::User,
        format!(
            "Engine execution observation (not a new user instruction): the preceding model output reached its configured output-token ceiling and is incomplete. No tool call proposed in that model step was executed; that entire proposed batch was discarded, including syntactically complete calls. Earlier steps and their actual effects are unchanged. {}",
            if continuation {
                "Continue from confirmed history, honor the original task and constraints, and keep the next response within the existing output ceiling. Do not replay earlier effects. Regenerate any needed tool call as a smaller complete call with a fresh ID; never append to a discarded partial JSON argument. Do not treat a partial answer or an unfinished plan as completed work."
            } else {
                "No further automatic continuation was admitted. This is not task completion or permission to retry effects."
            }
        ),
    )
}

pub(crate) fn retain_partial_text(request: &mut ChatModelRequest, content: &[ChatContentPart]) {
    // A truncated signature/opaque reasoning chain must not be replayed as
    // valid provider continuation. Only visible text from this step survives.
    let content: Vec<_> = content
        .iter()
        .filter(|part| {
            matches!(part,
        ChatContentPart::Text { text } if !text.is_empty())
        })
        .cloned()
        .collect();
    if !content.is_empty() {
        request.input.messages.push(ChatMessage {
            role: ChatRole::Assistant,
            content,
            provider_round_id: None,
        });
    }
    request.input.provider_round_parent = None;
    for message in &mut request.input.messages {
        message.provider_round_id = None;
    }
}
