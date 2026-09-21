//! Conservative classification of provider-declared unsupported technical
//! capabilities. Only machine-readable error fields participate; diagnostic
//! prose is never evidence.

use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderTechnicalCapability {
    FunctionCalling,
    Reasoning,
    Streaming,
}

pub fn classify_unsupported_technical_capability_body(
    body: &[u8],
) -> Option<ProviderTechnicalCapability> {
    let value = serde_json::from_slice::<Value>(body).ok()?;
    let object = value
        .pointer("/response/error")
        .or_else(|| value.get("error"))
        .unwrap_or(&value)
        .as_object()?;
    classify_unsupported_technical_capability(object)
}

pub fn classify_unsupported_technical_capability(
    error: &Map<String, Value>,
) -> Option<ProviderTechnicalCapability> {
    let normalized = |field: &str| {
        error
            .get(field)
            .and_then(Value::as_str)
            .map(|value| value.trim().to_ascii_lowercase().replace('-', "_"))
    };
    let codes = ["code", "type", "status"]
        .into_iter()
        .filter_map(normalized)
        .collect::<Vec<_>>();
    let direct = |values: &[&str]| {
        codes
            .iter()
            .any(|code| values.iter().any(|expected| code == expected))
    };
    if direct(&[
        "function_calling_not_supported",
        "tool_calls_not_supported",
        "tools_not_supported",
        "unsupported_function_calling",
        "unsupported_tool_calls",
        "unsupported_tools",
    ]) {
        return Some(ProviderTechnicalCapability::FunctionCalling);
    }
    if direct(&[
        "reasoning_not_supported",
        "thinking_not_supported",
        "unsupported_reasoning",
        "unsupported_thinking",
    ]) {
        return Some(ProviderTechnicalCapability::Reasoning);
    }
    if direct(&[
        "streaming_not_supported",
        "stream_not_supported",
        "unsupported_streaming",
        "unsupported_stream",
    ]) {
        return Some(ProviderTechnicalCapability::Streaming);
    }
    if !direct(&[
        "unsupported_parameter",
        "unsupported_feature",
        "unsupported_value",
        "not_supported",
    ]) {
        return None;
    }
    match ["param", "parameter", "field"]
        .into_iter()
        .find_map(normalized)?
        .as_str()
    {
        "tools" | "tool_choice" | "functions" | "function_call" | "function_calling" => {
            Some(ProviderTechnicalCapability::FunctionCalling)
        }
        "reasoning" | "reasoning_effort" | "thinking" => {
            Some(ProviderTechnicalCapability::Reasoning)
        }
        "stream" | "stream_options" | "streaming" => {
            Some(ProviderTechnicalCapability::Streaming)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_machine_fields_are_required() {
        assert_eq!(
            classify_unsupported_technical_capability_body(
                br#"{"error":{"code":"unsupported_parameter","param":"tools","message":"ignored"}}"#,
            ),
            Some(ProviderTechnicalCapability::FunctionCalling)
        );
        for body in [
            br#"{"error":{"code":"invalid_request_error","param":"tools","message":"tools unsupported"}}"#.as_slice(),
            br#"{"error":{"code":"unsupported_parameter","param":"temperature"}}"#.as_slice(),
            br#"{"message":"streaming not supported"}"#.as_slice(),
        ] {
            assert_eq!(classify_unsupported_technical_capability_body(body), None);
        }
    }
}
