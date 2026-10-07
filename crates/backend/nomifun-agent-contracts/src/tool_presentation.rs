//! Shared presentation vocabulary. These names never confer authority: callers
//! retain the exact capability/action or frozen contribution identity separately.
use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize)]
pub struct ToolPresentation {
    pub capability_id: Option<String>,
    pub action_id: Option<String>,
    pub name: String,
    pub titles: BTreeMap<String, String>,
    pub target_fields: Vec<String>,
    pub target_kind: Option<String>,
    pub receipt_action: Option<String>,
}

pub fn tool_presentations() -> &'static [ToolPresentation] {
    static CATALOGUE: OnceLock<Vec<ToolPresentation>> = OnceLock::new();
    CATALOGUE.get_or_init(|| {
        serde_json::from_str(include_str!("../contracts/tool-presentation.json"))
            .expect("checked tool presentation catalogue")
    })
}

pub fn tool_presentation(capability_id: &str, action_id: &str) -> Option<&'static ToolPresentation> {
    tool_presentations().iter().find(|entry| {
        entry.capability_id.as_deref() == Some(capability_id)
            && entry.action_id.as_deref() == Some(action_id)
    })
}

pub fn platform_tool_name(capability_id: &str, action_id: &str) -> String {
    if let Some(presentation) = tool_presentation(capability_id, action_id) {
        return presentation.name.clone();
    }
    let action = relative_action(capability_id, action_id);
    namespaced_tool_name(
        "platform", capability_id, action,
        format!("{capability_id}\0{action_id}").as_bytes(),
    )
}

pub fn relative_action<'a>(capability_id: &str, action_id: &'a str) -> &'a str {
    action_id.strip_prefix(capability_id)
        .and_then(|suffix| suffix.strip_prefix('/'))
        .unwrap_or(action_id)
}

/// Provider-safe, deterministic alias. Preserve both origin and action even
/// when their names are long; the hash binds the full, untruncated identity.
pub fn namespaced_tool_name(namespace: &str, origin: &str, action: &str, identity: &[u8]) -> String {
    let namespace = slug(namespace);
    assert!(namespace.len() <= 12, "tool namespace exceeds name budget");
    let mut origin = slug(origin);
    let mut action = slug(action);
    let available = 64 - namespace.len() - 6 - 20;
    let origin_budget = origin.len().min(16).min(available - 8);
    let action_budget = available - origin_budget;
    origin.truncate(origin_budget);
    action.truncate(action_budget);
    let hash = hex::encode(Sha256::digest(identity));
    format!(
        "{namespace}__{}__{}__{}",
        origin.trim_end_matches('_'), action.trim_end_matches('_'), &hash[..20],
    )
}

fn slug(value: &str) -> String {
    let mut result = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() {
            result.push(char::from(byte.to_ascii_lowercase()));
        } else if !result.is_empty() && !result.ends_with('_') {
            result.push('_');
        }
    }
    let result = result.trim_end_matches('_');
    if result.is_empty() {
        "tool".into()
    } else {
        result.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn catalogue_has_unique_names_exact_actions_and_complete_translations() {
        let mut names = BTreeSet::new();
        let mut actions = BTreeSet::new();
        for entry in tool_presentations() {
            assert!(names.insert(&entry.name), "duplicate name {}", entry.name);
            assert!(entry.name.len() <= 64);
            assert!(entry.name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
            assert!(!entry.titles["zh-CN"].is_empty());
            assert!(!entry.titles["en-US"].is_empty());
            if let Some(action) = &entry.action_id {
                assert!(actions.insert((entry.capability_id.as_ref().unwrap(), action)));
            }
        }
        assert_eq!(platform_tool_name("web.research", "web.research/search"), "web_search");
        assert_eq!(platform_tool_name("web.research", "web.research/fetch"), "web_fetch");
        assert!(platform_tool_name("third.party", "web.research/search").starts_with("platform__"));
    }

    #[test]
    fn external_aliases_preserve_actions_and_disambiguate_sanitized_origins() {
        let name = namespaced_tool_name("mcp", &"server".repeat(40), "search_documents", b"exact-a");
        assert!(name.len() <= 64);
        assert!(name.contains("__search_documents__"));
        assert_eq!(name, namespaced_tool_name("mcp", &"server".repeat(40), "search_documents", b"exact-a"));
        assert_ne!(namespaced_tool_name("plugin", "a-b", "run", b"a-b/run"),
            namespaced_tool_name("plugin", "a_b", "run", b"a_b/run"));
        let unicode = namespaced_tool_name("plugin", "知识库", "搜索", b"unicode");
        assert!(unicode.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
        assert!(!unicode.contains("____"));
    }
}
