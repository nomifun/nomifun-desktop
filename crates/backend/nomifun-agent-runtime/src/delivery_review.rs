//! One bounded publication review for explicit multi-item action tasks.
//! This is model interpretation, never semantic proof or execution authority.
use nomifun_chat_model_broker::{ChatContentPart, ChatMessage, ChatRole};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDeliveryReviewState {
    pub pending: bool,
    pub used: bool,
    pub input_revision: usize,
    #[serde(default)]
    pub account_repair: bool,
}

impl AgentDeliveryReviewState {
    pub(crate) fn align_inputs(&mut self, revision: usize) {
        if self.input_revision != revision {
            *self = Self { input_revision: revision, ..Default::default() };
        }
    }

    pub(crate) fn begin(&mut self, inputs: &[ChatMessage], observations: usize) -> bool {
        self.align_inputs(inputs.len());
        if self.used || observations < 3 || !inputs.iter().any(multi_item_input) { return false; }
        self.used = true;
        self.pending = true;
        true
    }

    pub(crate) fn valid_for(&self, inputs: usize) -> bool {
        (!self.pending || self.used) && !(self.pending && self.account_repair)
            && self.input_revision <= inputs
            && (!(self.used || self.account_repair) || self.input_revision > 0)
    }
}

fn multi_item_input(input: &ChatMessage) -> bool {
    if input.role != ChatRole::User { return false; }
    input.content.iter().any(|part| {
        let ChatContentPart::Text { text } = part else { return false; };
        let mut fenced = false;
        let mut numbers = std::collections::BTreeSet::new();
        for line in text.lines().map(str::trim_start) {
            if line.starts_with("```") || line.starts_with("~~~") { fenced = !fenced; continue; }
            if fenced { continue; }
            let end = line.find(|c: char| !c.is_ascii_digit()).unwrap_or(line.len());
            if end == 0 || end > 2 { continue; }
            let suffix = &line[end..];
            if suffix.starts_with(". ") || suffix.starts_with(") ") || suffix.starts_with('、') {
                numbers.insert(&line[..end]);
            }
        }
        numbers.len() >= 2
    })
}

pub(crate) fn multi_item_task(inputs: &[ChatMessage]) -> bool {
    inputs.iter().any(multi_item_input)
}

/// Syntactic delivery slots only. They preserve source text; they do not
/// interpret intent, grant actions, or prove semantic coverage.
pub(crate) fn delivery_slots(inputs: &[ChatMessage]) -> Vec<(String, String)> {
    let mut slots = Vec::new();
    for (input_index, input) in inputs.iter().enumerate() {
        if !multi_item_input(input) { continue; }
        for (part_index, part) in input.content.iter().enumerate() {
            let ChatContentPart::Text { text } = part else { continue; };
            let mut fenced = false;
            for (line_index, line) in text.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("```") || trimmed.starts_with("~~~") { fenced = !fenced; continue; }
                if fenced { continue; }
                let end = trimmed.find(|c: char| !c.is_ascii_digit()).unwrap_or(trimmed.len());
                if end == 0 || end > 2 { continue; }
                let suffix = &trimmed[end..];
                if suffix.starts_with(". ") || suffix.starts_with(") ") || suffix.starts_with('、') {
                    slots.push((format!("input_{input_index}_part_{part_index}_line_{line_index}"), trimmed.to_owned()));
                }
            }
        }
    }
    slots
}

pub(crate) const INSTRUCTION: &str = concat!(
    "A candidate completion account is awaiting its single delivery review; it has NOT been published as task completion. ",
    "Compare the candidate summary with EVERY item and constraint in the complete accepted user inputs and the actual recorded results. ",
    "Where the user requested actual values, deliver those values, not generic statements that an inspection succeeded. ",
    "Distinguish earlier observed facts from later unchecked state without omitting the known earlier facts. ",
    "Preserve exact bytes, operation order, actual exit codes and all recorded failures; successful later calls never erase failures. ",
    "Use the user's language and plain public wording, not internal evidence eligibility, routing, schema or self-instructions. ",
    "Submit the corrected report_completion alone, or the same report if all requested results are already complete. ",
    "This phase exposes ONLY report_completion: do not execute, read, discover, retry or repeat any operation. ",
    "Missing required work stays blocked; do not invent values, omit requirements or reinterpret partial work as success. ",
    "This is one model assessment within the unchanged turn budget, not independent verification or new authority."
);

#[cfg(test)]
mod tests {
    use super::*;

    fn input(text: &str) -> Vec<ChatMessage> {
        vec![crate::context_lifecycle::text_message(ChatRole::User, text.into())]
    }

    #[test]
    fn review_is_once_per_accepted_scope_not_per_repeated_report() {
        let inputs = input("1. Read the actual directory.\n2. Report its exact entries.");
        let mut state = AgentDeliveryReviewState::default();
        assert!(!state.begin(&inputs, 2));
        assert!(state.begin(&inputs, 3));
        assert!(state.pending && state.used && state.valid_for(1));
        assert!(!state.begin(&inputs, 3));
        state.pending = false;
        assert!(!state.begin(&inputs, 4));
        let mut changed = inputs; changed.extend(input("Do not continue the old task."));
        state.align_inputs(changed.len());
        assert!(!state.pending && !state.used);
    }

    #[test]
    fn examples_and_single_actions_do_not_create_a_publication_phase() {
        for text in ["Read one file.", "1. Read one file.", "```text\n1. example\n2. example\n```", "> 1. quote\n> 2. quote"] {
            assert!(!AgentDeliveryReviewState::default().begin(&input(text), 8));
        }
        assert!(AgentDeliveryReviewState::default().begin(&input("1、读取\n2、报告"), 3));
    }
}
