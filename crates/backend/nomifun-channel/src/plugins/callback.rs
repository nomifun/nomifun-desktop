//! Shared callback-data encoding for interactive buttons across channels.
//!
//! Channels that support inline buttons (Telegram, Discord, Slack, Mattermost)
//! encode an [`ActionButton`] into a compact `custom_id`/`callback_data` string
//! `"category:action"` or `"category:action:k=v,k=v"`, and decode the reverse
//! when the user clicks. The `category` prefix mirrors the routing in
//! `ActionExecutor` so the decoded [`crate::types::UnifiedAction`] lands in the
//! right handler group.
//!
//! Telegram predates this module and keeps its own private copy; Discord and the
//! later channels share this one.

use std::collections::HashMap;

use crate::types::{ActionButton, ActionCategory};

/// A decoded callback payload from an interactive button.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedCallback {
    pub category: ActionCategory,
    pub action: String,
    pub params: Option<HashMap<String, String>>,
}

/// Derive the category prefix from an action name, matching `ActionExecutor`
/// routing:
///   - `system.confirm` → `"chat"` (handled by `handle_chat_action`)
///   - `pairing.*` → `"platform"`
///   - `chat.*` / `action.*` → `"chat"`
///   - everything else (`session.*`, `help.*`, `agent.*`, `system.*`, ...) → `"system"`
pub fn action_category_prefix(action: &str) -> &'static str {
    if action == "system.confirm" {
        return "chat";
    }
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
    let action = parts[1].to_string();
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
        assert_eq!(action_category_prefix("system.confirm"), "chat");
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
            label: "Confirm".into(),
            action: "system.confirm".into(),
            params: Some(HashMap::from([("callId".into(), "abc".into())])),
        };
        let s = format_callback_data(&btn);
        assert!(s.starts_with("chat:system.confirm:"));
        assert!(s.contains("callId=abc"));
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
        let p = parse_callback_data("chat:system.confirm:callId=abc,value=yes").unwrap();
        assert_eq!(p.action, "system.confirm");
        let params = p.params.unwrap();
        assert_eq!(params.get("callId").unwrap(), "abc");
        assert_eq!(params.get("value").unwrap(), "yes");
    }

    #[test]
    fn parse_invalid() {
        assert!(parse_callback_data("nope").is_none());
        assert!(parse_callback_data("unknown:action").is_none());
    }

    #[test]
    fn roundtrip_confirm_routes_to_chat() {
        let btn = ActionButton {
            label: "Yes".into(),
            action: "system.confirm".into(),
            params: Some(HashMap::from([("value".into(), "yes".into())])),
        };
        let parsed = parse_callback_data(&format_callback_data(&btn)).unwrap();
        // system.confirm is routed to chat handler
        assert_eq!(parsed.category, ActionCategory::Chat);
        assert_eq!(parsed.action, "system.confirm");
    }
}
