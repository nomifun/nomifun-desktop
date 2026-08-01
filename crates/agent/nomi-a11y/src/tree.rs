//! Role normalization and text formatting for accessibility-tree element
//! lists, shared by every OS backend. Backends build `ElementEntry` lists
//! directly from their native API (entries must map back to live platform
//! handles); this module renders them for the model.

use crate::engine::ElementEntry;

/// Strip the platform `AX`/`UIA_` prefix and lowercase so the model sees
/// stable cross-platform role names (`button`, `textfield`, …).
pub fn normalize_role(role: &str) -> String {
    let r = role
        .strip_prefix("AX")
        .or_else(|| role.strip_prefix("UIA_"))
        .unwrap_or(role);
    r.to_lowercase()
}

/// Render entries as a numbered text list for the model:
/// `[14] button "Submit" enabled`.
pub fn format_entries(entries: &[ElementEntry]) -> String {
    if entries.is_empty() {
        return "No interactable elements found in the accessibility tree.".to_string();
    }
    let mut out = String::new();
    for e in entries {
        out.push_str(&format!("[{}] {}", e.r#ref, e.role));
        if let Some(name) = &e.name {
            out.push_str(&format!(" {:?}", truncate(name, 80)));
        }
        if let Some(value) = &e.value {
            if Some(value) != e.name.as_ref() {
                out.push_str(&format!(" = {:?}", truncate(value, 60)));
            }
        }
        if !e.states.is_empty() {
            out.push_str(&format!(" [{}]", e.states.join(",")));
        }
        out.push('\n');
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max).collect();
        format!("{t}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Rect, Source};

    #[test]
    fn normalize_strips_platform_prefixes() {
        assert_eq!(normalize_role("AXButton"), "button");
        assert_eq!(normalize_role("UIA_EditControlTypeId"), "editcontroltypeid");
        assert_eq!(normalize_role("push button"), "push button");
    }

    #[test]
    fn format_is_readable() {
        let entries = vec![ElementEntry {
            r#ref: 1,
            role: normalize_role("AXButton"),
            name: Some("Save".to_string()),
            value: None,
            states: vec![],
            bounds: Rect { x: 0.0, y: 0.0, w: 50.0, h: 20.0 },
            source: Source::A11y,
        }];
        let text = format_entries(&entries);
        assert!(text.contains("[1] button"));
        assert!(text.contains("Save"));
    }

    #[test]
    fn format_empty_reports_no_elements() {
        assert!(format_entries(&[]).contains("No interactable elements"));
    }
}
