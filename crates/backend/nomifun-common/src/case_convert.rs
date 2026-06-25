//! JSON key case conversion.
//!
//! Third-party payloads (notably the ACP SDK) serialise to camelCase,
//! but our frontend contract — and every other response shape we own —
//! is snake_case. Instead of maintaining a parallel typed mirror for
//! each external struct, we deep-walk the produced `serde_json::Value`
//! and rewrite object keys before handing it to the wire.

use serde_json::{Map, Value};

/// Convert a single identifier from camelCase / PascalCase to snake_case.
///
/// Already-snake_case strings pass through unchanged. Leading acronyms
/// like `MCPServer` become `mcp_server`; trailing numeric runs
/// (`sessionV2`) stay attached to their preceding word (`session_v2`).
/// Underscores already present are preserved without duplication.
pub fn camel_to_snake(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 4);
    let chars: Vec<char> = input.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() {
            let prev_is_lower_or_digit = i > 0 && (chars[i - 1].is_ascii_lowercase() || chars[i - 1].is_ascii_digit());
            let next_is_lower = chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase());
            let prev_is_upper = i > 0 && chars[i - 1].is_ascii_uppercase();
            if prev_is_lower_or_digit || (prev_is_upper && next_is_lower) {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Recursively rewrite every object key in `value` to snake_case.
///
/// Arrays and primitives are walked but not otherwise transformed. The
/// `_meta` key emitted by the ACP SDK is left verbatim — the protocol
/// reserves it for passthrough metadata whose inner keys are not ours
/// to normalise.
pub fn normalize_keys_to_snake_case(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let converted: Map<String, Value> = std::mem::take(map)
                .into_iter()
                .map(|(k, mut v)| {
                    if k != "_meta" {
                        normalize_keys_to_snake_case(&mut v);
                    }
                    let new_key = if k == "_meta" { k } else { camel_to_snake(&k) };
                    (new_key, v)
                })
                .collect();
            *map = converted;
        }
        Value::Array(items) => {
            for item in items {
                normalize_keys_to_snake_case(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn camel_to_snake_basic() {
        assert_eq!(camel_to_snake("currentModelId"), "current_model_id");
        assert_eq!(camel_to_snake("already_snake"), "already_snake");
        assert_eq!(camel_to_snake("id"), "id");
        assert_eq!(camel_to_snake(""), "");
    }

    #[test]
    fn camel_to_snake_acronyms() {
        assert_eq!(camel_to_snake("MCPServer"), "mcp_server");
        assert_eq!(camel_to_snake("sessionV2"), "session_v2");
        assert_eq!(camel_to_snake("URLPath"), "url_path");
    }

    #[test]
    fn normalize_nested_object() {
        let mut v = json!({
            "currentModeId": "yolo",
            "availableModes": [
                { "id": "a", "nameLabel": "A" },
                { "id": "b", "nameLabel": "B" }
            ]
        });
        normalize_keys_to_snake_case(&mut v);
        assert_eq!(
            v,
            json!({
                "current_mode_id": "yolo",
                "available_modes": [
                    { "id": "a", "name_label": "A" },
                    { "id": "b", "name_label": "B" }
                ]
            })
        );
    }

    #[test]
    fn normalize_preserves_meta_content() {
        let mut v = json!({
            "availableModes": [],
            "_meta": { "keepThisAsIs": true }
        });
        normalize_keys_to_snake_case(&mut v);
        assert_eq!(v["_meta"]["keepThisAsIs"], true);
        assert!(v.get("available_modes").is_some());
    }

    #[test]
    fn normalize_leaves_primitives() {
        let mut v = json!([1, "two", true, null]);
        normalize_keys_to_snake_case(&mut v);
        assert_eq!(v, json!([1, "two", true, null]));
    }
}
