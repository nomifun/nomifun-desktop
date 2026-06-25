//! Maps a gateway dispatch result (`serde_json::Value`) onto an MCP
//! `CallToolResult`. Mirrors `nomifun-app`'s `gateway_stdio::build_tool_result`
//! image seam, but operates on the in-process `Value` directly (no HTTP hop).

use rmcp::model::{CallToolResult, Content};
use serde_json::Value;

/// Build the MCP tool result from a gateway dispatch result value.
///
/// Image seam (matches the inward stdio bridge): a capability that returns
/// images attaches `_mcp_images: [{"mime_type","data"}]`; those become proper
/// MCP `image` content parts and the key is stripped from the text payload so
/// the base64 isn't also emitted as text tokens. Dispatch errors are returned
/// as `{"error": ...}` JSON text in a success result — identical to the inward
/// bridge's behaviour, so external clients see the same shape.
pub fn build_tool_result(mut value: Value) -> CallToolResult {
    let images: Vec<Content> = value
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
        && let Value::Object(map) = &mut value
    {
        map.remove("_mcp_images");
    }

    let text = serde_json::to_string(&value).unwrap_or_else(|_| value.to_string());
    let mut contents = vec![Content::text(text)];
    contents.extend(images);
    CallToolResult::success(contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_value_becomes_one_text_part() {
        let r = build_tool_result(serde_json::json!({"ok": true}));
        assert_eq!(r.content.len(), 1);
    }

    #[test]
    fn images_marker_splits_into_text_plus_image() {
        let r = build_tool_result(serde_json::json!({
            "note": "shot",
            "_mcp_images": [{"mime_type": "image/png", "data": "AAAA"}]
        }));
        assert_eq!(r.content.len(), 2, "one text part + one image part");
    }
}
