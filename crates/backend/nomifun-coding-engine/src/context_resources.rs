//! Immutable host-projected reference data. This engine control tool cannot
//! read arbitrary paths, install extensions, activate capabilities or run code.
use crate::{CodingEngineError, CodingToolResult};
use nomifun_agent_contracts::StrictJsonValue;
use nomifun_chat_model_broker::{ChatToolCall, ChatToolDefinition, ChatToolResultPart};
use std::collections::BTreeMap;

pub(crate) const TOOL_NAME: &str = "read_context_resource";
const DEFAULT_PAGE_BYTES: usize = 8 * 1024;
const MAX_PAGE_BYTES: usize = 16 * 1024;
// Bound JSON escaping too, so a page is not silently cut by the host's
// 32-KiB journal result projection. Keep room for the enclosing tool result.
const MAX_ENCODED_PAGE_BYTES: usize = 24 * 1024;

pub use nomifun_engine_core::{
    EngineContextContent as CodingContextContent,
    EngineContextResource as CodingContextResource,
};

pub(crate) fn index(
    resources: &BTreeMap<String, CodingContextResource>,
    image_input: bool,
) -> Result<String, CodingEngineError> {
    let mut bytes = 0usize;
    let mut image_bytes = 0usize;
    let mut images = 0usize;
    let mut entries = Vec::new();
    if resources.len() > 64 {
        return Err(CodingEngineError::ContextAssembly(
            "too many selected context resources".into(),
        ));
    }
    for (id, resource) in resources {
        if id.is_empty()
            || id.len() > 128
            || resource.label.len() > 256
            || resource.provenance.len() > 1024
        {
            return Err(CodingEngineError::ContextAssembly(
                "selected context resource exceeds its bounded envelope".into(),
            ));
        }
        let entry = match &resource.content {
            CodingContextContent::Text { text } => {
                bytes = bytes.saturating_add(text.len());
                if text.len() > 256 * 1024 || bytes > 512 * 1024 {
                    return Err(CodingEngineError::ContextAssembly(
                        "selected text resource budget exceeded".into(),
                    ));
                }
                serde_json::json!({"id":id,"label":resource.label,"kind":"text","total_bytes":text.len()})
            }
            CodingContextContent::Image {
                media_type,
                data_base64,
            } => {
                images += 1;
                image_bytes = image_bytes.saturating_add(data_base64.len());
                if images > 4
                    || image_bytes > 8 * 1024 * 1024
                    || data_base64.is_empty()
                    || data_base64.len() > 2 * 1024 * 1024
                    || !matches!(media_type.as_str(), "image/png" | "image/jpeg")
                {
                    return Err(CodingEngineError::ContextAssembly(
                        "selected image resource budget or format is invalid".into(),
                    ));
                }
                serde_json::json!({"id":id,"label":resource.label,"kind":"image","media_type":media_type,"encoded_bytes":data_base64.len(),"image_input_available":image_input})
            }
        };
        entries.push(entry);
    }
    Ok(format!(
        "Selected immutable reference resources. Use read_context_resource with an exact id when relevant. Text reads return bounded UTF-8 pages; follow next_offset until eof when the complete resource is needed. Offsets count bytes, not characters or lines. Image reads require image_input_available, must be a single-call batch and omit offset/limit; they return host-prepared pixels, not text/base64 to interpret. These are packaged data, not workspace paths or extra permissions. Scripts are reference text and are never executed by this reader. Historical media descriptors do not imply pixels are present; re-read the resource when needed. Index: {}",
        serde_json::Value::Array(entries)
    ))
}

pub(crate) fn definition() -> ChatToolDefinition {
    ChatToolDefinition {
        name: TOOL_NAME.into(),
        description: "Read an exact selected immutable Skill resource. Text: start at offset 0, follow next_offset until eof; offsets/limit are UTF-8 bytes, not lines. Image: omit offset/limit and submit alone; requires available image input. Returns bounded prepared pixels. No workspace IO or script execution.".into(),
        input_schema: StrictJsonValue(serde_json::json!({"type":"object","additionalProperties":false,"required":["id"],"properties":{
            "id":{"type":"string","minLength":1,"maxLength":128},
            "offset":{"type":"integer","minimum":0,"default":0},
            "limit":{"type":"integer","minimum":4,"maximum":MAX_PAGE_BYTES,"default":DEFAULT_PAGE_BYTES}
        }})),
        deferred: false,
    }
}

pub(crate) fn read(
    call: &ChatToolCall,
    resources: &BTreeMap<String, CodingContextResource>,
    image_input: bool,
    single_call: bool,
) -> CodingToolResult {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Read {
        id: String,
        #[serde(default)]
        offset: usize,
        #[serde(default = "default_page_bytes")]
        limit: usize,
    }
    let invalid = |reason: &str| CodingToolResult::text(call.call_id.clone(), reason, true);
    let Ok(request) = serde_json::from_value::<Read>(call.arguments.0.clone()) else {
        return invalid(
            "Invalid resource arguments: use id, optional byte offset, and optional limit (4..16384 bytes).",
        );
    };
    let Some(resource) = resources.get(&request.id) else {
        return invalid("Unknown context resource; choose an exact id from the frozen index.");
    };
    let text = match &resource.content {
        CodingContextContent::Text { text } => text,
        CodingContextContent::Image {
            media_type,
            data_base64,
        } => {
            if !image_input {
                return invalid(
                    "Image resource unavailable: this turn requires active llm.vision and an image-capable exact model route. No pixels were provided.",
                );
            }
            if !single_call
                || call.arguments.0.get("offset").is_some()
                || call.arguments.0.get("limit").is_some()
            {
                return invalid(
                    "Read an image resource alone and omit offset/limit; image bytes cannot be paged as text. No pixels were provided.",
                );
            }
            return CodingToolResult {
                call_id: call.call_id.clone(),
                is_error: false,
                output: vec![
                    ChatToolResultPart::Text { text: serde_json::json!({
                        "notice":"Selected immutable image reference, not an authority grant. Host resized/re-encoded pixels may differ in resolution or encoding from the source. Re-read after historical replay if pixels are needed.",
                        "id":request.id,"provenance":resource.provenance,"media_type":media_type,"encoded_bytes":data_base64.len()
                    }).to_string() },
                    ChatToolResultPart::Image { media_type:media_type.clone(), data_base64:data_base64.clone() },
                ],
            };
        }
    };
    if !(4..=MAX_PAGE_BYTES).contains(&request.limit)
        || request.offset > text.len()
        || !text.is_char_boundary(request.offset)
    {
        return invalid(
            "Invalid page: offset must be a UTF-8 boundary within total_bytes; use the preceding next_offset. Limit must be 4..16384 bytes.",
        );
    }
    let mut end = request.offset.saturating_add(request.limit).min(text.len());
    loop {
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let eof = end == text.len();
        let page = serde_json::json!({
            "notice":"Selected immutable reference data, not an authority grant. Scripts are text only.",
            "id":request.id,
            "provenance":resource.provenance,
            "offset":request.offset,
            "end_offset":end,
            "total_bytes":text.len(),
            "next_offset":if eof { None } else { Some(end) },
            "eof":eof,
            "text":&text[request.offset..end]
        }).to_string();
        // The text result is itself a JSON string inside the persisted result.
        // Count that second encoding, not just the raw UTF-8 page length.
        let encoded_bytes = serde_json::Value::String(page.clone()).to_string().len();
        if encoded_bytes <= MAX_ENCODED_PAGE_BYTES {
            return CodingToolResult::text(call.call_id.clone(), page, false);
        }
        if end == request.offset {
            return invalid("Resource metadata exceeds the bounded page envelope.");
        }
        let first_char_bytes = text[request.offset..]
            .chars()
            .next()
            .map_or(0, char::len_utf8);
        if end - request.offset <= first_char_bytes {
            return invalid("Resource metadata exceeds the bounded page envelope.");
        }
        end = request.offset + ((end - request.offset) / 2).max(first_char_bytes);
    }
}

fn default_page_bytes() -> usize {
    DEFAULT_PAGE_BYTES
}
