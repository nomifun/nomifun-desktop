const INLINE_THRESHOLD: usize = 80;

fn format_value(value: &serde_json::Value, depth: usize) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let oneliner = serde_json::to_string(value).unwrap_or_default();
            if oneliner.len() <= INLINE_THRESHOLD {
                return oneliner;
            }
            let indent = "  ".repeat(depth + 1);
            let close_indent = "  ".repeat(depth);
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{indent}{}: {}", serde_json::to_string(k).unwrap(), format_value(v, depth + 1)))
                .collect();
            format!("{{\n{}\n{close_indent}}}", entries.join(",\n"))
        }
        serde_json::Value::Array(arr) => {
            let oneliner = serde_json::to_string(value).unwrap_or_default();
            if oneliner.len() <= INLINE_THRESHOLD {
                return oneliner;
            }
            let indent = "  ".repeat(depth + 1);
            let close_indent = "  ".repeat(depth);
            let items: Vec<String> = arr
                .iter()
                .map(|v| format!("{indent}{}", format_value(v, depth + 1)))
                .collect();
            format!("[\n{}\n{close_indent}]", items.join(",\n"))
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

pub fn compact_json(text: &str) -> String {
    compact_json_block(text, false).unwrap_or_else(|| text.to_owned())
}

// Parse with serde instead of counting brackets, which may occur inside strings.
// Only fold the plain prefix. The suffix may contain more structured data.
pub(crate) fn compact_json_block(text: &str, fold_logs: bool) -> Option<String> {
    let start = text.find(['{', '['])?;
    let mut values = serde_json::Deserializer::from_str(&text[start..]).into_iter::<serde_json::Value>();
    let value = values.next()?.ok()?;
    let end = start + values.byte_offset();
    let compacted = format_value(&value, 0);
    let body = if compacted.len() < end - start { &compacted } else { &text[start..end] };
    if fold_logs {
        Some(format!("{}{}{}", super::fold::fold_repeated_lines(&text[..start]), body,
            &text[end..]))
    } else {
        Some(format!("{}{}{}", &text[..start], body, &text[end..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_4space_to_2space() {
        let input = r#"{
    "name": "Alice Wonderland",
    "email": "alice@example.com",
    "age": 30,
    "address": "123 Main Street, Anytown, USA 12345",
    "phone": "+1-555-0123"
}"#;
        let result = compact_json(input);
        assert!(
            result.contains("  \"name\""),
            "should use 2-space indent: {result}"
        );
        assert!(
            !result.contains("    \"name\""),
            "should not have 4-space indent"
        );
    }

    #[test]
    fn compact_short_object_inline() {
        let input = r#"{
    "user": {
        "id": 1,
        "name": "Alice"
    }
}"#;
        let result = compact_json(input);
        assert!(
            result.contains(r#"{"id":1,"name":"Alice"}"#)
                || result.contains(r#"{"id": 1, "name": "Alice"}"#),
            "short nested object should be inlined: {result}"
        );
    }

    #[test]
    fn compact_non_json_unchanged() {
        let input = "This is not JSON\njust plain text";
        assert_eq!(compact_json(input), input);
    }

    #[test]
    fn compact_already_minified() {
        let input = r#"{"id":1,"name":"Alice"}"#;
        let result = compact_json(input);
        assert_eq!(
            result.len(),
            input.len(),
            "already compact JSON should not grow"
        );
    }

    #[test]
    fn compact_preserves_array_structure() {
        let input = r#"[
    {
        "id": 1,
        "name": "Alice"
    },
    {
        "id": 2,
        "name": "Bob"
    }
]"#;
        let result = compact_json(input);
        assert!(result.len() < input.len(), "should be shorter than input");
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed[0]["name"], "Alice");
        assert_eq!(parsed[1]["name"], "Bob");
    }

    #[test]
    fn compact_mixed_text_with_json_block() {
        let input = "Exit code: 0\nSTDOUT:\n{\n    \"status\": \"ok\"\n}\nSTDERR:\n";
        let result = compact_json(input);
        assert!(result.contains("\"status\""));
    }

    #[test]
    fn compact_empty_input() {
        assert_eq!(compact_json(""), "");
    }
}
