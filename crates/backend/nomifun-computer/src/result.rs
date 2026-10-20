//! Driver evidence projection; never infer execution from diagnostic prose.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultEffectOutcome {
    Succeeded,
    Failed,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultPart {
    Text(String),
    Image { data: String, mime_type: String },
}
#[derive(Debug, Clone)]
pub struct NormalizedResult {
    pub parts: Vec<ResultPart>,
    pub is_error: bool,
    pub structured_content: Option<Value>,
    pub effect: ResultEffectOutcome,
}
pub fn normalize(raw: &Value, image_enabled: bool) -> NormalizedResult {
    let mut structured = raw.get("structuredContent").cloned();
    if let Some(value) = &mut structured {
        rewrite_next_call(value);
        if !image_enabled {
            strip_images(value);
        }
    }
    let mut parts = Vec::new();
    let mut text_evidence = None;
    if let Some(content) = raw.get("content").and_then(Value::as_array) {
        for item in content {
            match item.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        // JSON content is an explicit driver payload, not free-form prose.
                        let text = if let Ok(mut value) = serde_json::from_str::<Value>(text) {
                            rewrite_next_call(&mut value);
                            if !image_enabled {
                                strip_images(&mut value);
                            }
                            if value.get("effect").is_some()
                                || value.get("status").is_some()
                                || value.get("refusal").is_some()
                            {
                                text_evidence = Some(value.clone());
                            }
                            value.to_string()
                        } else {
                            text.to_owned()
                        };
                        parts.push(ResultPart::Text(text));
                    }
                }
                Some("image") if image_enabled => {
                    if let (Some(data), Some(mime)) = (
                        item.get("data").and_then(Value::as_str),
                        item.get("mimeType").and_then(Value::as_str),
                    ) {
                        parts.push(ResultPart::Image {
                            data: data.to_owned(),
                            mime_type: mime.to_owned(),
                        });
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(value) = &structured {
        let text = value.to_string();
        if !parts
            .iter()
            .any(|part| matches!(part,ResultPart::Text(existing)if existing==&text))
        {
            parts.push(ResultPart::Text(text));
        }
    }
    if parts.is_empty() {
        parts.push(ResultPart::Text(if raw.get("content").is_some() {
            "No text content; image parts are omitted for this route.".into()
        } else {
            raw.to_string()
        }));
    }
    let is_error = raw.get("isError").and_then(Value::as_bool).unwrap_or(false);
    let evidence = structured
        .as_ref()
        .or(text_evidence.as_ref())
        .unwrap_or(raw);
    let marker = evidence
        .get("effect")
        .and_then(Value::as_str)
        .or_else(|| evidence.get("status").and_then(Value::as_str));
    let effect = match marker {
        Some("confirmed" | "partial" | "unverifiable" | "suspected_noop") => {
            ResultEffectOutcome::Succeeded
        }
        Some("refused" | "not_started") => ResultEffectOutcome::Failed,
        Some("unknown") => ResultEffectOutcome::Unknown,
        _ if evidence.get("refusal").is_some() => ResultEffectOutcome::Failed,
        _ if evidence.get("code").and_then(Value::as_str) == Some("invalid_batch")
            && evidence.get("executed").and_then(Value::as_u64) == Some(0) =>
        {
            ResultEffectOutcome::Failed
        }
        _ if batch_effect(evidence).is_some() => batch_effect(evidence).unwrap(),
        _ if is_error => ResultEffectOutcome::Unknown,
        _ => ResultEffectOutcome::Succeeded,
    };
    let is_error = is_error || effect != ResultEffectOutcome::Succeeded;
    NormalizedResult {
        parts,
        is_error,
        structured_content: structured,
        effect,
    }
}
fn batch_effect(evidence: &Value) -> Option<ResultEffectOutcome> {
    let steps = evidence.get("steps")?.as_array()?;
    let executed = evidence.get("executed")?.as_u64()?;
    if executed != steps.len() as u64 || evidence.get("total")?.as_u64()? < executed {
        return None;
    }
    let mut delivered = false;
    for step in steps {
        if step.get("skipped").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let action = step
            .get("tool")
            .and_then(Value::as_str)
            .and_then(crate::surface::tool)
            .is_some_and(|tool| matches!(tool.effect_class.as_str(), "Physical" | "ExecuteLocal"));
        let succeeded = step.get("ok").and_then(Value::as_bool)?;
        let phase = step.get("phase").and_then(Value::as_str);
        if !succeeded && phase == Some("action") {
            return Some(ResultEffectOutcome::Unknown);
        }
        if action && (succeeded || phase == Some("expect")) {
            delivered = true;
        }
        if !succeeded && !matches!(phase, Some("wait_for" | "find" | "expect" | "time_budget")) {
            return Some(ResultEffectOutcome::Unknown);
        }
    }
    Some(
        if delivered || evidence.get("ok").and_then(Value::as_bool) == Some(true) {
            ResultEffectOutcome::Succeeded
        } else {
            ResultEffectOutcome::Failed
        },
    )
}
fn strip_images(value: &mut Value) {
    match value {
        Value::Array(items) => {
            items.retain(|item| item.get("type").and_then(Value::as_str) != Some("image"));
            for item in items {
                strip_images(item);
            }
        }
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("image") {
                *value = Value::Null;
            } else {
                for value in object.values_mut() {
                    strip_images(value);
                }
            }
        }
        _ => {}
    }
}
pub fn rewrite_next_call(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(next) = object.get_mut("next_call").and_then(Value::as_object_mut) {
                if let Some(name) = next
                    .get("tool")
                    .and_then(Value::as_str)
                    .and_then(crate::surface::tool)
                    .map(|tool| tool.model_tool.clone())
                {
                    next.insert("tool".into(), json!(name));
                }
            }
            for value in object.values_mut() {
                rewrite_next_call(value);
            }
        }
        Value::Array(items) => {
            for value in items {
                rewrite_next_call(value);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_parts_keep_images_out_of_no_image_routes() {
        let raw = json!({"content":[{"type":"text","text":"read"},{"type":"image","data":"AA==","mimeType":"image/png"}],"structuredContent":{"next_call":{"tool":"get_window_state","arguments":{}}},"isError":false});
        let full = normalize(&raw, true);
        assert!(matches!(&full.parts[1],ResultPart::Image{mime_type,..}if mime_type=="image/png"));
        assert_eq!(
            full.structured_content.unwrap()["next_call"]["tool"],
            "computer_get_window_state"
        );
        let reduced = normalize(&raw, false);
        assert!(
            reduced
                .parts
                .iter()
                .all(|part| !matches!(part, ResultPart::Image { .. }))
        );
        let only_image = normalize(
            &json!({"content":[{"type":"image","data":"secret-image","mimeType":"image/png"}]}),
            false,
        );
        assert!(!format!("{:?}", only_image.parts).contains("secret-image"));
    }
    #[test]
    fn effect_evidence_is_not_confused_with_transport_or_tool_success() {
        for effect in ["confirmed", "partial", "unverifiable", "suspected_noop"] {
            assert_eq!(
                normalize(
                    &json!({"isError":true,"structuredContent":{"effect":effect}}),
                    true
                )
                .effect,
                ResultEffectOutcome::Succeeded
            );
        }
        assert_eq!(normalize(&json!({"isError":true,"structuredContent":{"status":"refused","refusal":{"code":"stale"}}}),true).effect,ResultEffectOutcome::Failed);
        assert_eq!(
            normalize(&json!({"isError":true}), true).effect,
            ResultEffectOutcome::Unknown
        );
    }

    #[test]
    fn batch_rejection_partial_delivery_and_unknown_action_remain_distinct() {
        assert_eq!(
            normalize(
                &json!({"isError":true,"structuredContent":{"code":"invalid_batch","executed":0}}),
                true
            )
            .effect,
            ResultEffectOutcome::Failed
        );
        let after_action = json!({"isError":true,"structuredContent":{"ok":false,"total":2,"executed":1,"steps":[{"tool":"click","ok":false,"phase":"expect"}]}});
        assert_eq!(
            normalize(&after_action, true).effect,
            ResultEffectOutcome::Succeeded
        );
        let before_action = json!({"isError":true,"structuredContent":{"ok":false,"total":1,"executed":1,"steps":[{"tool":"click","ok":false,"phase":"find"}]}});
        assert_eq!(
            normalize(&before_action, true).effect,
            ResultEffectOutcome::Failed
        );
        let uncertain = json!({"isError":true,"structuredContent":{"ok":false,"total":1,"executed":1,"steps":[{"tool":"click","ok":false,"phase":"action"}]}});
        assert_eq!(
            normalize(&uncertain, true).effect,
            ResultEffectOutcome::Unknown
        );
    }
    #[test]
    fn explicit_json_refusal_in_text_is_evidence_and_nested_images_are_not_text() {
        let text=json!({"status":"refused","refusal":{"code":"not_executed"},"next_call":{"tool":"get_window_state"}}).to_string();
        assert_eq!(
            normalize(
                &json!({"isError":true,"content":[{"type":"text","text":text}]}),
                false
            )
            .effect,
            ResultEffectOutcome::Failed
        );
        let projected = normalize(
            &json!({"structuredContent":{"content":[{"type":"image","data":"private-pixels","mimeType":"image/png"}]}}),
            false,
        );
        assert!(!format!("{:?}", projected.parts).contains("private-pixels"));
    }
    #[test]
    fn typed_failed_or_unknown_evidence_stops_the_model_batch_even_without_is_error() {
        for raw in [
            json!({"structuredContent":{"effect":"unknown"}}),
            json!({"isError":false,"structuredContent":{"status":"refused","refusal":{"code":"not_started"}}}),
        ] {
            assert!(normalize(&raw, true).is_error);
        }
        let delivered = json!({"isError":true,"structuredContent":{"effect":"partial"}});
        let result = normalize(&delivered, true);
        assert_eq!(result.effect, ResultEffectOutcome::Succeeded);
        assert!(result.is_error);
    }
}
