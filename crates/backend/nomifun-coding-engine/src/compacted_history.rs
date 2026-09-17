//! Portable model context only, never an effect receipt or recovery proof.
use crate::CodingEngineError;
use nomifun_chat_model_broker::{
    ChatContentPart, ChatMessage, ChatRole, ChatToolResultPart, ToolCallId,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_ITEMS: usize = 4096;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CodingCompactedItem {
    AcceptedInput { index: usize },
    Message { message: ChatMessage },
}

fn invalid(message: impl std::fmt::Display) -> CodingEngineError {
    CodingEngineError::ReplayContract(format!("compaction replacement: {message}"))
}

/// Capture the selected replacement without its summary. Reverse matching
/// preserves identical repeated accepted inputs and their multiplicity.
pub(crate) fn capture(
    messages: &[ChatMessage],
    requirements: &[ChatMessage],
    call_ids: &[ToolCallId],
) -> Result<Vec<CodingCompactedItem>, CodingEngineError> {
    if messages.len() > MAX_ITEMS {
        return Err(invalid("message count exceeds bound"));
    }
    let mut positions = BTreeMap::new();
    let mut cursor = messages.len();
    for (index, requirement) in requirements.iter().enumerate().rev() {
        let position = messages[..cursor]
            .iter()
            .rposition(|message| {
                message.role == requirement.role && message.content == requirement.content
            })
            .ok_or_else(|| invalid("accepted input missing from replacement"))?;
        positions.insert(position, index);
        cursor = position;
    }
    let instructions = instruction_reads(messages);
    let mut items = Vec::new();
    let mut bytes = 0usize;
    for (position, message) in messages.iter().enumerate() {
        let item = if let Some(index) = positions.get(&position) {
            CodingCompactedItem::AcceptedInput { index: *index }
        } else {
            CodingCompactedItem::Message {
                message: portable(message, &instructions)?,
            }
        };
        bytes = bytes.saturating_add(crate::stream_limits::serialized_size(
            &item,
            MAX_BYTES.saturating_sub(bytes),
        )?);
        items.push(item);
    }
    restore(&items, requirements, call_ids)?;
    Ok(items)
}

/// Only normal model-history replay consumes these observations. Isolated
/// archive inspection must not import another turn's tool results as evidence.
pub(crate) fn restore(
    items: &[CodingCompactedItem],
    requirements: &[ChatMessage],
    call_ids: &[ToolCallId],
) -> Result<Vec<ChatMessage>, CodingEngineError> {
    if items.len() > MAX_ITEMS {
        return Err(invalid("message count exceeds bound"));
    }
    crate::stream_limits::serialized_size(&items, MAX_BYTES).map_err(invalid)?;
    let mut next_input = 0usize;
    let mut messages = Vec::new();
    for item in items {
        match item {
            CodingCompactedItem::AcceptedInput { index } => {
                if *index != next_input {
                    return Err(invalid("accepted inputs repeated, omitted or reordered"));
                }
                let mut message = requirements
                    .get(*index)
                    .ok_or_else(|| invalid("unknown accepted input"))?
                    .clone();
                message.provider_round_id = None;
                messages.push(message);
                next_input += 1;
            }
            CodingCompactedItem::Message { message } => {
                validate_portable(message)?;
                messages.push(message.clone());
            }
        }
    }
    if next_input != requirements.len() {
        return Err(invalid("accepted inputs missing"));
    }
    if call_ids.is_empty() {
        if items
            .iter()
            .any(|item| matches!(item, CodingCompactedItem::Message { .. }))
        {
            return Err(invalid("unreferenced messages without a tool suffix"));
        }
    } else {
        let exchange = crate::context_tail::selected(&messages, call_ids)
            .map_err(invalid)?
            .ok_or_else(|| invalid("references do not match a complete contiguous suffix"))?;
        if exchange
            .with_required_inputs(requirements)
            .map_err(invalid)?
            != messages
        {
            return Err(invalid("replacement contains an unselected prefix"));
        }
    }
    Ok(messages)
}

fn instruction_reads(messages: &[ChatMessage]) -> BTreeSet<ToolCallId> {
    messages
        .iter()
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
            (call_id.as_ref().starts_with("coding-instructions:")
                || (name == "read_file"
                    && arguments
                        .0
                        .get("path")
                        .and_then(|value| value.as_str())
                        .is_some_and(|path| {
                            path.rsplit(['/', '\\']).next().is_some_and(|name| {
                                name.eq_ignore_ascii_case("AGENTS.md")
                                    || name.eq_ignore_ascii_case("AGENTS.override.md")
                            })
                        })))
            .then(|| call_id.clone())
        })
        .collect()
}

fn media_notice(media_type: &str, bytes: usize) -> String {
    format!(
        "[Historical media {media_type}, {bytes} encoded bytes; binary body omitted from replay. Do not infer its contents.]"
    )
}

fn portable(
    message: &ChatMessage,
    instructions: &BTreeSet<ToolCallId>,
) -> Result<ChatMessage, CodingEngineError> {
    // Never clone raw pixels or provider-private blocks into the event.
    let content = message.content.iter().map(|part| match part {
        ChatContentPart::Text { text } => ChatContentPart::Text { text: text.clone() },
        ChatContentPart::Image { media_type, data_base64 } | ChatContentPart::Audio { media_type, data_base64 } =>
            ChatContentPart::Text { text: media_notice(media_type, data_base64.len()) },
        ChatContentPart::Reasoning { .. } | ChatContentPart::ProviderReasoning { .. } =>
            ChatContentPart::Text { text: "[Private reasoning omitted from replay]".into() },
        ChatContentPart::ToolCall { call_id, name, arguments, .. } => ChatContentPart::ToolCall {
            call_id: call_id.clone(), name: name.clone(), arguments: arguments.clone(), provider_metadata: None,
        },
        ChatContentPart::ToolResult { call_id, output, is_error } => ChatContentPart::ToolResult {
            call_id: call_id.clone(), is_error: *is_error,
            output: if instructions.contains(call_id) {
                vec![ChatToolResultPart::Text { text: "[Repository instruction body omitted from replay; re-read current scoped rules through the authorized workspace tool.]".into() }]
            } else { output.iter().map(|part| match part {
                ChatToolResultPart::Text { text } => ChatToolResultPart::Text { text: text.clone() },
                ChatToolResultPart::Image { media_type, data_base64 } | ChatToolResultPart::Audio { media_type, data_base64 } =>
                    ChatToolResultPart::Text { text: media_notice(media_type, data_base64.len()) },
            }).collect() },
        },
    }).collect();
    let projected = ChatMessage {
        role: message.role,
        content,
        provider_round_id: None,
    };
    validate_portable(&projected)?;
    Ok(projected)
}

fn validate_portable(message: &ChatMessage) -> Result<(), CodingEngineError> {
    if message.provider_round_id.is_some() || message.content.is_empty() {
        return Err(invalid("empty message or private round state"));
    }
    for part in &message.content {
        match (message.role, part) {
            (ChatRole::User | ChatRole::Assistant, ChatContentPart::Text { .. }) => {}
            (
                ChatRole::Assistant,
                ChatContentPart::ToolCall {
                    call_id,
                    name,
                    provider_metadata: None,
                    ..
                },
            ) => {
                crate::stream_limits::identity(call_id, name).map_err(invalid)?;
            }
            (ChatRole::Tool, ChatContentPart::ToolResult { output, .. })
                if !output.is_empty()
                    && output
                        .iter()
                        .all(|part| matches!(part, ChatToolResultPart::Text { .. })) => {}
            _ => return Err(invalid("non-portable content or privileged role")),
        }
    }
    Ok(())
}
