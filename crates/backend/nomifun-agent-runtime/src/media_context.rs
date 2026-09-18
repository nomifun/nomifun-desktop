//! Media bytes bound transport memory, not text tokens. Like Codex's history
//! estimator, account for resized images separately from base64 serialization.
use crate::AgentEngineError;
use nomifun_chat_model_broker::{ChatContentPart, ChatMessage, ChatModelInput, ChatToolResultPart};

// Conservative envelope for the host's <=1568px resized images. This is not a
// provider tokenizer or billing claim; observed usage supplies a second bound.
const IMAGE_TOKEN_RESERVE: usize = 4096;

pub(crate) fn estimate_tokens(input: &ChatModelInput, encoded_bytes: usize) -> usize {
    let mut binary_bytes = 0usize;
    let mut images = 0usize;
    for message in &input.messages {
        for part in &message.content {
            match part {
                ChatContentPart::Image { data_base64, .. } => {
                    binary_bytes = binary_bytes.saturating_add(data_base64.len());
                    images += 1;
                }
                ChatContentPart::ToolResult { output, .. } => {
                    for part in output {
                        if let ChatToolResultPart::Image { data_base64, .. } = part {
                            binary_bytes = binary_bytes.saturating_add(data_base64.len());
                            images += 1;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    encoded_bytes
        .saturating_sub(binary_bytes)
        .div_ceil(3)
        .saturating_add(images.saturating_mul(IMAGE_TOKEN_RESERVE))
}

fn notice(kind: &str, media_type: &str, bytes: usize) -> String {
    format!(
        "[{kind} attachment: {media_type}, {bytes} encoded bytes; binary payload omitted from summarization. Do not infer its contents from this descriptor.]"
    )
}

pub(crate) fn summary_source(
    messages: &[ChatMessage],
) -> Result<crate::compaction_source::SummarySource, AgentEngineError> {
    let instruction_reads = messages
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|part| {
            if let ChatContentPart::ToolCall {
                call_id,
                name,
                arguments,
                ..
            } = part
                && name == "read_file"
                && arguments
                    .0
                    .get("path")
                    .and_then(|v| v.as_str())
                    .is_some_and(|path| {
                        path.rsplit(['/', '\\']).next().is_some_and(|name| {
                            name.eq_ignore_ascii_case("AGENTS.md")
                                || name.eq_ignore_ascii_case("AGENTS.override.md")
                        })
                    })
            {
                Some(call_id.clone())
            } else {
                None
            }
        })
        .collect::<std::collections::BTreeSet<_>>();
    let mut source = crate::compaction_source::SummarySource::default();
    // Sanitize one message at a time; do not clone every binary attachment
    // and opaque block before discarding it from the summary source.
    for original in messages {
        let mut message = original.clone();
        for part in &mut message.content {
            match part {
                ChatContentPart::Image {
                    media_type,
                    data_base64,
                } => {
                    *part = ChatContentPart::Text {
                        text: notice("Image", media_type, data_base64.len()),
                    };
                }
                ChatContentPart::Audio {
                    media_type,
                    data_base64,
                } => {
                    *part = ChatContentPart::Text {
                        text: notice("Audio", media_type, data_base64.len()),
                    };
                }
                ChatContentPart::ToolResult {
                    call_id, output, ..
                } => {
                    if instruction_reads.contains(call_id) {
                        *output = vec![ChatToolResultPart::Text { text: "[Repository instruction body omitted; current scoped instructions are retained separately and must be re-read on a later turn.]".into() }];
                        continue;
                    }
                    for part in output {
                        let text = match part {
                            ChatToolResultPart::Image {
                                media_type,
                                data_base64,
                            } => Some(notice("Image", media_type, data_base64.len())),
                            ChatToolResultPart::Audio {
                                media_type,
                                data_base64,
                            } => Some(notice("Audio", media_type, data_base64.len())),
                            ChatToolResultPart::Text { .. } => None,
                        };
                        if let Some(text) = text {
                            *part = ChatToolResultPart::Text { text };
                        }
                    }
                }
                // Native reasoning/signatures are not a factual transcript and
                // must not be interpreted by a separate summary model call.
                ChatContentPart::Reasoning { .. } | ChatContentPart::ProviderReasoning { .. } => {
                    *part = ChatContentPart::Text {
                        text: "[Private reasoning omitted]".into(),
                    };
                }
                ChatContentPart::ToolCall {
                    provider_metadata, ..
                } => {
                    // Provider-private signatures/continuation fields belong
                    // to live model history, not a separate summary request.
                    *provider_metadata = None;
                }
                _ => {}
            }
        }
        message.provider_round_id = None;
        source.push(&message)?;
    }
    Ok(source)
}
