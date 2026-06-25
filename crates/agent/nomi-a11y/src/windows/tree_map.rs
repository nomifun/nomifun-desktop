//! Pure transform layer for the Windows UIA backend.
//!
//! The actor (which owns COM) captures the cached UIA tree of every target
//! window into a `RawNode` forest of plain data — a synthetic `desktop` root
//! whose children are per-window subtrees, each node carrying an index into a
//! parallel handle table — and this module turns it into two products:
//!   * the numbered `ElementEntry` Set-of-Marks list (`build_entries`), grouped
//!     per window then in reading order, with a `ref → handle_idx` map so the
//!     actor can rebuild `ref → UIElement` for actuation; and
//!   * a hierarchical **semantic tree** text rendering (`render_tree`):
//!     `desktop → window → structural container → [ref] control`, pruning empty
//!     containers — the view the model reasons over.
//!
//! Keeping it free of any `uiautomation` / COM type makes the non-trivial logic
//! (interactability filter, off-screen handling, per-window reading-order
//! numbering, depth/budget truncation, the ref→handle correlation that must
//! survive the sort, and the structural-tree pruning) unit-testable without a
//! live UIA session; the COM reads in the actor are exercised by the `winsmoke`
//! example instead.
//!
//! It is intentionally Windows-local rather than routed through the neutral
//! `tree::flatten_interactable`: every emitted entry must map back to its live
//! `UIElement` handle for `invoke`, which the neutral `UiNode` cannot carry, the
//! off-screen-park filtering is UIA-specific, and the neutral layer has no
//! multi-window / semantic-tree concept. It reuses the neutral `normalize_role`
//! so role names stay consistent across platforms.

use std::collections::HashMap;

use crate::engine::{ElementEntry, Rect, Source};
use crate::tree::normalize_role;

/// Synthetic role for the forest root the actor builds above the per-window
/// subtrees. Excluded from emission and rendered as the literal `desktop` line.
pub(crate) const DESKTOP_ROLE: &str = "desktop";
/// Role the actor stamps on each top-level window node. Excluded from emission
/// (you target controls, not the frame) and rendered as `window "title"`.
pub(crate) const WINDOW_ROLE: &str = "window";

/// One UIA element captured as plain, COM-free data. `handle_idx` indexes the
/// actor's parallel `Vec<UIElement>` handle table, so a surviving entry can be
/// mapped back to its element for actuation. `children` preserve document order.
#[derive(Debug, Clone)]
pub(crate) struct RawNode {
    pub handle_idx: usize,
    pub role: String,
    pub name: Option<String>,
    pub value: Option<String>,
    pub states: Vec<String>,
    pub bounds: Rect,
    /// Inherently actionable or keyboard-focusable (the primary interactability
    /// signal). A named/valued node is also emitted even when this is false.
    pub actionable: bool,
    /// Has a ScrollPattern with a scrollable axis — emitted as a target so the
    /// model can scroll the region even when it is not otherwise actionable.
    pub scrollable: bool,
    /// Scrolled/clipped/parked off-screen: never emitted as a target, but its
    /// children are still traversed (a visible control inside an off-screen
    /// container is still reachable).
    pub offscreen: bool,
    pub children: Vec<RawNode>,
}

impl RawNode {
    /// A synthetic structural node (desktop root / window node) carrying no live
    /// handle. `handle_idx` is a sentinel that is never inserted into the handle
    /// table; such nodes are excluded from emission by role.
    pub(crate) fn structural(role: &str, name: Option<String>, bounds: Rect, children: Vec<RawNode>) -> Self {
        RawNode {
            handle_idx: usize::MAX,
            role: role.to_string(),
            name,
            value: None,
            states: Vec::new(),
            bounds,
            actionable: false,
            scrollable: false,
            offscreen: false,
            children,
        }
    }
}

/// Roles that are never themselves emitted as targets even when named: the
/// synthetic forest scaffolding. (Real window-like panes inside an app still
/// surface via their control type.)
fn is_scaffold_role(role: &str) -> bool {
    role == DESKTOP_ROLE || role == WINDOW_ROLE
}

/// True if this node should be emitted as a numbered target: on-screen, with
/// non-empty bounds, not forest scaffolding, and either actionable, scrollable,
/// or a named/valued leaf. A named *structural container* (toolbar/group/pane/…)
/// is a grouping branch in the semantic tree, not a click target, so it is not
/// emitted unless it is itself actionable or scrollable.
fn is_emittable(n: &RawNode) -> bool {
    if n.offscreen || n.bounds.is_empty() || is_scaffold_role(&n.role) {
        return false;
    }
    if n.actionable || n.scrollable {
        return true;
    }
    (n.name.is_some() || n.value.is_some()) && !is_structural_role(&n.role)
}

/// Container roles promoted to a labelled branch in the semantic tree when they
/// carry a name and have emittable descendants (otherwise pruned). Gives the
/// model the grouping context ("this button is inside the Formatting toolbar")
/// without numbering the container itself.
fn is_structural_role(role: &str) -> bool {
    matches!(
        role,
        "pane" | "group" | "toolbar" | "menubar" | "menu" | "tab" | "tree" | "list"
            | "table" | "datagrid" | "statusbar" | "titlebar" | "header" | "tabitem"
    )
}

/// The verb the model performs on a control of this role — surfaced in the
/// semantic tree as `[action: …]` so the affordance is explicit. Mirrors
/// Windows-MCP's action map (edit→fill, checkbox→toggle, …); scrollable regions
/// override to `scroll`.
pub(crate) fn action_for(role: &str, scrollable: bool) -> &'static str {
    if scrollable && role != "slider" {
        return "scroll";
    }
    match role {
        "edit" => "fill",
        "checkbox" => "toggle",
        "combobox" => "select",
        "radiobutton" => "select",
        "slider" => "slide",
        "document" => "scroll",
        _ => "click",
    }
}

/// Map a UIA `ToggleState` code (Off=0, On=1, Indeterminate=2) to a state label.
/// Off yields none (the unremarkable default), keeping the list terse.
pub(crate) fn toggle_label(code: i32) -> Option<&'static str> {
    match code {
        1 => Some("checked"),
        2 => Some("indeterminate"),
        _ => None,
    }
}

/// Map a UIA `ExpandCollapseState` code (Collapsed=0, Expanded=1,
/// PartiallyExpanded=2, LeafNode=3) to a state label. LeafNode (nothing to
/// expand) yields none.
pub(crate) fn expand_label(code: i32) -> Option<&'static str> {
    match code {
        0 => Some("collapsed"),
        1 => Some("expanded"),
        2 => Some("partially-expanded"),
        _ => None,
    }
}

/// Filter the `RawNode` forest to emittable targets (honoring `max_depth` +
/// `node_budget`), number them per-window then in reading order (top-to-bottom,
/// left-to-right), and return:
///   * the `ElementEntry` list (1-based `ref`s),
///   * the parallel handle indices in the SAME order (to rebuild ref→`UIElement`),
///   * a `handle_idx → ref` map (so the semantic renderer can annotate nodes),
///   * whether the forest was truncated by depth or budget.
///
/// `root` is either the synthetic `desktop` forest root (children = windows) or
/// a single window subtree (used directly).
pub(crate) fn build_entries(
    root: &RawNode,
    max_depth: usize,
    node_budget: usize,
) -> (Vec<ElementEntry>, Vec<usize>, HashMap<usize, u32>, bool) {
    let windows: Vec<&RawNode> = if root.role == DESKTOP_ROLE {
        root.children.iter().collect()
    } else {
        vec![root]
    };

    // Collect (window_index, node) so refs group by window (foreground first),
    // then within a window by reading order — matching the semantic tree layout.
    let mut collected: Vec<(usize, &RawNode)> = Vec::new();
    let mut truncated = false;
    for (wi, win) in windows.iter().enumerate() {
        collect(win, wi, 0, max_depth, node_budget, &mut collected, &mut truncated);
    }

    // Stable sort by (window, rounded y, rounded x): equal positions keep DFS
    // order. Window grouping dominates so refs never interleave across windows.
    collected.sort_by(|(awi, a), (bwi, b)| {
        (
            *awi,
            a.bounds.y.round() as i64,
            a.bounds.x.round() as i64,
        )
            .cmp(&(*bwi, b.bounds.y.round() as i64, b.bounds.x.round() as i64))
    });

    let mut entries = Vec::with_capacity(collected.len());
    let mut handle_indices = Vec::with_capacity(collected.len());
    let mut ref_by_handle = HashMap::with_capacity(collected.len());
    for (i, (_wi, node)) in collected.iter().enumerate() {
        let r = i as u32 + 1; // 1-based: matches the [ref] the model sees
        entries.push(ElementEntry {
            r#ref: r,
            role: normalize_role(&node.role),
            name: node.name.clone().filter(|s| !s.trim().is_empty()),
            value: node.value.clone().filter(|s| !s.trim().is_empty()),
            states: node.states.clone(),
            bounds: node.bounds,
            source: Source::A11y,
        });
        handle_indices.push(node.handle_idx);
        ref_by_handle.insert(node.handle_idx, r);
    }
    (entries, handle_indices, ref_by_handle, truncated)
}

/// Depth-first collect of emittable nodes within one window subtree. An
/// off-screen / non-emittable node is not pushed, but its children are still
/// traversed (until the depth cap), so a visible control inside an off-screen
/// container is reached. `depth` is measured from the window root (0).
fn collect<'a>(
    node: &'a RawNode,
    win_idx: usize,
    depth: usize,
    max_depth: usize,
    budget: usize,
    out: &mut Vec<(usize, &'a RawNode)>,
    truncated: &mut bool,
) {
    if is_emittable(node) {
        if out.len() >= budget {
            *truncated = true;
            return;
        }
        out.push((win_idx, node));
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
        collect(child, win_idx, depth + 1, max_depth, budget, out, truncated);
    }
}

// ---- semantic tree rendering --------------------------------------------
//
// Two phases (mirrors Windows-MCP's SemanticNode build + prune + render):
//   1. `build_sem` collapses the raw forest to only meaningful nodes — desktop,
//      windows, named structural containers, and emittable controls — making
//      transparent wrapper panes disappear so descendants attach to the nearest
//      meaningful ancestor.
//   2. `render_sem` draws it with ├──/└── connectors.

#[derive(Debug)]
enum SemKind {
    Desktop,
    Window,
    Structural,
    /// An emittable control, carrying its `[ref]`.
    Control(u32),
}

#[derive(Debug)]
struct SemNode {
    kind: SemKind,
    role: String,
    name: String,
    /// Center coordinates (emittable controls only).
    coords: Option<(i64, i64)>,
    scrollable: bool,
    states: Vec<String>,
    children: Vec<SemNode>,
}

/// Build the meaningful-node tree for `node`, appending the resulting node(s)
/// to `parent_children`. Transparent nodes (unnamed containers, plain wrappers)
/// contribute their children directly to the parent. Returns nothing; mutates
/// `parent_children`.
fn build_sem(node: &RawNode, ref_by_handle: &HashMap<usize, u32>, parent_children: &mut Vec<SemNode>) {
    let role = normalize_role(&node.role);

    // Desktop / window scaffolding: always a branch.
    if node.role == DESKTOP_ROLE {
        let mut me = SemNode {
            kind: SemKind::Desktop,
            role,
            name: node.name.clone().unwrap_or_default(),
            coords: None,
            scrollable: false,
            states: Vec::new(),
            children: Vec::new(),
        };
        for c in &node.children {
            build_sem(c, ref_by_handle, &mut me.children);
        }
        parent_children.push(me);
        return;
    }
    if node.role == WINDOW_ROLE {
        let mut me = SemNode {
            kind: SemKind::Window,
            role,
            name: node.name.clone().unwrap_or_default(),
            coords: None,
            scrollable: false,
            states: Vec::new(),
            children: Vec::new(),
        };
        for c in &node.children {
            build_sem(c, ref_by_handle, &mut me.children);
        }
        parent_children.push(me);
        return;
    }

    // An emittable control: a numbered leaf-or-branch.
    if let Some(&r) = ref_by_handle.get(&node.handle_idx) {
        let (cx, cy) = node.bounds.center();
        let mut me = SemNode {
            kind: SemKind::Control(r),
            role,
            name: node.name.clone().unwrap_or_default(),
            coords: Some((cx.round() as i64, cy.round() as i64)),
            scrollable: node.scrollable,
            states: node.states.clone(),
            children: Vec::new(),
        };
        for c in &node.children {
            build_sem(c, ref_by_handle, &mut me.children);
        }
        parent_children.push(me);
        return;
    }

    // A named structural container: tentatively a branch, kept only if it ends
    // up with children (pruned below otherwise).
    let named = node.name.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false);
    if named && is_structural_role(&role) && !node.offscreen {
        let mut me = SemNode {
            kind: SemKind::Structural,
            role,
            name: node.name.clone().unwrap_or_default(),
            coords: None,
            scrollable: false,
            states: Vec::new(),
            children: Vec::new(),
        };
        for c in &node.children {
            build_sem(c, ref_by_handle, &mut me.children);
        }
        if !me.children.is_empty() {
            parent_children.push(me);
        }
        return;
    }

    // Transparent: attach children to the current parent.
    for c in &node.children {
        build_sem(c, ref_by_handle, parent_children);
    }
}

fn format_sem_line(node: &SemNode) -> String {
    match &node.kind {
        SemKind::Desktop => "desktop".to_string(),
        SemKind::Window => format!("window {:?}", node.name),
        SemKind::Structural => {
            if node.name.is_empty() {
                node.role.clone()
            } else {
                format!("{} {:?}", node.role, node.name)
            }
        }
        SemKind::Control(r) => {
            let mut s = format!("[{r}] {}", node.role);
            if !node.name.is_empty() {
                s.push_str(&format!(" {:?}", truncate(&node.name, 80)));
            }
            if let Some((x, y)) = node.coords {
                s.push_str(&format!(" ({x},{y})"));
            }
            s.push_str(&format!("  [action: {}]", action_for(&node.role, node.scrollable)));
            if !node.states.is_empty() {
                s.push_str(&format!("  [{}]", node.states.join(",")));
            }
            s
        }
    }
}

fn render_sem(node: &SemNode, lines: &mut Vec<String>, prefix: &str, is_last: bool, is_root: bool) {
    if is_root {
        lines.push(format_sem_line(node));
    } else {
        let connector = if is_last { "└── " } else { "├── " };
        lines.push(format!("{prefix}{connector}{}", format_sem_line(node)));
    }
    let extension = if is_root {
        ""
    } else if is_last {
        "    "
    } else {
        "│   "
    };
    let child_prefix = format!("{prefix}{extension}");
    let n = node.children.len();
    for (i, child) in node.children.iter().enumerate() {
        render_sem(child, lines, &child_prefix, i == n - 1, false);
    }
}

/// Render the raw forest as the hierarchical semantic tree the model reads.
/// `root` is the synthetic `desktop` root (or a single window subtree).
pub(crate) fn render_tree(root: &RawNode, ref_by_handle: &HashMap<usize, u32>) -> String {
    let mut tops: Vec<SemNode> = Vec::new();
    build_sem(root, ref_by_handle, &mut tops);
    let mut lines: Vec<String> = Vec::new();
    let n = tops.len();
    for (i, top) in tops.iter().enumerate() {
        render_sem(top, &mut lines, "", i == n - 1, true);
    }
    lines.join("\n")
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

    fn n(handle_idx: usize, role: &str, name: Option<&str>, x: f64, y: f64, actionable: bool) -> RawNode {
        RawNode {
            handle_idx,
            role: role.to_string(),
            name: name.map(|s| s.to_string()),
            value: None,
            states: vec![],
            bounds: Rect { x, y, w: 40.0, h: 16.0 },
            actionable,
            scrollable: false,
            offscreen: false,
            children: vec![],
        }
    }

    fn window(children: Vec<RawNode>) -> RawNode {
        RawNode {
            handle_idx: 0,
            role: "window".to_string(),
            name: Some("Test Window".to_string()),
            value: None,
            states: vec![],
            bounds: Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 },
            actionable: false,
            scrollable: false,
            offscreen: false,
            children,
        }
    }

    #[test]
    fn emits_actionable_or_named_and_skips_plain_containers() {
        let root = window(vec![
            n(1, "button", None, 10.0, 50.0, true),    // actionable, no name → emit
            n(2, "group", None, 10.0, 80.0, false),    // not actionable, no name → skip
            n(3, "text", Some("Hello"), 10.0, 110.0, false), // named label → emit
        ]);
        let (entries, handles, ref_by_handle, trunc) = build_entries(&root, 12, 120);
        assert!(!trunc);
        // window root (role "window") skipped; button + text emitted.
        assert_eq!(entries.len(), 2, "entries: {entries:?}");
        assert_eq!(handles.len(), 2);
        assert_eq!(ref_by_handle.len(), 2);
        let roles: Vec<_> = entries.iter().map(|e| e.role.as_str()).collect();
        assert!(roles.contains(&"button"));
        assert!(roles.contains(&"text"));
    }

    #[test]
    fn skips_empty_bounds() {
        let mut ghost = n(1, "button", Some("Ghost"), 10.0, 10.0, true);
        ghost.bounds = Rect { x: 10.0, y: 10.0, w: 0.0, h: 0.0 };
        let (entries, _, _, _) = build_entries(&window(vec![ghost]), 12, 120);
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn offscreen_node_not_emitted_but_children_traversed() {
        let mut container = n(1, "pane", Some("Panel"), 5.0, 5.0, false);
        container.offscreen = true;
        container.bounds = Rect { x: 5.0, y: 5.0, w: 300.0, h: 300.0 };
        container.children = vec![n(2, "button", Some("Deep"), 20.0, 20.0, true)];
        let (entries, handles, _, _) = build_entries(&window(vec![container]), 12, 120);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name.as_deref(), Some("Deep"));
        assert_eq!(handles, vec![2]);
    }

    #[test]
    fn numbers_in_reading_order_and_correlates_handles_across_sort() {
        let root = window(vec![
            n(7, "button", Some("Bottom"), 10.0, 200.0, true),
            n(9, "button", Some("Top"), 10.0, 10.0, true),
        ]);
        let (entries, handles, ref_by_handle, _) = build_entries(&root, 12, 120);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name.as_deref(), Some("Top"));
        assert_eq!(entries[0].r#ref, 1);
        assert_eq!(handles[0], 9);
        assert_eq!(ref_by_handle[&9], 1);
        assert_eq!(entries[1].name.as_deref(), Some("Bottom"));
        assert_eq!(entries[1].r#ref, 2);
        assert_eq!(handles[1], 7);
        assert_eq!(ref_by_handle[&7], 2);
    }

    #[test]
    fn budget_truncates() {
        let kids: Vec<RawNode> = (1..=10).map(|i| n(i, "button", Some("b"), 0.0, i as f64, true)).collect();
        let (entries, handles, _, trunc) = build_entries(&window(kids), 12, 3);
        assert!(trunc);
        assert!(entries.len() <= 3);
        assert_eq!(entries.len(), handles.len());
    }

    #[test]
    fn depth_cap_truncates_and_drops_deep_nodes() {
        let gc = n(2, "button", Some("Deep"), 10.0, 10.0, true);
        let mut child = n(1, "pane", None, 5.0, 5.0, false);
        child.bounds = Rect { x: 5.0, y: 5.0, w: 100.0, h: 100.0 };
        child.children = vec![gc];
        let (entries, _, _, trunc) = build_entries(&window(vec![child]), 1, 120);
        assert!(trunc);
        assert!(entries.iter().all(|e| e.name.as_deref() != Some("Deep")));
    }

    #[test]
    fn states_and_value_pass_through() {
        let mut cb = n(1, "checkbox", Some("Agree"), 10.0, 10.0, true);
        cb.states = vec!["checked".into(), "focused".into()];
        cb.value = Some("on".into());
        let (entries, _, _, _) = build_entries(&window(vec![cb]), 12, 120);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].states, vec!["checked".to_string(), "focused".to_string()]);
        assert_eq!(entries[0].value.as_deref(), Some("on"));
        assert_eq!(entries[0].role, "checkbox");
    }

    #[test]
    fn toggle_and_expand_labels_map_uia_codes() {
        assert_eq!(toggle_label(0), None);
        assert_eq!(toggle_label(1), Some("checked"));
        assert_eq!(toggle_label(2), Some("indeterminate"));
        assert_eq!(toggle_label(99), None);
        assert_eq!(expand_label(0), Some("collapsed"));
        assert_eq!(expand_label(1), Some("expanded"));
        assert_eq!(expand_label(2), Some("partially-expanded"));
        assert_eq!(expand_label(3), None);
    }

    // ---- new behavior: scrollables, action map, multi-window, rendering ----

    #[test]
    fn scrollable_container_is_emitted_even_without_name_or_action() {
        let mut scroll = n(1, "pane", None, 0.0, 0.0, false);
        scroll.scrollable = true;
        scroll.bounds = Rect { x: 0.0, y: 0.0, w: 300.0, h: 300.0 };
        let (entries, _, _, _) = build_entries(&window(vec![scroll]), 12, 120);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].role, "pane");
    }

    #[test]
    fn action_map_matches_role_and_scrollable() {
        assert_eq!(action_for("edit", false), "fill");
        assert_eq!(action_for("checkbox", false), "toggle");
        assert_eq!(action_for("combobox", false), "select");
        assert_eq!(action_for("radiobutton", false), "select");
        assert_eq!(action_for("slider", false), "slide");
        assert_eq!(action_for("document", false), "scroll");
        assert_eq!(action_for("button", false), "click");
        assert_eq!(action_for("pane", true), "scroll"); // scrollable overrides
        assert_eq!(action_for("slider", true), "slide"); // slider keeps slide
    }

    #[test]
    fn build_entries_groups_refs_by_window_then_reading_order() {
        // Window B sits visually ABOVE window A (smaller y), but refs must group
        // by window order (A first in the forest), not by global y.
        let win_a = RawNode::structural(
            WINDOW_ROLE,
            Some("App A".into()),
            Rect { x: 0.0, y: 100.0, w: 400.0, h: 400.0 },
            vec![n(10, "button", Some("A1"), 10.0, 300.0, true)],
        );
        let win_b = RawNode::structural(
            WINDOW_ROLE,
            Some("App B".into()),
            Rect { x: 500.0, y: 0.0, w: 400.0, h: 400.0 },
            vec![n(20, "button", Some("B1"), 510.0, 10.0, true)],
        );
        let desktop = RawNode::structural(
            DESKTOP_ROLE,
            None,
            Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 },
            vec![win_a, win_b],
        );
        let (entries, _, _, _) = build_entries(&desktop, 12, 120);
        assert_eq!(entries.len(), 2);
        // A1 (window A, listed first) gets [1] even though B1 is higher on screen.
        assert_eq!(entries[0].name.as_deref(), Some("A1"));
        assert_eq!(entries[0].r#ref, 1);
        assert_eq!(entries[1].name.as_deref(), Some("B1"));
        assert_eq!(entries[1].r#ref, 2);
    }

    #[test]
    fn render_tree_shows_desktop_windows_and_refs() {
        let win = RawNode::structural(
            WINDOW_ROLE,
            Some("Notepad".into()),
            Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 },
            vec![n(5, "button", Some("Save"), 100.0, 50.0, true)],
        );
        let desktop = RawNode::structural(
            DESKTOP_ROLE,
            None,
            Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 },
            vec![win],
        );
        let (_, _, ref_by_handle, _) = build_entries(&desktop, 12, 120);
        let tree = render_tree(&desktop, &ref_by_handle);
        assert!(tree.starts_with("desktop"), "tree:\n{tree}");
        assert!(tree.contains(r#"window "Notepad""#), "tree:\n{tree}");
        assert!(tree.contains(r#"[1] button "Save""#), "tree:\n{tree}");
        assert!(tree.contains("[action: click]"), "tree:\n{tree}");
        assert!(tree.contains("(120,58)"), "center of Save; tree:\n{tree}");
    }

    #[test]
    fn render_tree_promotes_named_container_and_prunes_empty_one() {
        // A named toolbar with an actionable child → promoted to a branch with
        // the button numbered under it. An empty named group (no emittable
        // descendant) → pruned.
        let named_toolbar = RawNode {
            handle_idx: 100,
            role: "toolbar".into(),
            name: Some("Formatting".into()),
            value: None,
            states: vec![],
            bounds: Rect { x: 0.0, y: 0.0, w: 800.0, h: 40.0 },
            actionable: false,
            scrollable: false,
            offscreen: false,
            children: vec![n(1, "button", Some("Bold"), 10.0, 10.0, true)],
        };
        let empty_group = RawNode {
            handle_idx: 101,
            role: "group".into(),
            name: Some("Empty".into()),
            value: None,
            states: vec![],
            bounds: Rect { x: 0.0, y: 100.0, w: 800.0, h: 40.0 },
            actionable: false,
            scrollable: false,
            offscreen: false,
            children: vec![n(2, "group", None, 0.0, 0.0, false)], // non-emittable
        };
        let win = RawNode::structural(
            WINDOW_ROLE,
            Some("App".into()),
            Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 },
            vec![named_toolbar, empty_group],
        );
        let desktop = RawNode::structural(DESKTOP_ROLE, None, win.bounds, vec![win]);
        let (entries, _, ref_by_handle, _) = build_entries(&desktop, 12, 120);
        // Only the Bold button is emittable (toolbar/group are structural).
        assert_eq!(entries.len(), 1, "entries: {entries:?}");
        let tree = render_tree(&desktop, &ref_by_handle);
        assert!(tree.contains(r#"toolbar "Formatting""#), "tree:\n{tree}");
        assert!(tree.contains(r#"[1] button "Bold""#), "tree:\n{tree}");
        assert!(!tree.contains("Empty"), "empty container must be pruned; tree:\n{tree}");
    }

    #[test]
    fn render_tree_collapses_transparent_wrappers() {
        // unnamed pane wrapper → its child attaches to the window directly.
        let mut wrapper = n(1, "pane", None, 0.0, 0.0, false);
        wrapper.bounds = Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 };
        wrapper.children = vec![n(2, "button", Some("Go"), 10.0, 10.0, true)];
        let win = RawNode::structural(WINDOW_ROLE, Some("W".into()), wrapper.bounds, vec![wrapper]);
        let desktop = RawNode::structural(DESKTOP_ROLE, None, win.bounds, vec![win]);
        let (_, _, ref_by_handle, _) = build_entries(&desktop, 12, 120);
        let tree = render_tree(&desktop, &ref_by_handle);
        // No "pane" line (unnamed wrapper collapsed); button present under window.
        assert!(!tree.contains("pane"), "transparent wrapper must collapse; tree:\n{tree}");
        assert!(tree.contains(r#"[1] button "Go""#), "tree:\n{tree}");
    }
}
