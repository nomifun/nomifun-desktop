//! Shared callback-data encoding for interactive buttons across channels.
//!
//! Channels that support inline buttons (Telegram, Discord, Slack, QQ Bot)
//! encode an [`ActionButton`] into a compact `custom_id`/`callback_data` string
//! `"category:action"` or `"category:action:k=v,k=v"`, and decode the reverse
//! when the user clicks. Only the current Channel action surface is accepted;
//! unknown or stale callbacks are rejected before they can become a unified
//! inbound action.

use std::collections::HashMap;

use crate::types::{ActionButton, ActionCategory};

/// A decoded callback payload from an interactive button.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCallback {
    pub category: ActionCategory,
    pub action: String,
    pub params: Option<HashMap<String, String>>,
}

/// Returns whether an action is allowed on the current Channel callback wire.
///
/// Ordinary product namespaces remain open so a newly added Channel action
/// does not require a transport parser release. The retired Agent namespace
/// and confirmation verb are rejected before a callback can become a unified
/// inbound action.
pub fn is_supported_callback_action(action: &str) -> bool {
    let Some((namespace, verb)) = action.split_once('.') else {
        return false;
    };
    matches!(
        namespace,
        "pairing" | "session" | "help" | "settings" | "chat" | "action" | "confirm"
    ) && !(namespace == "system" && verb == "confirm")
}

/// Derive the category prefix from an action name, matching `ActionExecutor`
/// routing:
///   - `pairing.*` → `"platform"`
///   - `chat.*` / `action.*` → `"chat"`
///   - everything else (`session.*`, `help.*`, `agent.*`, `system.*`, ...) → `"system"`
pub fn action_category_prefix(action: &str) -> &'static str {
    match action.split('.').next().unwrap_or("") {
        "pairing" => "platform",
        "chat" | "action" => "chat",
        _ => "system",
    }
}

/// Encode an [`ActionButton`] into `"category:action"` or
/// `"category:action:k=v,k=v"`. Inverse of [`parse_callback_data`].
pub fn format_callback_data(btn: &ActionButton) -> String {
    let category = action_category_prefix(&btn.action);
    match &btn.params {
        Some(params) if !params.is_empty() => {
            let encoded: Vec<String> = params.iter().map(|(k, v)| format!("{k}={v}")).collect();
            format!("{category}:{}:{}", btn.action, encoded.join(","))
        }
        _ => format!("{category}:{}", btn.action),
    }
}

/// Parse a callback string `"category:action"` or `"category:action:k=v,k=v"`.
/// Returns `None` for malformed input or an unknown category.
pub fn parse_callback_data(data: &str) -> Option<ParsedCallback> {
    let parts: Vec<&str> = data.splitn(3, ':').collect();
    if parts.len() < 2 {
        return None;
    }
    let category = match parts[0] {
        "platform" => ActionCategory::Platform,
        "system" => ActionCategory::System,
        "chat" => ActionCategory::Chat,
        _ => return None,
    };
    let action = parts[1].trim();
    if action.is_empty() || !is_supported_callback_action(action) {
        return None;
    }
    let action = action.to_owned();
    let params = if parts.len() == 3 && !parts[2].is_empty() {
        let mut map = HashMap::new();
        for pair in parts[2].split(',') {
            if let Some((k, v)) = pair.split_once('=') {
                map.insert(k.to_string(), v.to_string());
            }
        }
        if map.is_empty() { None } else { Some(map) }
    } else {
        None
    };
    Some(ParsedCallback {
        category,
        action,
        params,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_prefix_mapping() {
        assert_eq!(action_category_prefix("pairing.show"), "platform");
        assert_eq!(action_category_prefix("chat.regenerate"), "chat");
        assert_eq!(action_category_prefix("action.copy"), "chat");
        assert_eq!(action_category_prefix("session.new"), "system");
        assert_eq!(action_category_prefix("confirm.yes"), "system");
    }

    #[test]
    fn format_no_params() {
        let btn = ActionButton {
            label: "Help".into(),
            action: "help.show".into(),
            params: None,
        };
        assert_eq!(format_callback_data(&btn), "system:help.show");
    }

    #[test]
    fn format_with_params() {
        let btn = ActionButton {
            label: "Continue".into(),
            action: "chat.continue".into(),
            params: Some(HashMap::from([("sessionId".into(), "abc".into())])),
        };
        let s = format_callback_data(&btn);
        assert!(s.starts_with("chat:chat.continue:"));
        assert!(s.contains("sessionId=abc"));
    }

    #[test]
    fn parse_category_action() {
        let p = parse_callback_data("system:session.new").unwrap();
        assert_eq!(p.category, ActionCategory::System);
        assert_eq!(p.action, "session.new");
        assert!(p.params.is_none());
    }

    #[test]
    fn parse_with_params() {
        let p = parse_callback_data("chat:chat.continue:sessionId=abc").unwrap();
        assert_eq!(p.action, "chat.continue");
        let params = p.params.unwrap();
        assert_eq!(params.get("sessionId").unwrap(), "abc");
    }

    #[test]
    fn parse_invalid() {
        assert!(parse_callback_data("nope").is_none());
        assert!(parse_callback_data("unknown:action").is_none());
    }

    #[test]
    fn unsupported_callbacks_are_rejected() {
        for data in [
            "system:unknown.switch:value=yes",
            "system:confirm:value=yes",
        ] {
            assert!(
                parse_callback_data(data).is_none(),
                "unsupported callback must be rejected: {data}"
            );
        }
    }

    #[test]
    fn channel_stop_callbacks_remain_supported() {
        for data in ["system:confirm.yes", "system:confirm.no"] {
            let parsed = parse_callback_data(data).expect("channel stop callback");
            assert_eq!(parsed.category, ActionCategory::System);
        }
    }
}
