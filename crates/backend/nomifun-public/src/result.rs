//! Maps a gateway dispatch result (`serde_json::Value`) onto an MCP
//! `CallToolResult`. Mirrors `nomifun-app`'s `gateway_stdio::build_tool_result`
//! image seam, but operates on the in-process `Value` directly (no HTTP hop).

use rmcp::model::{CallToolResult, Content};
use serde_json::Value;

/// Build the MCP tool result from a gateway dispatch result envelope.
///
/// Image seam (matches the inward stdio bridge): a successful result object may
/// attach `_mcp_images: [{"mime_type","data"}]`; those become proper MCP image
/// parts and the key is stripped from its text payload. The adapter accepts
/// exactly one top-level `result` or `error`; `needs_confirmation: true` is the
/// gateway permission gate's sole explicit control outcome.
pub fn build_tool_result(value: Value) -> CallToolResult {
    let has_result = value.get("result").is_some();
    let has_error = value.get("error").is_some();
    let is_confirmation = value
        .get("needs_confirmation")
        .and_then(Value::as_bool)
        == Some(true);
    if has_result && has_error {
        return protocol_error("gateway response contained both `result` and `error`");
    }
    if is_confirmation && (has_result || has_error) {
        return protocol_error("gateway response mixed a confirmation with a result envelope");
    }
    if let Some(error) = value.get("error") {
        return CallToolResult::error(vec![Content::text(format!("Error: {error}"))]);
    }
    let Some(result) = value.get("result") else {
        if is_confirmation {
            return CallToolResult::success(vec![Content::text(value.to_string())]);
        }
        return protocol_error("gateway response was missing `result` or `error`");
    };

    let mut result = result.clone();
    let images: Vec<Content> = result
        .get("_mcp_images")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|img| {
                    let data = img.get("data").and_then(Value::as_str)?;
                    let mime = img.get("mime_type").and_then(Value::as_str)?;
                    Some(Content::image(data.to_owned(), mime.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();

    if !images.is_empty()
        && let Value::Object(map) = &mut result
    {
        map.remove("_mcp_images");
    }

    let text = match result {
        Value::String(text) => text,
        other => serde_json::to_string(&other).unwrap_or_else(|_| other.to_string()),
    };
    let mut contents = vec![Content::text(text)];
    contents.extend(images);
    CallToolResult::success(contents)
}

fn protocol_error(message: &str) -> CallToolResult {
    CallToolResult::error(vec![Content::text(format!(
        "Error: invalid gateway tool response ({message})"
    ))])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_result_becomes_one_non_error_text_part() {
        let r = build_tool_result(serde_json::json!({
            "result": {
                "ok": true,
                "message": "Error is ordinary data here"
            }
        }));
        assert_eq!(r.content.len(), 1);
        assert_ne!(r.is_error, Some(true));
    }

    #[test]
    fn top_level_gateway_error_becomes_mcp_tool_error() {
        let r = build_tool_result(serde_json::json!({
            "error": "invalid arguments for this tool: missing field `kb_id`"
        }));
        assert_eq!(r.is_error, Some(true));
    }

    #[test]
    fn domain_is_error_field_is_not_protocol_metadata() {
        let r = build_tool_result(serde_json::json!({
            "result": {"operation": "validation", "isError": true}
        }));
        assert_ne!(r.is_error, Some(true));
    }

    #[test]
    fn nested_result_error_is_ordinary_success_data() {
        let r = build_tool_result(serde_json::json!({
            "result": {"error": "a domain field"}
        }));
        assert_ne!(r.is_error, Some(true));
    }

    #[test]
    fn malformed_or_ambiguous_envelopes_fail_closed() {
        for value in [
            serde_json::json!({"ok": true}),
            serde_json::json!(null),
            serde_json::json!({"result": "ok", "error": "failed"}),
            serde_json::json!({"result": "ok", "needs_confirmation": true}),
        ] {
            assert_eq!(build_tool_result(value).is_error, Some(true));
        }
    }

    #[test]
    fn explicit_confirmation_outcome_is_success() {
        let r = build_tool_result(serde_json::json!({
            "needs_confirmation": true,
            "tool": "nomi_delete"
        }));
        assert_ne!(r.is_error, Some(true));
    }

    #[test]
    fn images_marker_splits_into_text_plus_image() {
        let r = build_tool_result(serde_json::json!({
            "result": {
                "note": "shot",
                "_mcp_images": [{"mime_type": "image/png", "data": "AAAA"}]
            }
        }));
        assert_eq!(r.content.len(), 2, "one text part + one image part");
    }
}
