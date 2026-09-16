//! Coding-only model-window policy. Original results reach evidence tracking
//! and the bounded archive first; this module cannot execute or certify tools.
use nomifun_chat_model_broker::ChatToolResultPart;
use serde_json::{Value, json};

use crate::CodingToolResult;

const TEXT_BYTES: usize = 16 * 1024;
const ENCODED_BYTES: usize = 24 * 1024;
const MARKER: &str = "_nomifun_context_excerpt";

#[derive(Clone, Copy)]
pub(crate) enum ToolContextKind {
    Process,
    Diff,
}

impl ToolContextKind {
    /// Canonical Action, not a model-selected tool name. Unrecognized
    /// community output, file/instruction reads and media stay unchanged.
    pub fn for_action(action: &str) -> Option<Self> {
        match action {
            action if action.starts_with("workspace.process/") => Some(Self::Process),
            "workspace.vcs/diff" => Some(Self::Diff),
            _ => None,
        }
    }

    fn fields(self) -> &'static [&'static str] {
        match self {
            Self::Process => &["/output/text"],
            Self::Diff => &["/patch", "/staged_patch", "/unstaged_patch"],
        }
    }
}

/// Preserve the complete JSON envelope (including state, process_id, cursor,
/// exit code, cleanup and original truncation fields). Only recognized bulky
/// strings become explicit head/tail excerpts; these are not applicable patches.
/// A malformed/unknown/oversized-metadata envelope is left for normal compaction.
pub(crate) fn project(kind: ToolContextKind, result: &CodingToolResult) -> CodingToolResult {
    let [ChatToolResultPart::Text { text }] = result.output.as_slice() else {
        return result.clone();
    };
    let Ok(mut value) = serde_json::from_str::<Value>(text) else {
        return result.clone();
    };
    if !value.is_object() || value.get(MARKER).is_some() {
        return result.clone();
    }
    let fields = kind.fields();
    let mut source = Vec::new();
    for path in fields {
        let Some(field) = value.pointer(path).and_then(Value::as_str) else {
            return result.clone();
        };
        source.push((*path, field.to_owned()));
    }
    let total = source.iter().map(|(_, text)| text.len()).sum::<usize>();
    let Ok(original_size) = crate::stream_limits::serialized_size(result, usize::MAX) else {
        return result.clone();
    };
    if total <= TEXT_BYTES && original_size <= ENCODED_BYTES {
        return result.clone();
    }
    let target_size = ENCODED_BYTES.min(original_size.saturating_sub(1));
    // JSON escaping can expand a small UTF-8 excerpt several times. Halve the
    // text allowance until the complete encoded result fits, never truncate
    // serialized JSON or drop an operational metadata field to force a fit.
    let mut allowance = TEXT_BYTES / fields.len();
    loop {
        let mut retained = Vec::new();
        for (path, original) in &source {
            let (text, head, tail) = excerpt(original, allowance);
            *value
                .pointer_mut(path)
                .expect("selected JSON string retained") = Value::String(text);
            retained.push(json!({"field":path,"original_utf8_bytes":original.len(),
                "head_utf8_bytes":head,"tail_utf8_bytes":tail,
                "omitted_utf8_bytes":original.len().saturating_sub(head + tail)}));
        }
        value[MARKER] = json!({"version":1,"call_id":result.call_id,"fields":retained,
            "notice":"Engine model-context excerpt, not a complete log or applicable patch. Operational metadata and original is_error are unchanged; source cursor/retained_bytes/dropped_bytes still describe the owner output, not this excerpt. Missing text is not proof of absence or success. Use search_tool_history with query='' and call_id to locate retained original text, then read_tool_history; archive limits/eviction may make omitted text unavailable. Never rerun an effect solely to recover its output."});
        let projected = CodingToolResult {
            call_id: result.call_id.clone(),
            is_error: result.is_error,
            output: vec![ChatToolResultPart::Text {
                text: value.to_string(),
            }],
        };
        if crate::stream_limits::serialized_size(&projected, target_size).is_ok() {
            return projected;
        }
        if allowance == 0 {
            return result.clone();
        }
        allowance /= 2;
    }
}

fn excerpt(text: &str, allowance: usize) -> (String, usize, usize) {
    if text.len() <= allowance {
        return (text.into(), text.len(), 0);
    }
    let mut head = allowance / 2;
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = text.len().saturating_sub(allowance - allowance / 2);
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    (
        format!(
            "{}\n[Engine context excerpt: {} UTF-8 bytes omitted; not contiguous source]\n{}",
            &text[..head],
            tail - head,
            &text[tail..]
        ),
        head,
        text.len() - tail,
    )
}
