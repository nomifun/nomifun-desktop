//! Host-owned execution and report-only completion phases.
//! This is native checkpoint state, never evidence or execution authority.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentExecutionPhase {
    #[default]
    Execution,
    EndIntentReview,
    CompletionReview,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCompletionReviewState {
    pub phase: AgentExecutionPhase,
    pub input_revision: usize,
}

impl AgentCompletionReviewState {
    pub(crate) fn align_inputs(&mut self, revision: usize) {
        if self.input_revision != revision {
            *self = Self { input_revision: revision, ..Default::default() };
        }
    }

    pub(crate) fn enter_report_only(&mut self, revision: usize) {
        self.align_inputs(revision);
        self.phase = AgentExecutionPhase::CompletionReview;
    }

    pub(crate) fn is_report_only(&self) -> bool {
        self.phase == AgentExecutionPhase::CompletionReview
    }

    pub(crate) fn valid_for(&self, inputs: usize) -> bool {
        self.input_revision <= inputs
            && (self.phase == AgentExecutionPhase::Execution || self.input_revision > 0)
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
