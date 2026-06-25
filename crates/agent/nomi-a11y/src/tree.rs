//! Neutral accessibility-tree model + interactable filtering and text
//! formatting, shared by every OS backend. Backends build a `UiNode` tree from
//! their native API; this module turns it into the numbered `ElementEntry`
//! list the model consumes (and the overlay draws).

use crate::engine::{ElementEntry, Rect, Source};

/// A raw accessibility node as captured by a backend, before filtering.
#[derive(Debug, Clone)]
pub struct UiNode {
    pub role: String,
    pub name: Option<String>,
    pub value: Option<String>,
    pub states: Vec<String>,
    pub bounds: Option<Rect>,
    /// Backend's verdict that this node is actionable (has a default action /
    /// is a control role) — the primary interactability signal.
    pub actionable: bool,
    pub children: Vec<UiNode>,
}

impl UiNode {
    pub fn leaf(role: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            name: None,
            value: None,
            states: vec![],
            bounds: None,
            actionable: false,
            children: vec![],
        }
    }
}

fn is_interactable(n: &UiNode) -> bool {
    let Some(b) = n.bounds else { return false };
    if b.is_empty() {
        return false;
    }
    // Actionable per the backend, or a control that carries a label/value worth
    // targeting even if no explicit action was reported.
    n.actionable || n.name.is_some() || n.value.is_some()
}

/// Depth-first collect interactable nodes (honoring depth + budget), then number
/// them in reading order (top-to-bottom, left-to-right). Returns
/// `(entries, truncated)`.
pub fn flatten_interactable(
    root: &UiNode,
    max_depth: usize,
    node_budget: usize,
) -> (Vec<ElementEntry>, bool) {
    let mut collected: Vec<&UiNode> = Vec::new();
    let mut truncated = false;
    collect(root, 0, max_depth, node_budget, &mut collected, &mut truncated);

    // Reading order: sort by rounded (y, x) so the model's numbering tracks the
    // visual layout. Stable so equal positions keep DFS order.
    collected.sort_by(|a, b| {
        let (ax, ay) = a.bounds.map(|r| (r.x, r.y)).unwrap_or((0.0, 0.0));
        let (bx, by) = b.bounds.map(|r| (r.x, r.y)).unwrap_or((0.0, 0.0));
        (ay.round() as i64, ax.round() as i64).cmp(&(by.round() as i64, bx.round() as i64))
    });

    let entries = collected
        .into_iter()
        .enumerate()
        .map(|(i, n)| ElementEntry {
            r#ref: i as u32 + 1, // 1-based: matches the [ref] the model sees
            role: normalize_role(&n.role),
            name: n.name.clone().filter(|s| !s.trim().is_empty()),
            value: n.value.clone().filter(|s| !s.trim().is_empty()),
            states: n.states.clone(),
            bounds: n.bounds.unwrap_or(Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }),
            source: Source::A11y,
        })
        .collect();
    (entries, truncated)
}

fn collect<'a>(
    node: &'a UiNode,
    depth: usize,
    max_depth: usize,
    budget: usize,
    out: &mut Vec<&'a UiNode>,
    truncated: &mut bool,
) {
    if is_interactable(node) {
        if out.len() >= budget {
            *truncated = true;
            return;
        }
        out.push(node);
    }
    if depth >= max_depth {
        if !node.children.is_empty() {
            *truncated = true;
        }
        return;
    }
    for child in &node.children {
        if out.len() >= budget {
            *truncated = true;
            return;
        }
        collect(child, depth + 1, max_depth, budget, out, truncated);
    }
}

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

    fn node(role: &str, name: Option<&str>, x: f64, y: f64, actionable: bool, children: Vec<UiNode>) -> UiNode {
        UiNode {
            role: role.to_string(),
            name: name.map(|s| s.to_string()),
            value: None,
            states: vec![],
            bounds: Some(Rect { x, y, w: 50.0, h: 20.0 }),
            actionable,
            children,
        }
    }

    #[test]
    fn flattens_filters_and_numbers_in_reading_order() {
        // Root window (not actionable, no name) with two buttons out of order.
        let root = UiNode {
            role: "AXWindow".into(),
            name: None,
            value: None,
            states: vec![],
            bounds: Some(Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 }),
            actionable: false,
            children: vec![
                node("AXButton", Some("Bottom"), 10.0, 200.0, true, vec![]),
                node("AXButton", Some("Top"), 10.0, 10.0, true, vec![]),
            ],
        };
        let (entries, truncated) = flatten_interactable(&root, 12, 120);
        assert!(!truncated);
        assert_eq!(entries.len(), 2);
        // Sorted top-to-bottom: "Top" gets [1].
        assert_eq!(entries[0].name.as_deref(), Some("Top"));
        assert_eq!(entries[0].role, "button");
        assert_eq!(entries[0].r#ref, 1);
        assert_eq!(entries[1].name.as_deref(), Some("Bottom"));
    }

    #[test]
    fn budget_truncates() {
        let children: Vec<UiNode> = (0..10)
            .map(|i| node("AXButton", Some("b"), 0.0, i as f64, true, vec![]))
            .collect();
        let root = UiNode {
            role: "AXWindow".into(),
            name: None,
            value: None,
            states: vec![],
            bounds: Some(Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }),
            actionable: false,
            children,
        };
        let (entries, truncated) = flatten_interactable(&root, 12, 3);
        assert!(truncated);
        assert!(entries.len() <= 3);
    }

    #[test]
    fn format_is_readable() {
        let entries = flatten_interactable(
            &node("AXButton", Some("Save"), 0.0, 0.0, true, vec![]),
            12,
            120,
        )
        .0;
        let text = format_entries(&entries);
        assert!(text.contains("[1] button"));
        assert!(text.contains("Save"));
    }
}
