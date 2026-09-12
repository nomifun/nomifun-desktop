use super::*;
use serde_json::json;

#[test]
fn schema_normalization_preserves_keyword_named_properties() {
    let schema = json!({
        "type": "object",
        "properties": {
            "additionalProperties": {"type": ["string", "null"]},
            "type": {"type": "string"},
            "$ref": {"type": "string"}
        },
        "required": ["additionalProperties"],
        "additionalProperties": false
    });
    for sanitize in [sanitize_json_schema, sanitize_json_schema_for_gemini] {
        let actual = sanitize(&schema);
        assert_eq!(
            actual["properties"]["additionalProperties"]["type"],
            "string"
        );
        assert_eq!(actual["properties"]["type"], schema["properties"]["type"]);
        assert_eq!(actual["properties"]["$ref"], schema["properties"]["$ref"]);
        assert!(actual.get("additionalProperties").is_none());
    }
}

#[test]
fn schema_normalization_does_not_rewrite_literal_data() {
    let literal = json!({"type": ["string", "null"], "additionalProperties": true});
    let schema = json!({
        "type": "object",
        "properties": {"payload": {"const": literal}},
        "examples": [literal]
    });
    for sanitize in [sanitize_json_schema, sanitize_json_schema_for_gemini] {
        let actual = sanitize(&schema);
        assert_eq!(actual["properties"]["payload"]["const"], literal);
        assert_eq!(actual["examples"], schema["examples"]);
    }
}
