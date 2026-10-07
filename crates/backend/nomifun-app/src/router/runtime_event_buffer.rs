//! Bounded persistence projection. UI deltas remain live; replay keeps display
//! text, thinking, and completed calls without every provider fragment.
use nomifun_chat_model_broker::{ChatContentPart, ChatToolCall, ToolCallId};
use nomifun_agent_runtime::{AgentCompactedItem, AgentEngineEvent, AgentToolResult};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const FLUSH_BYTES: usize = 16 * 1024;
const MAX_THINKING_BYTES_PER_TURN: usize = 512 * 1024;

#[derive(Default)]
pub(super) struct AgentEventBuffer {
    text: Option<(u16, String)>,
    thinking: Option<(u16, String)>,
    thinking_bytes: usize,
    active_thinking_step: Option<u16>,
    instruction_reads: BTreeSet<ToolCallId>,
    proposal_ids: BTreeSet<ToolCallId>,
}

impl AgentEventBuffer {
    pub(super) fn project(&mut self, event: &AgentEngineEvent) -> Vec<AgentEngineEvent> {
        let mut records = Vec::new();
        let starts_thinking = matches!(event.reasoning_display_transition(), Some(Some(step))
            if self.active_thinking_step != Some(step));
        let closes_thinking = self.active_thinking_step.is_some()
            && event.reasoning_display_transition() == Some(None);
        if let Some(next) = event.reasoning_display_transition() {
            self.active_thinking_step = next;
        }
        match event {
            AgentEngineEvent::ContextCompacted {
                retained_context: Some(items),
                ..
            } => {
                self.flush(&mut records);
                let mut projected = event.clone();
                if let AgentEngineEvent::ContextCompacted {
                    retained_context: Some(context),
                    ..
                } = &mut projected
                {
                    *context = bounded_context(items);
                }
                records.push(projected);
            }
            AgentEngineEvent::ModelStepStarted { .. } => {
                self.proposal_ids.clear();
                self.flush(&mut records);
                records.push(event.clone());
            }
            AgentEngineEvent::ModelOutputTruncated { discarded_tool_call_ids, .. }
            | AgentEngineEvent::ModelResponseRejected { discarded_tool_call_ids, .. } => {
                for id in discarded_tool_call_ids { self.instruction_reads.remove(id); }
                self.flush(&mut records);
                records.push(event.clone());
            }
            AgentEngineEvent::ToolCallDelta {
                step,
                call_id,
                name,
                ..
            } if *step > 0 => {
                // Truncated calls may never reach ToolCallCompleted. Keep
                // one identity-only proposal fact, not private/partial JSON,
                // so replay can corroborate ModelOutputTruncated exactly.
                let new_proposal = self.proposal_ids.len() < 64 && self.proposal_ids.insert(call_id.clone());
                if closes_thinking || new_proposal {
                    self.flush(&mut records);
                    records.push(AgentEngineEvent::ToolCallDelta {
                        step: *step,
                        call_id: call_id.clone(),
                        name: name.clone(),
                        arguments_delta: String::new(),
                    });
                }
            }
            AgentEngineEvent::ToolCallCompleted { call, .. } if is_instruction_read(call) => {
                self.instruction_reads.insert(call.call_id.clone());
                self.flush(&mut records);
                records.push(event.clone());
            }
            AgentEngineEvent::ToolCompleted { step, result }
                if self.instruction_reads.remove(&result.call_id) =>
            {
                self.flush(&mut records);
                records.push(AgentEngineEvent::ToolCompleted {
                    step: *step,
                    result: instruction_result(result),
                });
            }
            AgentEngineEvent::ToolCompleted { step, result } => {
                self.flush(&mut records);
                records.push(AgentEngineEvent::ToolCompleted {
                    step: *step,
                    result: bounded_result(result),
                });
            }
            AgentEngineEvent::InstructionsUpdated { context } => {
                self.flush(&mut records);
                records.push(AgentEngineEvent::InstructionsUpdated {
                    context: digest_notice(context),
                });
            }
            AgentEngineEvent::OutputTextDelta { step, text } => {
                if self.thinking.is_some()
                    || self.text.as_ref().is_some_and(|(previous, _)| previous != step)
                {
                    self.flush(&mut records);
                }
                let (_, pending) = self.text.get_or_insert_with(|| (*step, String::new()));
                pending.push_str(text);
                if closes_thinking || pending.len() >= FLUSH_BYTES {
                    self.flush(&mut records);
                }
            }
            AgentEngineEvent::ReasoningDelta { step, text } => {
                if self.text.is_some()
                    || self.thinking.as_ref().is_some_and(|(previous, _)| previous != step)
                {
                    self.flush(&mut records);
                }
                let remaining = MAX_THINKING_BYTES_PER_TURN.saturating_sub(self.thinking_bytes);
                let mut end = text.len().min(remaining);
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                if end > 0 {
                    let (_, pending) = self.thinking.get_or_insert_with(|| (*step, String::new()));
                    pending.push_str(&text[..end]);
                    self.thinking_bytes += end;
                    // Commit the first fragment of a phase before publishing
                    // it live, so a cold history read can observe the same
                    // active step. Later fragments retain bounded buffering.
                    if starts_thinking || pending.len() >= FLUSH_BYTES {
                        self.flush(&mut records);
                    }
                } else if starts_thinking {
                    // The bounded body budget must not hide a real phase
                    // transition for a step that already has visible content.
                    records.push(AgentEngineEvent::ReasoningDelta { step: *step, text: String::new() });
                }
            }
            // Partial tool argument fragments remain transient; completed calls
            // carry the validated arguments needed by runtime replay.
            AgentEngineEvent::ToolCallDelta { .. } => {}
            _ => {
                self.flush(&mut records);
                records.push(event.clone());
            }
        }
        records
    }

    pub(super) fn flush(&mut self, records: &mut Vec<AgentEngineEvent>) {
        if let Some((step, text)) = self.text.take() {
            records.push(AgentEngineEvent::OutputTextDelta { step, text });
        }
        if let Some((step, text)) = self.thinking.take() {
            records.push(AgentEngineEvent::ReasoningDelta { step, text });
        }
    }
}

pub(super) fn is_instruction_read(call: &ChatToolCall) -> bool {
    call.call_id.as_ref().starts_with("agent-instructions:")
        || (call.name == "read_file"
            && call
                .arguments
                .0
                .get("path")
                .and_then(|v| v.as_str())
                .is_some_and(|path| {
                    path.rsplit(['/', '\\']).next().is_some_and(|name| {
                        name.eq_ignore_ascii_case("AGENTS.md")
                            || name.eq_ignore_ascii_case("AGENTS.override.md")
                    })
                }))
}

/// Compaction copies must obey the same durable observation policy as the
/// original ToolCompleted records, including results from preceding turns.
fn bounded_context(items: &[AgentCompactedItem]) -> Vec<AgentCompactedItem> {
    let instructions = items
        .iter()
        .filter_map(|item| match item {
            AgentCompactedItem::Message { message } => Some(message),
            _ => None,
        })
        .flat_map(|message| &message.content)
        .filter_map(|part| {
            let ChatContentPart::ToolCall {
                call_id,
                name,
                arguments,
                ..
            } = part
            else {
                return None;
            };
            let call = ChatToolCall {
                call_id: call_id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
                provider_metadata: None,
            };
            is_instruction_read(&call).then(|| call_id.clone())
        })
        .collect::<BTreeSet<_>>();
    let mut projected = items.to_vec();
    for item in &mut projected {
        let AgentCompactedItem::Message { message } = item else {
            continue;
        };
        for part in &mut message.content {
            if let ChatContentPart::ToolResult {
                call_id,
                output,
                is_error,
            } = part
            {
                let result = AgentToolResult {
                    call_id: call_id.clone(),
                    output: output.clone(),
                    is_error: *is_error,
                };
                let result = if instructions.contains(call_id) {
                    instruction_result(&result)
                } else {
                    bounded_result(&result)
                };
                *output = result.output;
            }
        }
    }
    projected
}

fn digest_notice(text: &str) -> String {
    format!(
        "Repository instruction body omitted from durable history ({} UTF-8 bytes, sha256:{:x}). Re-read current rules through the authorized workspace tool; this digest grants no authority.",
        text.len(),
        Sha256::digest(text.as_bytes())
    )
}

pub(super) fn instruction_result(
    result: &nomifun_agent_runtime::AgentToolResult,
) -> nomifun_agent_runtime::AgentToolResult {
    nomifun_agent_runtime::AgentToolResult::text(
        result.call_id.clone(),
        digest_notice(&result.output_text()),
        result.is_error,
    )
}

/// A bounded replay observation, not a substitute for the live tool result.
/// Binary tool output must not grow the evidence journal or be re-sent as if
/// the historical image were still available to the model.
pub(super) fn bounded_result(
    result: &nomifun_agent_runtime::AgentToolResult,
) -> nomifun_agent_runtime::AgentToolResult {
    super::engine_tool_host::bounded_engine_tool_result(result)
}

#[cfg(test)]
mod thinking_lifecycle_tests {
    use super::*;

    #[test]
    fn reasoning_phase_boundaries_commit_before_publication_and_keep_intermediate_chunks_buffered() {
        let mut buffer = AgentEventBuffer::default();
        let first = AgentEngineEvent::ReasoningDelta { step: 1, text: "Inspect. ".into() };
        assert_eq!(buffer.project(&first), vec![first]);
        let suffix = AgentEngineEvent::ReasoningDelta { step: 1, text: "Review the details. ".into() };
        assert!(buffer.project(&suffix).is_empty());
        let handoff = AgentEngineEvent::OutputTextDelta { step: 1, text: "Reading the file.".into() };
        assert_eq!(buffer.project(&handoff), vec![suffix, handoff]);
        let resumed = AgentEngineEvent::ReasoningDelta { step: 1, text: "Verify. ".into() };
        assert_eq!(buffer.project(&resumed), vec![resumed]);
        let suffix = AgentEngineEvent::ReasoningDelta { step: 1, text: "Check the result.".into() };
        assert!(buffer.project(&suffix).is_empty());
        let next = AgentEngineEvent::ModelStepStarted { step: 2, operation_id: "model:2".into() };
        assert_eq!(buffer.project(&next), vec![suffix, next]);
        let reasoning = AgentEngineEvent::ReasoningDelta { step: 2, text: "Next step.".into() };
        assert_eq!(buffer.project(&reasoning), vec![reasoning]);
        let mut tail = Vec::new();
        buffer.flush(&mut tail);
        assert!(tail.is_empty(), "boundary fragments are never written twice");
    }

    #[test]
    fn an_exhausted_reasoning_body_budget_still_records_a_reopened_phase() {
        let mut buffer = AgentEventBuffer::default();
        let first = AgentEngineEvent::ReasoningDelta { step: 1, text: "x".repeat(MAX_THINKING_BYTES_PER_TURN) };
        assert_eq!(buffer.project(&first), vec![first]);
        let handoff = AgentEngineEvent::OutputTextDelta { step: 1, text: "Reading.".into() };
        assert_eq!(buffer.project(&handoff), vec![handoff]);
        let resumed = AgentEngineEvent::ReasoningDelta { step: 1, text: "Visible live suffix".into() };
        assert_eq!(buffer.project(&resumed), vec![AgentEngineEvent::ReasoningDelta { step: 1, text: String::new() }]);
        assert_eq!(buffer.thinking_bytes, MAX_THINKING_BYTES_PER_TURN);
        assert!(buffer.project(&resumed).is_empty(), "budget exhaustion does not write per-delta lifecycle records");
    }

    #[test]
    fn repeated_tool_argument_fragments_still_commit_a_reasoning_handoff() {
        let mut buffer = AgentEventBuffer::default();
        let call = AgentEngineEvent::ToolCallDelta { step: 1, call_id: "call-a".into(), name: "read_file".into(), arguments_delta: "PRIVATE_PARTIAL_ARGUMENTS".into() };
        let mut expected = call.clone();
        if let AgentEngineEvent::ToolCallDelta { arguments_delta, .. } = &mut expected { arguments_delta.clear(); }
        assert_eq!(buffer.project(&call), vec![expected.clone()]);
        assert_eq!(buffer.project(&AgentEngineEvent::ReasoningDelta { step: 1, text: "Inspect. ".into() }).len(), 1);
        let suffix = AgentEngineEvent::ReasoningDelta { step: 1, text: "Check. ".into() };
        assert!(buffer.project(&suffix).is_empty());
        assert_eq!(buffer.project(&call), vec![suffix, expected.clone()]);
        // The proposal bound also cannot swallow a real phase boundary.
        buffer.proposal_ids = (0..64).map(|index| ToolCallId::from(format!("call-{index}"))).collect();
        assert_eq!(buffer.project(&AgentEngineEvent::ReasoningDelta { step: 1, text: "Review again.".into() }).len(), 1);
        assert_eq!(buffer.project(&call), vec![expected]);
        assert!(buffer.project(&call).is_empty(), "ordinary repeated argument fragments remain transient");
    }
}
