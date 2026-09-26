//! Correct model formatting without executing text, replaying effects, or
//! treating malformed output as completion. Every retry is a new model step.
use nomifun_chat_model_broker::{ChatMessage, ChatRole, ChatToolChoice};

#[derive(Default)]
pub(crate) struct ProtocolRecovery {
    consecutive: u8,
    total: u8,
    native_correction_pending: bool,
}

impl ProtocolRecovery {
    pub(crate) fn restore<'a>(&mut self, events: impl Iterator<Item = &'a crate::AgentEngineEvent>) {
        let mut previous = None;
        for event in events {
            match event {
                crate::AgentEngineEvent::ExecutionTailReconciled { retry_stall_guards: true, .. } => {
                    self.total = 0; self.consecutive = 0; previous = None;
                    self.native_correction_pending = false;
                }
                crate::AgentEngineEvent::ModelResponseRejected { step, continuation: true, .. } => {
                    self.total = self.total.saturating_add(1).min(8);
                    self.consecutive = if previous == step.checked_sub(1) { self.consecutive.saturating_add(1).min(2) } else { 1 };
                    previous = Some(*step);
                    self.native_correction_pending = true;
                }
                crate::AgentEngineEvent::ToolResultsOrdered { .. } => {
                    self.observe_valid_step(); previous = None;
                }
                crate::AgentEngineEvent::SteeringInputs { .. } => self.observe_new_input(),
                _ => {}
            }
        }
    }

    pub(crate) fn admit(&mut self, capacity: bool) -> bool {
        if !capacity || self.consecutive >= 2 || self.total >= 8 { return false; }
        self.consecutive += 1;
        self.total += 1;
        self.native_correction_pending = true;
        true
    }

    pub(crate) fn observe_valid_step(&mut self) {
        self.consecutive = 0;
        self.native_correction_pending = false;
    }

    /// A user may replace the intended action while correction is pending.
    /// Release the tool constraint without replenishing the retry budget.
    pub(crate) fn observe_new_input(&mut self) { self.native_correction_pending = false; }

    /// Request native calling during correction, within the advertised tools
    /// and existing retry budget. Some compatible APIs accept `required` but
    /// do not enforce it; the response guard and real tool receipts remain
    /// authoritative. Do not replace an owner's more specific selection.
    pub(crate) fn constrain_tool_choice(&self, choice: &mut ChatToolChoice, has_tools: bool) {
        if self.native_correction_pending && has_tools && !matches!(choice, ChatToolChoice::Specific { .. }) {
            *choice = ChatToolChoice::Required;
        }
    }
}

pub(crate) fn notice(continuation: bool) -> ChatMessage {
    crate::context_lifecycle::text_message(ChatRole::User, format!(
        "Engine protocol observation, not a new user instruction: the previous response emitted tool-call markup as text instead of native function calls. That response is incomplete and its ENTIRE proposed tool batch was discarded before execution, including any well-formed native calls in that batch. No markup or embedded command was executed. Earlier steps, effects, the original user request, constraints and plan are unchanged. {}",
        if continuation {
            "Use the actual native tool-call interface with an exact advertised tool name and schema. Send arrays/objects/booleans as JSON values, not strings containing JSON. Use fresh call IDs. Never repeat earlier executed effects or claim this rejected response completed work. If the user requested a protocol example rather than execution, display it in a fenced code block."
        } else {
            "The bounded protocol-correction budget is exhausted. This is not task completion or permission to retry effects."
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corrections_are_bounded_even_when_interleaved_with_valid_steps() {
        let mut recovery = ProtocolRecovery::default();
        assert!(!recovery.admit(false));
        assert!(recovery.admit(true));
        assert!(recovery.admit(true));
        assert!(!recovery.admit(true));
        for _ in 0..6 { recovery.observe_valid_step(); assert!(recovery.admit(true)); }
        recovery.observe_valid_step();
        assert!(!recovery.admit(true));
    }

    #[test]
    fn native_choice_applies_only_to_pending_correction_with_advertised_tools() {
        let mut recovery = ProtocolRecovery::default();
        let mut choice = ChatToolChoice::Auto;
        recovery.constrain_tool_choice(&mut choice, true);
        assert_eq!(choice, ChatToolChoice::Auto);
        assert!(recovery.admit(true));
        recovery.constrain_tool_choice(&mut choice, true);
        assert_eq!(choice, ChatToolChoice::Required);
        choice = ChatToolChoice::Specific { name: "read_file".into() };
        recovery.constrain_tool_choice(&mut choice, true);
        assert_eq!(choice, ChatToolChoice::Specific { name: "read_file".into() });
        choice = ChatToolChoice::None;
        recovery.constrain_tool_choice(&mut choice, false);
        assert_eq!(choice, ChatToolChoice::None);
        recovery.observe_valid_step();
        choice = ChatToolChoice::Auto;
        recovery.constrain_tool_choice(&mut choice, true);
        assert_eq!(choice, ChatToolChoice::Auto);
    }

    #[test]
    fn pending_protocol_constraint_survives_restore_without_resetting_retry_budget() {
        let rejected = crate::AgentEngineEvent::ModelResponseRejected {
            step: 1, discarded_tool_call_ids: vec![], continuation: true,
        };
        let mut recovery = ProtocolRecovery::default();
        recovery.restore([&rejected].into_iter());
        let mut choice = ChatToolChoice::Auto;
        recovery.constrain_tool_choice(&mut choice, true);
        assert_eq!(choice, ChatToolChoice::Required);
        assert!(recovery.admit(true));
        assert!(!recovery.admit(true));
    }

    #[test]
    fn new_user_input_releases_native_constraint_but_not_the_correction_budget() {
        let mut recovery = ProtocolRecovery::default();
        assert!(recovery.admit(true));
        recovery.observe_new_input();
        let mut choice = ChatToolChoice::Auto;
        recovery.constrain_tool_choice(&mut choice, true);
        assert_eq!(choice, ChatToolChoice::Auto);
        assert!(recovery.admit(true));
        assert!(!recovery.admit(true));
    }
}
