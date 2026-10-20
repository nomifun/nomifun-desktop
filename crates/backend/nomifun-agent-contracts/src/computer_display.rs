//! Read-only Computer display facts derived from the frozen action surface.
//! These helpers grant no authority and perform no desktop operation.
use serde::Deserialize;
use serde_json::Value;
use std::sync::OnceLock;

#[derive(Deserialize)]
struct FrozenActions { tools: Vec<FrozenAction> }
#[derive(Deserialize)]
struct FrozenAction { action_id: String, driver_tool: String, model_tool: String }

fn actions() -> &'static [FrozenAction] {
    static ACTIONS: OnceLock<FrozenActions> = OnceLock::new();
    &ACTIONS.get_or_init(|| serde_json::from_str(include_str!("../contracts/computer-tools.v1.json"))
        .expect("the generated Computer surface must be valid")).tools
}

pub fn computer_driver_tool(action_id: &str) -> Option<&'static str> {
    actions().iter().find(|tool| tool.action_id == action_id).map(|tool| tool.driver_tool.as_str())
}

pub fn computer_action_uses_foreground(action_id: &str, arguments: &Value) -> bool {
    computer_driver_tool(action_id).is_some_and(|driver|
        driver == "bring_to_front" || computer_arguments_use_foreground(arguments))
}

/// Shared by worker preparation and typed display projection, including batches.
pub fn computer_arguments_use_foreground(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            (key == "delivery_mode" && value == "foreground")
                || (key == "foreground_fallback" && value == &Value::Bool(true))
                || (key == "kind" && value == "desktop")
                || (key == "tool" && value.as_str().is_some_and(|name| actions().iter().any(|tool|
                    (name == tool.action_id || name == tool.driver_tool || name == tool.model_tool)
                        && tool.driver_tool == "bring_to_front")))
                || computer_arguments_use_foreground(value)
        }),
        Value::Array(items) => items.iter().any(computer_arguments_use_foreground),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn driver_identity_requires_the_exact_frozen_action() {
        let action = actions().iter().find(|tool| tool.driver_tool == "click").unwrap();
        assert_eq!(computer_driver_tool(&action.action_id), Some(action.driver_tool.as_str()));
        assert!(computer_driver_tool(&action.driver_tool).is_none());
        assert!(computer_driver_tool("unregistered/action").is_none());
    }

    #[test]
    fn foreground_matches_explicit_delivery_desktop_and_nested_batch_inputs() {
        assert!(computer_arguments_use_foreground(&json!({"delivery_mode":"foreground"})));
        assert!(computer_arguments_use_foreground(&json!({"foreground_fallback":true})));
        assert!(computer_arguments_use_foreground(&json!({"steps":[{"args":{"target":{"kind":"desktop"}}}]})));
        assert!(!computer_arguments_use_foreground(&json!({"delivery_mode":"background","text":"{\"delivery_mode\":\"foreground\"}"})));
        assert!(!computer_action_uses_foreground("unregistered/action", &json!({"delivery_mode":"foreground"})));
    }

    #[test]
    fn intrinsic_foreground_batch_tools_come_from_frozen_identities() {
        let action = actions().iter().find(|tool| tool.driver_tool == "bring_to_front").unwrap();
        assert!(computer_action_uses_foreground(&action.action_id, &json!({})));
        for name in [&action.action_id, &action.driver_tool, &action.model_tool] {
            assert!(computer_arguments_use_foreground(&json!({"steps":[{"tool":name,"args":{}}]})));
        }
    }
}
