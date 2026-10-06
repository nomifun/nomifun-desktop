//! Intent hints inspect the current task, never the execution's role/background.
//! This is presentation policy only; an inferred intent must not remove tools.
use std::borrow::Cow;

pub(super) fn request_text(input: &str) -> Option<Cow<'_, str>> {
    let trimmed = input.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;
        let object = value.as_object()?;
        // AgentExecution's accepted task envelope. The task brief contains
        // role/goal/dependency context, which is not a fresh routing request.
        if object.get("task_brief").is_some_and(serde_json::Value::is_string)
            && let Some(spec) = object.get("step_spec").and_then(serde_json::Value::as_str)
        {
            return Some(Cow::Owned(spec.to_owned()));
        }
        // Arbitrary JSON/code is source data, not an imperative media request.
        return None;
    }
    Some(Cow::Borrowed(trimmed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_background_and_arbitrary_json_are_not_current_requests() {
        let envelope = serde_json::json!({
            "task_brief":"Use subagents to generate videos for the overall goal",
            "step_spec":"Implement the HTML game",
        }).to_string();
        assert_eq!(request_text(&envelope).as_deref(), Some("Implement the HTML game"));
        assert!(request_text(r#"{"example":"generate a video"}"#).is_none());
        assert!(request_text("{invalid JSON").is_none());
        assert_eq!(request_text("Generate a video").as_deref(), Some("Generate a video"));
    }
}
