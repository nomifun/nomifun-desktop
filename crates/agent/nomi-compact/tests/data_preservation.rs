use nomi_compact::{CompactionLevel, compact_output, compact_output_toon};
use serde_json::{Value, json};

#[test]
fn safe_keeps_windows_lines_and_progress_output() {
    assert_eq!(
        compact_output("first\r\nsecond\r\n", CompactionLevel::Safe),
        "first\nsecond"
    );
    assert_eq!(
        compact_output("10%\r100%\r\nDone", CompactionLevel::Safe),
        "100%\nDone"
    );
    assert_eq!(
        nomi_compact::sanitize::collapse_cr_lines("\nfirst\n"),
        "\nfirst\n"
    );
}

#[test]
fn json_compaction_preserves_escaped_keys() {
    let value = json!({"quoted\"key\n\\": "x".repeat(100), "nested": {"hello": 1}});
    let input = serde_json::to_string_pretty(&value)
        .unwrap()
        .replace("  ", "    ");
    let result = nomi_compact::json::compact_json(&input);
    assert!(result.len() < input.len());
    assert_eq!(serde_json::from_str::<Value>(&result).unwrap(), value);
}

#[test]
fn full_does_not_fold_json_properties() {
    let value = json!({"common_field_001": 1, "common_field_002": 2, "common_field_003": 3, "common_field_004": 4});
    let input = serde_json::to_string_pretty(&value).unwrap();
    let result = compact_output(&input, CompactionLevel::Full);
    assert_eq!(serde_json::from_str::<Value>(&result).unwrap(), value);
    let prefixed = format!("log-0\nlog-1\nlog-2\n{input}\n{input}");
    let result = compact_output(&prefixed, CompactionLevel::Full);
    assert!(
        result.ends_with(&input),
        "a second JSON block must not be folded: {result}"
    );
}

#[test]
fn unicode_repeated_lines_fold_like_ascii() {
    let result = nomi_compact::fold::fold_repeated_lines("完成\n完成\n完成\n完成");
    assert!(result.contains("[... 2 identical lines]"), "{result}");
}

#[test]
fn toon_strings_keep_types_and_escape_sequences() {
    for value in [
        "", "true", "null", "42", "01", "1e3", " x ", "a\nb", "a\rb", "a\tb", "a\\b", "a\"b",
    ] {
        let encoded = nomi_compact::toon::toon_encode_array(&json!([{"value": value}])).unwrap();
        assert_eq!(
            encoded,
            format!("[1]{{value}}:\n  {}", serde_json::to_string(value).unwrap()),
            "{value:?}"
        );
    }
}

#[test]
fn toon_quotes_special_field_names() {
    let value = json!([{"a,b": "ok"}]);
    assert_eq!(
        nomi_compact::toon::toon_encode_array(&value).unwrap(),
        "[1]{\"a,b\"}:\n  ok"
    );
}

#[test]
fn toon_parses_brackets_inside_strings_and_preserves_surrounding_text() {
    let input = "\nSTDOUT:\n[{\"name\":\"] Alice\"}]\nSTDERR:\n";
    assert_eq!(
        compact_output_toon(input),
        "\nSTDOUT:\n[1]{name}:\n  \"] Alice\"\nSTDERR:\n"
    );
}
