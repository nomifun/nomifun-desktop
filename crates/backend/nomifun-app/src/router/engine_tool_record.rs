//! Canonical tool content, independent of the model's context budget.
//!
//! Text is an exact owner observation and may contain recovery state. Only
//! ephemeral media and repository instruction bodies have explicit historical
//! representations; ordinary text is never shortened to fit a journal record.
use nomifun_chat_model_broker::{ChatToolCall, ChatToolResultPart};
use nomifun_engine_core::EngineToolResult;
use sha2::{Digest, Sha256};

pub(super) fn is_instruction_read(call: &ChatToolCall) -> bool {
    call.call_id.as_ref().starts_with("agent-instructions:")
        || (call.name == "read_file"
            && call.arguments.0.get("path").and_then(|value| value.as_str())
                .is_some_and(|path| path.rsplit(['/', '\\']).next().is_some_and(|name| {
                    name.eq_ignore_ascii_case("AGENTS.md")
                        || name.eq_ignore_ascii_case("AGENTS.override.md")
                })))
}

pub(super) fn instruction_notice(text: &str) -> String {
    format!(
        "Repository instruction body omitted from durable history ({} UTF-8 bytes, sha256:{:x}). Re-read current rules through the authorized workspace tool; this digest grants no authority.",
        text.len(), Sha256::digest(text.as_bytes())
    )
}

pub(super) fn instruction_result(result: &EngineToolResult) -> EngineToolResult {
    EngineToolResult::text(result.call_id.clone(), instruction_notice(&result.output_text()), result.is_error)
}

pub(super) fn for_call(call: &ChatToolCall, result: &EngineToolResult) -> EngineToolResult {
    if is_instruction_read(call) { instruction_result(result) } else { durable_result(result) }
}

pub(super) fn durable_result(result: &EngineToolResult) -> EngineToolResult {
    EngineToolResult {
        call_id: result.call_id.clone(),
        is_error: result.is_error,
        output: result.output.iter().map(|part| match part {
            ChatToolResultPart::Text { .. } => part.clone(),
            ChatToolResultPart::Image { media_type, data_base64 }
            | ChatToolResultPart::Audio { media_type, data_base64 } => ChatToolResultPart::Text {
                text: format!("[Historical media {media_type}, {} encoded bytes; binary body omitted from replay]", data_base64.len()),
            },
        }).collect(),
    }
}
