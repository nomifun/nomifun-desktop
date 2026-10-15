//! Pure configuration-time conversion between the two OpenAI Chat protocols.
//! This never selects a runtime route or discards an unresolved semantic policy.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const CHAT: &str = "openai.chat_text";
const RESPONSES: &str = "openai.responses";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatProtocolTransitionIssueKind { Conflict, Unsupported }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatProtocolTransitionIssue {
    pub parameter: String,
    pub kind: ChatProtocolTransitionIssueKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatProtocolTransition {
    pub params: Map<String, Value>,
    /// Fields moved out under the project's target protocol schema, retained
    /// for editors (including the current Chat usage-stream controls).
    /// Callers must not send these fields to the target protocol.
    pub suspended: Map<String, Value>,
    /// A detector must resolve every issue before testing the converted request.
    pub issues: Vec<ChatProtocolTransitionIssue>,
}

/// Convert only documented equivalent shapes. Unknown vendor parameters and
/// `thinking` remain untouched. A conflict retains both original choices.
pub fn transition_chat_provider_params(params: &Map<String, Value>, source: &str, target: &str) -> ChatProtocolTransition {
    let mut result = ChatProtocolTransition {params:params.clone(),suspended:Map::new(),issues:Vec::new()};
    let (source,target) = (source.trim(),target.trim());
    if !matches!(source,CHAT | RESPONSES) || !matches!(target,CHAT | RESPONSES) { return result; }
    normalize_reasoning_effort(&mut result);
    if source == target { return result; }
    for key in if target == RESPONSES { ["stream_options","require_reasoning_content"] } else { ["include","chain_rounds"] } {
        suspend(&mut result,key);
    }
    let standard_alias_is_inapplicable = result.params.get("max_tokens_field").and_then(Value::as_str)
        .is_some_and(|field| match field.trim() {
            "max_tokens" | "max_completion_tokens" => target == RESPONSES,
            "max_output_tokens" => target == CHAT,
            _ => false,
        });
    if standard_alias_is_inapplicable { suspend(&mut result,"max_tokens_field"); }
    if target == RESPONSES { chat_to_responses(&mut result); }
    else { responses_to_chat(&mut result); }
    result
}

fn issue(result: &mut ChatProtocolTransition, parameter: &str, kind: ChatProtocolTransitionIssueKind) {
    result.issues.push(ChatProtocolTransitionIssue {parameter:parameter.into(),kind});
}

fn suspend(result: &mut ChatProtocolTransition, key: &str) {
    if let Some(value) = result.params.remove(key) { result.suspended.insert(key.into(),value); }
}

fn empty_policy(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Object(value) => value.is_empty(),
        _ => false,
    }
}

// Object-key order and an integer's ordinary JSON decimal spelling do not
// change a policy. Keep integer comparisons exact before considering f64;
// converting large integers to f64 would merge distinct schema boundaries.
fn semantically_equal(left: &Value, right: &Value) -> bool {
    match (left,right) {
        (Value::Object(left),Value::Object(right)) => left.len() == right.len()
            && left.iter().all(|(key,value)| right.get(key).is_some_and(|other| semantically_equal(value,other))),
        (Value::Array(left),Value::Array(right)) => left.len() == right.len()
            && left.iter().zip(right).all(|(left,right)| semantically_equal(left,right)),
        (Value::Number(left),Value::Number(right)) => {
            if left == right { return true; }
            let (integer,decimal) = match (left.is_f64(),right.is_f64()) {
                (false,true) => (left,right),
                (true,false) => (right,left),
                _ => return false,
            };
            const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
            let safe = integer.as_i64().is_some_and(|value| (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value))
                || integer.as_u64().is_some_and(|value| value <= MAX_SAFE_INTEGER as u64);
            safe && integer.as_f64() == decimal.as_f64()
        }
        _ => left == right,
    }
}

fn normalize_reasoning_effort(result: &mut ChatProtocolTransition) {
    if result.params.get("reasoning").is_some_and(empty_policy) {
        suspend(result,"reasoning");
        return;
    }
    let Some(effort) = result.params.get("reasoning").and_then(Value::as_object)
        .and_then(|reasoning| reasoning.get("effort")).cloned() else { return; };
    if result.params.get("reasoning_effort").is_some_and(|current| !semantically_equal(current,&effort)) {
        issue(result,"reasoning.effort",ChatProtocolTransitionIssueKind::Conflict);
        return;
    }
    result.params.entry("reasoning_effort").or_insert(effort);
    let reasoning = result.params.get_mut("reasoning").and_then(Value::as_object_mut).expect("reasoning object");
    reasoning.remove("effort");
    if reasoning.is_empty() { result.params.remove("reasoning"); }
}

fn chat_format_to_responses(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    match object.get("type")?.as_str()? {
        "text" | "json_object" => Some(value.clone()),
        "json_schema" => {
            if object.len() != 2 { return None; }
            let mut format = object.get("json_schema")?.as_object()?.clone();
            // Flattening must not replace a vendor-owned field in the wrapper.
            if format.contains_key("type") { return None; }
            format.insert("type".into(),Value::String("json_schema".into()));
            Some(Value::Object(format))
        }
        _ => None,
    }
}

fn responses_format_to_chat(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    match object.get("type")?.as_str()? {
        "text" | "json_object" => Some(value.clone()),
        "json_schema" => {
            let mut schema = object.clone();
            schema.remove("type");
            Some(serde_json::json!({"type":"json_schema","json_schema":schema}))
        }
        _ => None,
    }
}

fn merge_text_field(result: &mut ChatProtocolTransition, source: &str, field: &str, value: Value) -> bool {
    if result.params.get("text").is_some_and(empty_policy) { suspend(result,"text"); }
    if result.params.get("text").is_some_and(|text| !text.is_object()) {
        issue(result,source,ChatProtocolTransitionIssueKind::Unsupported);
        return false;
    }
    let text = result.params.entry("text").or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut().expect("text object");
    if let Some(current) = text.get(field) {
        if !semantically_equal(current,&value) {
            issue(result,source,ChatProtocolTransitionIssueKind::Conflict);
            return false;
        }
        return true;
    }
    text.insert(field.into(),value);
    true
}

fn chat_to_responses(result: &mut ChatProtocolTransition) {
    if result.params.get("text").is_some_and(empty_policy) { suspend(result,"text"); }
    if let Some(format) = result.params.get("response_format").cloned() {
        if empty_policy(&format) { suspend(result,"response_format"); }
        else if let Some(converted) = chat_format_to_responses(&format) {
            if merge_text_field(result,"response_format","format",converted) { result.params.remove("response_format"); }
        } else { issue(result,"response_format",ChatProtocolTransitionIssueKind::Unsupported); }
    }
    if let Some(verbosity) = result.params.get("verbosity").cloned() {
        if merge_text_field(result,"verbosity","verbosity",verbosity) { result.params.remove("verbosity"); }
    }
}

fn merge_top_field(result: &mut ChatProtocolTransition, source: &str, field: &str, value: Value) -> bool {
    if let Some(current) = result.params.get(field) {
        if !semantically_equal(current,&value) {
            issue(result,source,ChatProtocolTransitionIssueKind::Conflict);
            return false;
        }
        return true;
    }
    result.params.insert(field.into(),value);
    true
}

fn suspend_text_field(result: &mut ChatProtocolTransition, field: &str, value: Value) {
    result.suspended.entry("text").or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut().expect("suspended text object").insert(field.into(),value);
}

fn responses_to_chat(result: &mut ChatProtocolTransition) {
    if let Some(text) = result.params.get("text").cloned() {
        if empty_policy(&text) { suspend(result,"text"); }
        else if let Some(mut text) = text.as_object().cloned() {
            if let Some(format) = text.get("format").cloned() {
                if empty_policy(&format) {
                    suspend_text_field(result,"format",format);
                    text.remove("format");
                } else if let Some(converted) = responses_format_to_chat(&format) {
                    if merge_top_field(result,"text.format","response_format",converted) { text.remove("format"); }
                } else { issue(result,"text.format",ChatProtocolTransitionIssueKind::Unsupported); }
            }
            if let Some(verbosity) = text.get("verbosity").cloned() {
                if merge_top_field(result,"text.verbosity","verbosity",verbosity) { text.remove("verbosity"); }
            }
            if text.keys().any(|key| !matches!(key.as_str(),"format" | "verbosity")) {
                issue(result,"text",ChatProtocolTransitionIssueKind::Unsupported);
            }
            if text.is_empty() { result.params.remove("text"); }
            else { result.params.insert("text".into(),Value::Object(text)); }
        } else { issue(result,"text",ChatProtocolTransitionIssueKind::Unsupported); }
    }
    if let Some(reasoning) = result.params.get("reasoning") {
        if empty_policy(reasoning) { suspend(result,"reasoning"); }
        else if reasoning.as_object().is_none_or(|reasoning| reasoning.keys().any(|key| key != "effort")) {
            issue(result,"reasoning",ChatProtocolTransitionIssueKind::Unsupported);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_numeric_equality_does_not_merge_large_integer_schema_boundaries() {
        let parse = |value: &str| serde_json::from_str::<Value>(value).unwrap();
        assert!(semantically_equal(&parse(r#"{"minimum":1,"enum":[-2,3]}"#),
            &parse(r#"{"enum":[-2.0,3.0],"minimum":1.0}"#)));
        assert!(!semantically_equal(&parse("9007199254740992"),&parse("9007199254740993")));
        assert!(!semantically_equal(&parse("9007199254740993"),&parse("9007199254740992.0")));
        assert!(!semantically_equal(&parse("18446744073709551614"),&parse("18446744073709551615")));
    }
}
