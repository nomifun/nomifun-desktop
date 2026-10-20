//! Bounded persistence projection. UI deltas remain live; replay keeps display
//! text, thinking, and completed calls without every provider fragment.
use nomifun_chat_model_broker::{ChatContentPart, ChatToolCall, ToolCallId};
use nomifun_agent_contracts::ChatOutputTextSource;
use nomifun_agent_runtime::{AgentCompactedItem, AgentEngineEvent, AgentToolResult};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const FLUSH_BYTES: usize = 16 * 1024;
const MAX_THINKING_BYTES_PER_TURN: usize = 512 * 1024;

#[derive(Default)]
pub(super) struct AgentEventBuffer {
    text: Option<(u16, Option<ChatOutputTextSource>, String)>,
    thinking: Option<(u16, String)>,
    thinking_bytes: usize,
    narration_source: Option<(u16, Option<ChatOutputTextSource>)>,
    thinking_step: Option<u16>,
    instruction_reads: BTreeSet<ToolCallId>,
    // Reuse proposal identities through this Turn, including older steps. Late
    // confirmations are physical flushes, never new content boundaries.
    proposal_ids: BTreeMap<u16, BTreeSet<ToolCallId>>,
}

impl AgentEventBuffer {
    /// The owner may commit steering while the same model stream is active.
    /// Its next fragment must expose the new canonical boundary before live
    /// publication, even though the model's text source has not changed.
    pub(super) fn content_boundary(&mut self) {
        self.narration_source = None;
        self.thinking_step = None;
    }

    pub(super) fn project(&mut self, event: &AgentEngineEvent) -> Vec<AgentEngineEvent> {
        let mut records = Vec::new();
        // Body ownership is independent of the physical text/thinking buffers.
        // Interleaved narration cannot manufacture another reasoning body.
        let invocation = match event {
            AgentEngineEvent::ToolCallDelta { step, call_id, .. }
            | AgentEngineEvent::ToolStarted { step, call_id, .. } => Some((*step, call_id)),
            AgentEngineEvent::ToolCallCompleted { step, call } => Some((*step, &call.call_id)),
            _ => None,
        };
        let first_invocation = invocation.is_some_and(|(step, call)| {
            if call.as_ref().starts_with("agent-instructions:") { return false; }
            let seen = self.proposal_ids.entry(step).or_default();
            // Runtime rejects the 65th identity before publishing it. Keep the
            // same bound without evicting earlier identities or older steps.
            seen.len() < 64 && seen.insert(call.clone())
        });
        let transition = if invocation.is_some() && !first_invocation { None }
            else { event.reasoning_display_transition() };
        let closes_content = transition == Some(None);
        if closes_content {
            self.narration_source = None;
        }
        let starts_narration = match event {
            AgentEngineEvent::OutputTextDelta { step, source, text } if !text.is_empty() => {
                let next = (*step, source.clone());
                let starts = self.narration_source.as_ref() != Some(&next);
                self.narration_source = Some(next);
                starts
            }
            _ => false,
        };
        let starts_thinking = matches!(event, AgentEngineEvent::ReasoningDelta { step, .. }
            if self.thinking_step != Some(*step));
        if let Some(next) = transition { self.thinking_step = next; }
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
                self.flush(&mut records);
                records.push(event.clone());
            }
            AgentEngineEvent::ModelOutputTruncated { discarded_tool_call_ids, .. }
            | AgentEngineEvent::ModelResponseRejected { discarded_tool_call_ids, .. }
            | AgentEngineEvent::VoiceModelStepSuperseded {discarded_tool_call_ids,..} => {
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
                if first_invocation {
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
            AgentEngineEvent::ToolCompleted { step, result, runtime_hold }
                if self.instruction_reads.remove(&result.call_id) =>
            {
                self.flush(&mut records);
                records.push(AgentEngineEvent::ToolCompleted {
                    step: *step,
                    result: instruction_result(result),
                    runtime_hold: *runtime_hold,
                });
            }
            AgentEngineEvent::ToolCompleted { step, result, runtime_hold } => {
                self.flush(&mut records);
                records.push(AgentEngineEvent::ToolCompleted {
                    step: *step,
                    result: bounded_result(result),
                    runtime_hold: *runtime_hold,
                });
            }
            AgentEngineEvent::InstructionsUpdated { context } => {
                self.flush(&mut records);
                records.push(AgentEngineEvent::InstructionsUpdated {
                    context: digest_notice(context),
                });
            }
            AgentEngineEvent::OutputTextDelta { step, source, text } => {
                if text.is_empty() { return records; }
                if self.thinking.is_some()
                    || self.text.as_ref().is_some_and(|(previous, previous_source, _)| previous != step || previous_source != source)
                {
                    self.flush(&mut records);
                }
                let (_, _, pending) = self.text.get_or_insert_with(|| (*step, source.clone(), String::new()));
                pending.push_str(text);
                // The first actual fragment owns the canonical narration
                // boundary, including same-step resumes after a real tool or
                // provider block boundary. Reasoning never ends that source.
                // Later fragments keep the normal bounded write cadence.
                if starts_narration || pending.len() >= FLUSH_BYTES {
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
        if let Some((step, source, text)) = self.text.take() {
            records.push(AgentEngineEvent::OutputTextDelta { step, text, source });
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
mod content_lifecycle_tests {
    use super::*;

    #[test]
    fn first_narration_commits_before_publication_without_flushing_each_later_delta() {
        let mut buffer = AgentEventBuffer::default();
        let start = AgentEngineEvent::ModelStepStarted { step: 1, operation_id: "model:1".into(), correction_source: None };
        assert_eq!(buffer.project(&start), vec![start]);
        assert!(buffer.project(&AgentEngineEvent::OutputTextDelta { step: 1, text: String::new(), source: None, }).is_empty());
        let first = AgentEngineEvent::OutputTextDelta { step: 1, text: "开始检查😀。".into(), source: None, };
        assert_eq!(buffer.project(&first), vec![first]);
        let suffix = AgentEngineEvent::OutputTextDelta { step: 1, text: "后续内容。".into(), source: None, };
        assert!(buffer.project(&suffix).is_empty());
        let mut committed = Vec::new();
        buffer.flush(&mut committed);
        assert_eq!(committed, vec![suffix.clone()]);
        assert!(buffer.project(&suffix).is_empty(), "checkpoint flush does not restart the same phase");
    }

    #[test]
    fn narration_reopens_after_typed_model_tool_and_control_boundaries() {
        let boundaries = [
            AgentEngineEvent::ToolCallDelta { step: 1, call_id: "call".into(), name: "read_file".into(), arguments_delta: "PRIVATE_ARGUMENTS".into() },
            AgentEngineEvent::ToolStarted { step: 1, call_id: "call".into(), capability_id: "workspace.files".into(), action_id: "read".into() },
            AgentEngineEvent::ModelStepStarted { step: 2, operation_id: "model:2".into(), correction_source: None },
            AgentEngineEvent::ExecutionResumed { checkpoint_revision: 1, checkpoint_step: 1, model_steps: 1, execution_fence: 2, discarded_tool_call_ids: vec![] },
            AgentEngineEvent::DeliveryReviewSuperseded { step: 1, discarded_tool_call_ids: vec![], continuation: true },
        ];
        for boundary in boundaries {
            let mut buffer = AgentEventBuffer::default();
            buffer.project(&AgentEngineEvent::OutputTextDelta { step: 1, text: "Before.".into(), source: None, });
            let suffix = AgentEngineEvent::OutputTextDelta { step: 1, text: "Pending suffix.".into(), source: None, };
            assert!(buffer.project(&suffix).is_empty());
            let committed = buffer.project(&boundary);
            assert_eq!(committed.first(), Some(&suffix), "{boundary:?}");
            let step = if matches!(boundary, AgentEngineEvent::ModelStepStarted { step: 2, .. }) { 2 } else { 1 };
            let resumed = AgentEngineEvent::OutputTextDelta { step, text: "After.".into(), source: None, };
            assert_eq!(buffer.project(&resumed), vec![resumed], "{boundary:?}");
            assert!(buffer.project(&AgentEngineEvent::OutputTextDelta { step, text: "Still the same phase.".into(), source: None, }).is_empty());
        }
    }

    #[test]
    fn repeated_parameter_fragments_cannot_create_narration_boundaries_or_publish_private_arguments() {
        let mut buffer = AgentEventBuffer::default();
        let proposal = AgentEngineEvent::ToolCallDelta { step: 1, call_id: "same-call".into(), name: "read_file".into(), arguments_delta: "PRIVATE_ARGUMENTS".into() };
        buffer.project(&proposal);
        buffer.project(&AgentEngineEvent::OutputTextDelta { step: 1, text: "Narration.".into(), source: None, });
        let suffix = AgentEngineEvent::OutputTextDelta { step: 1, text: "Buffered suffix.".into(), source: None, };
        assert!(buffer.project(&suffix).is_empty());
        let committed = buffer.project(&proposal);
        assert!(committed.is_empty());
        assert!(!serde_json::to_string(&committed).unwrap().contains("PRIVATE_ARGUMENTS"));
        let resumed = AgentEngineEvent::OutputTextDelta { step: 1, text: "Resumed.".into(), source: None, };
        assert!(buffer.project(&resumed).is_empty());
        let mut pending=Vec::new();buffer.flush(&mut pending);
        assert_eq!(pending,vec![AgentEngineEvent::OutputTextDelta {step:1,text:"Buffered suffix.Resumed.".into(),source:None}]);
    }

    #[test]
    fn long_narration_keeps_exact_text_with_bounded_batches_after_its_first_anchor() {
        let mut buffer = AgentEventBuffer::default();
        let fragment = "x".repeat(1024);
        let mut committed = Vec::new();
        for _ in 0..50 {
            committed.extend(buffer.project(&AgentEngineEvent::OutputTextDelta { step: 1, text: fragment.clone(), source: None, }));
        }
        assert_eq!(committed.len(), 4, "one phase anchor followed by three 16KiB batches");
        buffer.flush(&mut committed);
        let chunks = committed.iter().map(|event| match event {
            AgentEngineEvent::OutputTextDelta { step: 1, text, .. } => text.as_str(),
            _ => panic!("unexpected narration record"),
        }).collect::<Vec<_>>();
        assert_eq!(chunks[0].len(), 1024);
        assert!(chunks.iter().all(|text| text.len() <= FLUSH_BYTES));
        assert_eq!(chunks.concat(), fragment.repeat(50));
        let mut duplicate = Vec::new();
        buffer.flush(&mut duplicate);
        assert!(duplicate.is_empty());
    }

    #[test]
    fn late_results_flush_pending_text_without_reopening_its_canonical_phase() {
        let mut buffer = AgentEventBuffer::default();
        let first = AgentEngineEvent::OutputTextDelta { step: 1, text: "Answer.".into(), source: None, };
        assert_eq!(buffer.project(&first), vec![first]);
        let suffix = AgentEngineEvent::OutputTextDelta { step: 1, text: " More.".into(), source: None, };
        assert!(buffer.project(&suffix).is_empty());
        let result = AgentEngineEvent::ToolCompleted { step: 1, result: AgentToolResult::text("earlier-call".into(), "done", false), runtime_hold: None };
        let committed = buffer.project(&result);
        assert_eq!(committed[0], suffix);
        assert_eq!(committed.len(), 2);
        assert!(buffer.project(&AgentEngineEvent::OutputTextDelta { step: 1, text: " Continued.".into(), source: None, }).is_empty());
    }

    #[test]
    fn accepted_completion_delivery_remains_one_unchanged_atomic_record() {
        let mut buffer = AgentEventBuffer::default();
        buffer.project(&AgentEngineEvent::OutputTextDelta { step: 1, text: "Progress.".into(), source: None, });
        let suffix = AgentEngineEvent::OutputTextDelta { step: 1, text: " Buffered progress.".into(), source: None, };
        assert!(buffer.project(&suffix).is_empty());
        let delivery = AgentEngineEvent::CompletionDelivered { step: 1, text: "完整的已验收结果😀\n\n保留原始交付内容。".into() };
        assert_eq!(buffer.project(&delivery), vec![suffix, delivery]);
        let mut tail = Vec::new();
        buffer.flush(&mut tail);
        assert!(tail.is_empty(), "an accepted delivery is not split or written twice");
    }

    #[test]
    fn bounded_tool_results_preserve_runtime_hold_without_classifying_owner_errors() {
        for runtime_hold in [None, Some(nomifun_agent_runtime::RuntimeToolHoldReason::ArgumentsRejected)] {
            let event = AgentEngineEvent::ToolCompleted { step: 1, runtime_hold,
                result: nomifun_agent_runtime::AgentToolResult::text("call".into(), "x".repeat(100_000), true) };
            let records = AgentEventBuffer::default().project(&event);
            assert_eq!(records.len(), 1);
            let AgentEngineEvent::ToolCompleted { runtime_hold: recorded, result, .. } = &records[0] else { panic!("missing tool result") };
            assert_eq!(*recorded, runtime_hold);
            assert!(result.output_text().len() < 100_000);
            assert!(result.is_error);
        }
    }

    #[test]
    fn independent_reasoning_body_keeps_interleaved_suffixes_buffered_until_a_real_boundary() {
        let mut buffer = AgentEventBuffer::default();
        let first = AgentEngineEvent::ReasoningDelta { step: 1, text: "Inspect. ".into() };
        assert_eq!(buffer.project(&first), vec![first]);
        let suffix = AgentEngineEvent::ReasoningDelta { step: 1, text: "Review the details. ".into() };
        assert!(buffer.project(&suffix).is_empty());
        let handoff = AgentEngineEvent::OutputTextDelta { step: 1, text: "Reading the file.".into(), source: None, };
        assert_eq!(buffer.project(&handoff), vec![suffix, handoff]);
        let resumed = AgentEngineEvent::ReasoningDelta { step: 1, text: "Verify. ".into() };
        assert!(buffer.project(&resumed).is_empty(), "narration did not end this reasoning body");
        let suffix = AgentEngineEvent::ReasoningDelta { step: 1, text: "Check the result.".into() };
        assert!(buffer.project(&suffix).is_empty());
        let next = AgentEngineEvent::ModelStepStarted { step: 2, operation_id: "model:2".into(), correction_source: None };
        let combined = AgentEngineEvent::ReasoningDelta { step: 1, text: "Verify. Check the result.".into() };
        assert_eq!(buffer.project(&next), vec![combined, next]);
        let reasoning = AgentEngineEvent::ReasoningDelta { step: 2, text: "Next step.".into() };
        assert_eq!(buffer.project(&reasoning), vec![reasoning]);
        let mut tail = Vec::new();
        buffer.flush(&mut tail);
        assert!(tail.is_empty(), "boundary fragments are never written twice");
    }

    #[test]
    fn reasoning_does_not_reopen_narration_or_change_its_provider_source() {
        let source = ChatOutputTextSource { item_id: "message-one".into(), output_index: Some(1), content_index: 0 };
        let mut buffer = AgentEventBuffer::default();
        let first = AgentEngineEvent::OutputTextDelta { step: 1, text: "CL".into(), source: Some(source.clone()) };
        assert_eq!(buffer.project(&first), vec![first]);
        let thought = AgentEngineEvent::ReasoningDelta { step: 1, text: "Verify installation.".into() };
        assert_eq!(buffer.project(&thought), vec![thought]);
        let suffix = AgentEngineEvent::OutputTextDelta { step: 1, text: "I 和 SKILL 都已安装成功。".into(), source: Some(source) };
        assert!(buffer.project(&suffix).is_empty(), "a reasoning observation is not a new text block");
        let mut tail = Vec::new();
        buffer.flush(&mut tail);
        assert_eq!(tail, vec![suffix]);
    }

    #[test]
    fn provider_text_blocks_commit_their_own_anchors_and_never_share_a_batch() {
        let mut buffer = AgentEventBuffer::default();
        let mut committed = Vec::new();
        for (item_id, content_index, text) in [("one",0,"First"),("one",0," tail"),("one",1,"Second"),("two",0,"Third")] {
            committed.extend(buffer.project(&AgentEngineEvent::OutputTextDelta { step: 1, text: text.into(),
                source: Some(ChatOutputTextSource { item_id: item_id.into(), output_index: None, content_index }) }));
        }
        buffer.flush(&mut committed);
        assert_eq!(committed.len(), 4);
        for (event, (id, index, text)) in committed.iter().zip([("one",0,"First"),("one",0," tail"),("one",1,"Second"),("two",0,"Third")]) {
            let AgentEngineEvent::OutputTextDelta { source: Some(source), text: body, .. } = event else { panic!("source lost") };
            assert_eq!((&source.item_id,source.content_index,body.as_str()), (&id.to_owned(),index,text));
        }
    }

    #[test]
    fn owner_steering_commits_the_next_fragment_even_with_an_unchanged_text_source() {
        let mut buffer = AgentEventBuffer::default();
        buffer.project(&AgentEngineEvent::OutputTextDelta { step: 1, text: "Before".into(), source: None });
        assert!(buffer.project(&AgentEngineEvent::OutputTextDelta { step: 1, text: " pending".into(), source: None }).is_empty());
        buffer.content_boundary();
        assert_eq!(buffer.project(&AgentEngineEvent::OutputTextDelta { step: 1, text: " after".into(), source: None }),
            vec![AgentEngineEvent::OutputTextDelta { step: 1, text: " pending after".into(), source: None }]);
    }

    #[test]
    fn exhausted_reasoning_budget_preserves_only_real_new_body_anchors() {
        let mut buffer = AgentEventBuffer::default();
        let first = AgentEngineEvent::ReasoningDelta { step: 1, text: "x".repeat(MAX_THINKING_BYTES_PER_TURN) };
        assert_eq!(buffer.project(&first), vec![first]);
        let handoff = AgentEngineEvent::OutputTextDelta { step: 1, text: "Reading.".into(), source: None, };
        assert_eq!(buffer.project(&handoff), vec![handoff]);
        let resumed = AgentEngineEvent::ReasoningDelta { step: 1, text: "Visible live suffix".into() };
        assert!(buffer.project(&resumed).is_empty(), "interleaved narration cannot open an empty reasoning body");
        let proposal = AgentEngineEvent::ToolCallDelta { step: 1, call_id: "call".into(), name: "read_file".into(), arguments_delta: String::new() };
        assert_eq!(buffer.project(&proposal), vec![proposal]);
        assert_eq!(buffer.project(&resumed), vec![AgentEngineEvent::ReasoningDelta { step: 1, text: String::new() }]);
        assert_eq!(buffer.thinking_bytes, MAX_THINKING_BYTES_PER_TURN);
        assert!(buffer.project(&resumed).is_empty(), "budget exhaustion does not write per-delta lifecycle records");
    }

    #[test]
    fn content_boundary_preserves_pending_bytes_and_anchors_the_next_reasoning_fragment() {
        let mut buffer = AgentEventBuffer::default();
        buffer.project(&AgentEngineEvent::ReasoningDelta { step: 1, text: "First".into() });
        assert!(buffer.project(&AgentEngineEvent::ReasoningDelta { step: 1, text: " pending".into() }).is_empty());
        buffer.content_boundary();
        assert_eq!(buffer.project(&AgentEngineEvent::ReasoningDelta { step: 1, text: " after steering".into() }),
            vec![AgentEngineEvent::ReasoningDelta { step: 1, text: " pending after steering".into() }]);
        let mut remaining = Vec::new(); buffer.flush(&mut remaining);
        assert!(remaining.is_empty());
    }

    #[test]
    fn interleaved_reasoning_keeps_the_original_byte_flush_and_turn_budget() {
        let mut buffer = AgentEventBuffer::default();
        buffer.project(&AgentEngineEvent::ReasoningDelta { step: 1, text: "The".into() });
        buffer.project(&AgentEngineEvent::OutputTextDelta { step: 1, text: "好的".into(), source: None });
        assert!(buffer.project(&AgentEngineEvent::ReasoningDelta { step: 1, text: "x".repeat(FLUSH_BYTES - 1) }).is_empty());
        assert_eq!(buffer.project(&AgentEngineEvent::ReasoningDelta { step: 1, text: ".".into() }),
            vec![AgentEngineEvent::ReasoningDelta { step: 1, text: format!("{}.", "x".repeat(FLUSH_BYTES - 1)) }]);
        let retained = MAX_THINKING_BYTES_PER_TURN - buffer.thinking_bytes;
        let events = buffer.project(&AgentEngineEvent::ReasoningDelta { step: 1, text: "界".repeat(retained) });
        let body = events.iter().filter_map(|event| match event { AgentEngineEvent::ReasoningDelta { text, .. } => Some(text.as_str()), _ => None }).collect::<String>();
        assert!(body.len() <= retained && retained - body.len() < 3);
        assert!(buffer.thinking_bytes <= MAX_THINKING_BYTES_PER_TURN);
    }

    #[test]
    fn late_results_and_duplicate_starts_only_flush_existing_reasoning_bytes() {
        let mut buffer = AgentEventBuffer::default();
        let start = AgentEngineEvent::ToolStarted { step: 1, call_id: "old".into(),
            capability_id: "workspace.files".into(), action_id: "workspace.files/read".into() };
        let mut records = buffer.project(&start);
        records.extend(buffer.project(&AgentEngineEvent::ReasoningDelta { step: 1, text: "Let me".into() }));
        assert!(buffer.project(&AgentEngineEvent::ReasoningDelta { step: 1, text: " make the call".into() }).is_empty());
        records.extend(buffer.project(&AgentEngineEvent::ToolCompleted { step: 1, runtime_hold: None,
            result: AgentToolResult::text("old".into(), "done", false) }));
        assert!(buffer.project(&AgentEngineEvent::ReasoningDelta { step: 1, text: " now".into() }).is_empty(),
            "late result did not reset reasoning ownership");
        records.extend(buffer.project(&start));
        records.extend(buffer.project(&AgentEngineEvent::ReasoningDelta { step: 1, text: ".".into() }));
        buffer.flush(&mut records);
        let body = records.iter().filter_map(|event| match event { AgentEngineEvent::ReasoningDelta { text, .. } => Some(text.as_str()), _ => None }).collect::<String>();
        assert_eq!(body,"Let me make the call now.");
        assert_eq!(records.iter().filter(|event| matches!(event, AgentEngineEvent::ToolStarted { .. })).count(),2);
        assert_eq!(buffer.thinking_step,Some(1),"duplicate admission did not close the current reasoning body");
    }

    #[test]
    fn one_invocation_has_one_boundary_across_fragments_completion_admission_and_old_steps() {
        let mut buffer = AgentEventBuffer::default();
        let internal=AgentEngineEvent::ToolCallCompleted {step:0,call:ChatToolCall {call_id:"agent-instructions:1".into(),name:"read_file".into(),
            arguments:nomifun_agent_contracts::StrictJsonValue(serde_json::json!({"path":"AGENTS.md"})),provider_metadata:None}};
        buffer.project(&AgentEngineEvent::ReasoningDelta {step:1,text:"Current reasoning.".into()});
        buffer.project(&internal);
        assert!(buffer.proposal_ids.is_empty(),"reserved host reads do not consume model proposal slots");
        assert_eq!(buffer.thinking_step,Some(1),"internal instruction observations do not own displayed content");
        let call = AgentEngineEvent::ToolCallDelta { step: 1, call_id: "call-a".into(), name: "read_file".into(), arguments_delta: "PRIVATE_PARTIAL_ARGUMENTS".into() };
        let mut expected = call.clone();
        if let AgentEngineEvent::ToolCallDelta { arguments_delta, .. } = &mut expected { arguments_delta.clear(); }
        assert_eq!(buffer.project(&call), vec![expected.clone()]);
        assert_eq!(buffer.project(&AgentEngineEvent::ReasoningDelta { step: 1, text: "Inspect. ".into() }).len(), 1);
        let suffix = AgentEngineEvent::ReasoningDelta { step: 1, text: "Check. ".into() };
        assert!(buffer.project(&suffix).is_empty());
        assert!(buffer.project(&call).is_empty());
        let completed=AgentEngineEvent::ToolCallCompleted {step:1,call:ChatToolCall {call_id:"call-a".into(),name:"read_file".into(),
            arguments:nomifun_agent_contracts::StrictJsonValue(serde_json::json!({"path":"file"})),provider_metadata:None}};
        assert_eq!(buffer.project(&completed),vec![suffix,completed.clone()]);
        assert!(buffer.project(&AgentEngineEvent::ReasoningDelta {step:1,text:"Same body.".into()}).is_empty());
        assert_eq!(buffer.thinking_step,Some(1));
        buffer.project(&AgentEngineEvent::ModelStepStarted {step:2,operation_id:"step-2".into(),correction_source:None});
        buffer.project(&AgentEngineEvent::ReasoningDelta {step:2,text:"New step.".into()});
        assert!(buffer.project(&call).is_empty(),"old-step parameter fragments are still recognized");
        buffer.project(&completed);
        assert_eq!(buffer.thinking_step,Some(2),"old-step confirmation cannot reset the current body");
        assert!(buffer.project(&call).is_empty(), "ordinary repeated argument fragments remain transient");
        for index in 1..64 {
            buffer.project(&AgentEngineEvent::ToolCallDelta {step:1,call_id:format!("other-{index}").into(),name:"read_file".into(),arguments_delta:String::new()});
        }
        assert_eq!(buffer.proposal_ids[&1].len(),64);
        buffer.project(&AgentEngineEvent::ReasoningDelta {step:2,text:"Current.".into()});
        let overflow=AgentEngineEvent::ToolCallDelta {step:1,call_id:"rejected-65th".into(),name:"read_file".into(),arguments_delta:String::new()};
        assert!(buffer.project(&overflow).is_empty(),"the Runtime-invalid 65th identity cannot evict a real proposal");
        assert!(buffer.project(&call).is_empty());assert_eq!(buffer.thinking_step,Some(2));
    }
}
