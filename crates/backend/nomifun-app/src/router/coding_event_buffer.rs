//! Bounded persistence projection. UI deltas remain live; replay needs text
//! segments and completed calls, not every provider transport fragment.
use nomifun_chat_model_broker::{ChatContentPart, ChatToolCall, ToolCallId};
use nomifun_coding_engine::{CodingCompactedItem, CodingEngineEvent, CodingToolResult};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const FLUSH_BYTES: usize = 16 * 1024;

#[derive(Default)]
pub(super) struct CodingEventBuffer {
    text: Option<(u16, String)>,
    instruction_reads: BTreeSet<ToolCallId>,
    proposal_ids: BTreeSet<ToolCallId>,
}

impl CodingEventBuffer {
    pub(super) fn project(&mut self, event: &CodingEngineEvent) -> Vec<CodingEngineEvent> {
        let mut records = Vec::new();
        match event {
            CodingEngineEvent::ContextCompacted {
                retained_context: Some(items),
                ..
            } => {
                self.flush(&mut records);
                let mut projected = event.clone();
                if let CodingEngineEvent::ContextCompacted {
                    retained_context: Some(context),
                    ..
                } = &mut projected
                {
                    *context = bounded_context(items);
                }
                records.push(projected);
            }
            CodingEngineEvent::ModelStepStarted { .. } => {
                self.proposal_ids.clear();
                self.flush(&mut records);
                records.push(event.clone());
            }
            CodingEngineEvent::ToolCallDelta {
                step,
                call_id,
                name,
                ..
            } if *step > 0 => {
                // Truncated calls may never reach ToolCallCompleted. Keep
                // one identity-only proposal fact, not private/partial JSON,
                // so replay can corroborate ModelOutputTruncated exactly.
                if self.proposal_ids.len() < 64 && self.proposal_ids.insert(call_id.clone()) {
                    self.flush(&mut records);
                    records.push(CodingEngineEvent::ToolCallDelta {
                        step: *step,
                        call_id: call_id.clone(),
                        name: name.clone(),
                        arguments_delta: String::new(),
                    });
                }
            }
            CodingEngineEvent::ToolCallCompleted { call, .. } if is_instruction_read(call) => {
                self.instruction_reads.insert(call.call_id.clone());
                self.flush(&mut records);
                records.push(event.clone());
            }
            CodingEngineEvent::ToolCompleted { step, result }
                if self.instruction_reads.remove(&result.call_id) =>
            {
                self.flush(&mut records);
                records.push(CodingEngineEvent::ToolCompleted {
                    step: *step,
                    result: instruction_result(result),
                });
            }
            CodingEngineEvent::ToolCompleted { step, result } => {
                self.flush(&mut records);
                records.push(CodingEngineEvent::ToolCompleted {
                    step: *step,
                    result: bounded_result(result),
                });
            }
            CodingEngineEvent::InstructionsUpdated { context } => {
                self.flush(&mut records);
                records.push(CodingEngineEvent::InstructionsUpdated {
                    context: digest_notice(context),
                });
            }
            CodingEngineEvent::OutputTextDelta { step, text } => {
                if self
                    .text
                    .as_ref()
                    .is_some_and(|(previous, _)| previous != step)
                {
                    self.flush(&mut records);
                }
                let (_, pending) = self.text.get_or_insert_with(|| (*step, String::new()));
                pending.push_str(text);
                if pending.len() >= FLUSH_BYTES {
                    self.flush(&mut records);
                }
            }
            // Actual argument fragments and private reasoning stay transient.
            CodingEngineEvent::ToolCallDelta { .. } | CodingEngineEvent::ReasoningDelta { .. } => {}
            _ => {
                self.flush(&mut records);
                records.push(event.clone());
            }
        }
        records
    }

    pub(super) fn flush(&mut self, records: &mut Vec<CodingEngineEvent>) {
        if let Some((step, text)) = self.text.take() {
            records.push(CodingEngineEvent::OutputTextDelta { step, text });
        }
    }
}

pub(super) fn is_instruction_read(call: &ChatToolCall) -> bool {
    call.call_id.as_ref().starts_with("coding-instructions:")
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
fn bounded_context(items: &[CodingCompactedItem]) -> Vec<CodingCompactedItem> {
    let instructions = items
        .iter()
        .filter_map(|item| match item {
            CodingCompactedItem::Message { message } => Some(message),
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
        let CodingCompactedItem::Message { message } = item else {
            continue;
        };
        for part in &mut message.content {
            if let ChatContentPart::ToolResult {
                call_id,
                output,
                is_error,
            } = part
            {
                let result = CodingToolResult {
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
    result: &nomifun_coding_engine::CodingToolResult,
) -> nomifun_coding_engine::CodingToolResult {
    nomifun_coding_engine::CodingToolResult::text(
        result.call_id.clone(),
        digest_notice(&result.output_text()),
        result.is_error,
    )
}

/// A bounded replay observation, not a substitute for the live tool result.
/// Binary tool output must not grow the evidence journal or be re-sent as if
/// the historical image were still available to the model.
pub(super) fn bounded_result(
    result: &nomifun_coding_engine::CodingToolResult,
) -> nomifun_coding_engine::CodingToolResult {
    super::engine_tool_host::bounded_engine_tool_result(result)
}
