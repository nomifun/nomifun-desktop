//! Host-owned completion phases and the bounded candidate delivery review.
//! This is native checkpoint state, never evidence or execution authority.
use nomifun_chat_model_broker::ChatMessage;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentExecutionPhase {
    #[default]
    Execution,
    CompletionReview,
    DeliveryReview,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCompletionReviewState {
    pub phase: AgentExecutionPhase,
    pub delivery_pending: bool,
    pub delivery_used: bool,
    pub input_revision: usize,
    pub account_repair: bool,
}

impl AgentCompletionReviewState {
    pub(crate) fn align_inputs(&mut self, revision: usize) {
        if self.input_revision != revision {
            *self = Self { input_revision: revision, ..Default::default() };
        }
    }

    pub(crate) fn begin(&mut self, inputs: &[ChatMessage], observations: usize) -> bool {
        self.align_inputs(inputs.len());
        if self.delivery_used || observations < 3 || !crate::delivery_review::multi_item_task(inputs) { return false; }
        self.delivery_used = true;
        self.delivery_pending = true;
        self.phase = AgentExecutionPhase::DeliveryReview;
        true
    }

    pub(crate) fn is_report_only(&self) -> bool {
        self.phase != AgentExecutionPhase::Execution
    }

    pub(crate) fn valid_for(&self, inputs: usize) -> bool {
        (!self.delivery_pending || self.delivery_used) && !(self.delivery_pending && self.account_repair)
            && self.input_revision <= inputs
            && (!(self.delivery_used || self.account_repair || self.is_report_only()) || self.input_revision > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_review_requires_an_explicit_phase_and_new_input_releases_it() {
        let mut state = AgentCompletionReviewState { phase: AgentExecutionPhase::CompletionReview, input_revision: 1, ..Default::default() };
        assert!(state.valid_for(1));
        let mut encoded = serde_json::to_value(&state).unwrap();
        encoded.as_object_mut().unwrap().remove("phase");
        assert!(serde_json::from_value::<AgentCompletionReviewState>(encoded).is_err(), "missing phase must not reopen execution");
        state.align_inputs(2);
        assert_eq!(state.phase, AgentExecutionPhase::Execution);
    }
}
