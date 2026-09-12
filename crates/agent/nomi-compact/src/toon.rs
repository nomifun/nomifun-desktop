pub fn toon_encode_array(value: &serde_json::Value) -> Option<String> {
    let arr = value.as_array()?;
    if arr.is_empty() {
        return None;
    }

    let first = arr[0].as_object()?;
    let fields: Vec<&str> = first.keys().map(|k| k.as_str()).collect();

    if fields.is_empty() {
        return None;
    }

    for item in arr {
        let obj = item.as_object()?;
        if obj.len() != fields.len() {
            return None;
        }
        for field in &fields {
            let val = obj.get(*field)?;
            if val.is_object() || val.is_array() {
                return None;
            }
        }
    }

    let mut result = String::new();
    let header = format!("[{}]{{{}}}:", arr.len(), fields.iter().map(|f| format_toon_string(f)).collect::<Vec<_>>().join(","));
    result.push_str(&header);
    result.push('\n');

    for item in arr {
        let obj = item.as_object().unwrap();
        result.push_str("  ");
        let values: Vec<String> = fields
            .iter()
            .map(|f| format_toon_value(obj.get(*f).unwrap()))
            .collect();
        result.push_str(&values.join(","));
        result.push('\n');
    }

    if result.ends_with('\n') {
        result.pop();
    }

    Some(result)
}

fn format_toon_string(value: &str) -> String {
    // Bare words stay compact; ambiguous scalars and syntax use JSON escaping.
    if !value.is_empty()
        && value.trim() == value
        && !matches!(value, "true" | "false" | "null")
        && value.parse::<f64>().is_err()
        && !value.starts_with('-')
        && !value.chars().any(|c| c.is_control() || matches!(c, ',' | ':' | '[' | ']' | '{' | '}' | '"' | '\\'))
    {
        value.to_owned()
    } else {
        serde_json::to_string(value).unwrap()
    }
}

fn format_toon_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format_toon_string(s),
        _ => serde_json::to_string(value).unwrap(),
    }
}

pub fn try_toon_encode(text: &str) -> String {
    let Some(start) = text.find('[') else { return text.to_owned() };
    let mut values = serde_json::Deserializer::from_str(&text[start..]).into_iter::<serde_json::Value>();
    if let Some(Ok(value)) = values.next()
        && let Some(encoded) = toon_encode_array(&value)
    {
        let end = start + values.byte_offset();
        return format!("{}{}{}", &text[..start], encoded, &text[end..]);
    }
    text.to_owned()
}

pub fn toon_format_instructions() -> &'static str {
    "\
# TOON Format

Tool results may contain data in TOON (Token-Oriented Object Notation) tabular format \
for token efficiency. Format:

```
[N]{field1,field2,...}:
  value1,value2,...
  value1,value2,...
```

- `[N]` is the array length
- `{fields}` are column headers
- Each indented line is one row, values comma-separated
- Ambiguous string values and column headers are quoted using JSON string escapes

This is equivalent to a JSON array of objects. Example:
```
[2]{id,name,role}:
  1,Alice,admin
  2,Bob,user
```
equals `[{\"id\":1,\"name\":\"Alice\",\"role\":\"admin\"},{\"id\":2,\"name\":\"Bob\",\"role\":\"user\"}]`"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_uniform_array() {
        let json = r#"[
            {"id": 1, "name": "Alice", "role": "admin"},
            {"id": 2, "name": "Bob", "role": "user"}
        ]"#;
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        let result = toon_encode_array(&value);
        assert!(result.is_some());
        let encoded = result.unwrap();
        assert!(
            encoded.contains("[2]{id,name,role}:"),
            "should have header: {encoded}"
        );
        assert!(encoded.contains("1,Alice,admin"));
        assert!(encoded.contains("2,Bob,user"));
    }

    #[test]
    fn encode_non_uniform_array_returns_none() {
        let json = r#"[{"id": 1}, {"name": "Bob"}]"#;
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(toon_encode_array(&value).is_none());
    }

    #[test]
    fn encode_nested_values_returns_none() {
        let json = r#"[{"id": 1, "meta": {"x": 1}}]"#;
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(toon_encode_array(&value).is_none());
    }

    #[test]
    fn encode_empty_array_returns_none() {
        let value = serde_json::json!([]);
        assert!(toon_encode_array(&value).is_none());
    }

    #[test]
    fn encode_single_element() {
        let json = r#"[{"id": 1, "name": "Alice"}]"#;
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        let result = toon_encode_array(&value);
        assert!(result.is_some());
        let encoded = result.unwrap();
        assert!(encoded.contains("[1]{id,name}:"));
    }

    #[test]
    fn encode_values_with_commas_quoted() {
        let json = r#"[{"name": "Alice, Jr.", "age": 30}]"#;
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        let result = toon_encode_array(&value);
        assert!(result.is_some());
        let encoded = result.unwrap();
        assert!(
            encoded.contains("\"Alice, Jr.\""),
            "comma in value should be quoted: {encoded}"
        );
    }

    #[test]
    fn toon_prompt_instructions_not_empty() {
        let instructions = toon_format_instructions();
        assert!(!instructions.is_empty());
        assert!(instructions.contains("TOON"));
    }

    #[test]
    fn try_toon_encode_text_with_json_array() {
        let input = "Exit code: 0\nSTDOUT:\n[{\"id\":1,\"name\":\"Alice\",\"role\":\"admin\"},{\"id\":2,\"name\":\"Bob\",\"role\":\"user\"}]\nSTDERR:\n";
        let result = try_toon_encode(input);
        assert!(
            result.contains("[2]{id,name,role}:"),
            "should contain TOON header: {result}"
        );
        assert!(result.contains("Exit code: 0"), "should preserve prefix");
    }
}
