//! The Windows UIA actor: a single dedicated thread that initializes COM as MTA
//! (`CoInitializeEx(COINIT_MULTITHREADED)`), owns the `UIAutomation` instance
//! and every `UIElement` handle, and is the sole caller of UI Automation. The
//! public engine sends commands over a channel and blocks on a per-command
//! reply, so UIA handles (which have COM apartment affinity) never leave this
//! thread. Only serializable `Snapshot` / `Effect` data crosses the boundary,
//! so the handle is `Send + Sync` with no `unsafe impl`.
//!
//! We use the high-level `uiautomation` crate rather than hand-rolling COM: it
//! wraps `IUIAutomation*`, the control-view tree walker, control types, and the
//! Invoke/Value/Scroll/RangeValue patterns. COM init + the window-enumeration /
//! foreground / focus Win32 calls go through `windows` (windows-rs) directly.
//!
//! Coordinate system: UIA `BoundingRectangle` is already global, device-pixel,
//! top-left origin — exactly the space xcap/enigo use on Windows — so bounds are
//! returned verbatim with no flip or logical↔physical conversion (see the
//! backend guide §5).
//!
//! Observe model: `observe()` with `pid == None` captures the **desktop** — the
//! foreground window plus the other visible top-level application windows — as a
//! `desktop → window → …` forest, so the model can see and act across windows
//! (dialogs, pickers, a second app). `pid == Some` captures just that app's main
//! window. Each window's subtree is pulled in a single `find_first_build_cache`
//! IPC; per-window failures are logged and skipped rather than failing the whole
//! observe.
//!
//! Snapshot caching: this backend re-walks the tree on every `observe`. A
//! macOS-style event-driven cache was prototyped and removed: correct
//! invalidation needs StructureChanged + LayoutInvalidated + property-changed +
//! focus-changed + scroll handlers (and even then has races); since tool
//! observes are seconds apart any short-TTL cache never hits, and a partial
//! cache silently serves stale snapshots — strictly worse than a fast re-walk.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

use uiautomation::controls::ControlType;
use uiautomation::core::UICacheRequest;
use uiautomation::types::{Point, TreeScope, UIProperty};
use uiautomation::variants::Value;
use uiautomation::{UIAutomation, UIElement, UITreeWalker, patterns};

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GetClassNameW, GetForegroundWindow, GetWindow, GetWindowTextLengthW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_RESTORE, SetForegroundWindow,
    ShowWindow,
};
use windows::core::{BOOL, PWSTR};

use crate::engine::{
    A11yError, Effect, ElementAction, ObserveOpts, Rect, Snapshot, SnapshotGen, Target,
};

use super::tree_map::{self, DESKTOP_ROLE, RawNode, WINDOW_ROLE};

/// Hard cap on top-level windows captured in a desktop (pid==None) observe. The
/// foreground window is always first; the rest are other visible app windows in
/// z-order. Bounds the per-observe cost (one cached IPC + a tree walk each).
const MAX_WINDOWS: usize = 8;

// ---- role mapping: UIA ControlType → stable lowercase cross-platform name ----

/// Map a UIA `ControlType` to the stable lowercase role name the model sees,
/// aligned with the macOS backend's normalized roles (`button`, `edit`, …).
fn ct_to_role(ct: ControlType) -> &'static str {
    match ct {
        ControlType::Button => "button",
        ControlType::Calendar => "calendar",
        ControlType::CheckBox => "checkbox",
        ControlType::ComboBox => "combobox",
        ControlType::Edit => "edit",
        ControlType::Hyperlink => "hyperlink",
        ControlType::Image => "image",
        ControlType::ListItem => "listitem",
        ControlType::List => "list",
        ControlType::Menu => "menu",
        ControlType::MenuBar => "menubar",
        ControlType::MenuItem => "menuitem",
        ControlType::ProgressBar => "progressbar",
        ControlType::RadioButton => "radiobutton",
        ControlType::ScrollBar => "scrollbar",
        ControlType::Slider => "slider",
        ControlType::Spinner => "spinner",
        ControlType::StatusBar => "statusbar",
        ControlType::Tab => "tab",
        ControlType::TabItem => "tabitem",
        ControlType::Text => "text",
        ControlType::ToolBar => "toolbar",
        ControlType::ToolTip => "tooltip",
        ControlType::Tree => "tree",
        ControlType::TreeItem => "treeitem",
        ControlType::Custom => "custom",
        ControlType::Group => "group",
        ControlType::Thumb => "thumb",
        ControlType::DataGrid => "datagrid",
        ControlType::DataItem => "dataitem",
        ControlType::Document => "document",
        ControlType::SplitButton => "splitbutton",
        ControlType::Window => "window",
        ControlType::Pane => "pane",
        ControlType::Header => "header",
        ControlType::HeaderItem => "headeritem",
        ControlType::Table => "table",
        ControlType::TitleBar => "titlebar",
        ControlType::Separator => "separator",
        ControlType::SemanticZoom => "semanticzoom",
        ControlType::AppBar => "appbar",
    }
}

/// Whether a control is an interaction target, from its control type plus
/// keyboard-focusability. Pure so both the cached and live readers share it.
fn actionable_from(ct: ControlType, focusable: bool) -> bool {
    let by_type = matches!(
        ct,
        ControlType::Button
            | ControlType::CheckBox
            | ControlType::RadioButton
            | ControlType::ComboBox
            | ControlType::Edit
            | ControlType::Hyperlink
            | ControlType::ListItem
            | ControlType::MenuItem
            | ControlType::Tab
            | ControlType::TabItem
            | ControlType::TreeItem
            | ControlType::Slider
            | ControlType::SplitButton
            | ControlType::Spinner
            | ControlType::Document
    );
    by_type || focusable
}

/// Format a numeric value tersely: integers without a decimal tail, else 2 dp.
fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v:.2}")
    }
}

/// Read display text through the Text pattern, bounded in provider UTF-16 units.
fn text_via_text_pattern(elem: &UIElement) -> Option<String> {
    const MAX: i32 = 2000;
    let pat = elem.get_pattern::<patterns::UITextPattern>().ok()?;
    let range = pat.get_document_range().ok()?;
    let text = range.get_text(MAX).ok()?;
    let t = text.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

/// Password protection is checked before either content getter. Unknown
/// protection also avoids content reads; no password value is projected.
fn read_value(elem: &UIElement, ct: ControlType) -> Option<String> {
    if !matches!(ct, ControlType::Edit | ControlType::Document | ControlType::ComboBox)
        || elem.is_password().unwrap_or(true)
    {
        return None;
    }
    elem.get_pattern::<patterns::UIValuePattern>()
        .ok()
        .and_then(|p| p.get_value().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| text_via_text_pattern(elem))
}

// ---- capturing a window into a RawNode tree -----------------------------

/// Properties fetched in bulk by the cache request — exactly those every node
/// needs, so the per-node read storm collapses into one cross-process call. The
/// `Is*PatternAvailable` flags gate the matching state property so a default
/// value on a non-supporting element is never misread as a real state.
const CACHED_PROPS: [UIProperty; 27] = [
    UIProperty::ControlType,
    UIProperty::Name,
    UIProperty::BoundingRectangle,
    UIProperty::IsEnabled,
    UIProperty::IsKeyboardFocusable,
    UIProperty::HasKeyboardFocus,
    UIProperty::IsOffscreen,
    UIProperty::HelpText,
    UIProperty::IsPassword,
    UIProperty::AcceleratorKey,
    UIProperty::IsTogglePatternAvailable,
    UIProperty::ToggleToggleState,
    UIProperty::IsSelectionItemPatternAvailable,
    UIProperty::SelectionItemIsSelected,
    UIProperty::IsExpandCollapsePatternAvailable,
    UIProperty::ExpandCollapseExpandCollapseState,
    UIProperty::IsValuePatternAvailable,
    UIProperty::ValueIsReadOnly,
    UIProperty::IsRangeValuePatternAvailable,
    UIProperty::RangeValueValue,
    UIProperty::RangeValueMinimum,
    UIProperty::RangeValueMaximum,
    UIProperty::IsScrollPatternAvailable,
    UIProperty::ScrollVerticallyScrollable,
    UIProperty::ScrollHorizontallyScrollable,
    UIProperty::ScrollVerticalScrollPercent,
    UIProperty::ScrollHorizontalScrollPercent,
];

/// Build the subtree-scoped cache request used by `build_raw_cached`.
fn build_cache_request(auto: &UIAutomation) -> Result<UICacheRequest, A11yError> {
    let cache = auto
        .create_cache_request()
        .map_err(|e| uia_err("create_cache_request", e))?;
    for p in CACHED_PROPS {
        cache
            .add_property(p)
            .map_err(|e| uia_err("cache add_property", e))?;
    }
    cache
        .set_tree_scope(TreeScope::Subtree)
        .map_err(|e| uia_err("cache set_tree_scope", e))?;
    Ok(cache)
}

/// Convert a uiautomation `Rect` (corners) to our `Rect`, avoiding the +1px that
/// `get_width()`/`get_height()` add (which would let a 0-size element slip past
/// `is_empty()`).
fn rect_of(r: &uiautomation::types::Rect) -> Rect {
    let (left, top) = (r.get_left(), r.get_top());
    Rect {
        x: left as f64,
        y: top as f64,
        w: r.get_right() as f64 - left as f64,
        h: r.get_bottom() as f64 - top as f64,
    }
}

/// Minimized/parked windows report elements at the off-screen park (~-32000) —
/// unclickable phantoms. Treated as off-screen: traversed (children may be real)
/// but never emitted.
fn is_parked(b: &Rect) -> bool {
    b.x <= -30000.0 || b.y <= -30000.0
}

fn cached_bool(el: &UIElement, p: UIProperty) -> Option<bool> {
    el.get_cached_property_value(p)
        .ok()
        .and_then(|v| (&v).try_into().ok())
}

fn cached_i32(el: &UIElement, p: UIProperty) -> Option<i32> {
    el.get_cached_property_value(p)
        .ok()
        .and_then(|v| (&v).try_into().ok())
}

fn cached_f64(el: &UIElement, p: UIProperty) -> Option<f64> {
    el.get_cached_property_value(p)
        .ok()
        .and_then(|v| (&v).try_into().ok())
}

fn cached_string(el: &UIElement, p: UIProperty) -> Option<String> {
    let v = el.get_cached_property_value(p).ok()?;
    let s: String = (&v).try_into().ok()?;
    let t = s.trim().to_string();
    if t.is_empty() { None } else { Some(t) }
}

/// Read a control-pattern state code, but only when the pattern is actually
/// available (per its cached `Is*PatternAvailable` flag); otherwise `None`.
fn cached_state_code(el: &UIElement, available: UIProperty, state: UIProperty) -> Option<i32> {
    if cached_bool(el, available).unwrap_or(false) {
        cached_i32(el, state)
    } else {
        None
    }
}

/// Append semantic states derived from cached control-pattern properties. Each
/// pattern read is gated by the matching `Is*PatternAvailable` flag.
fn push_pattern_states(el: &UIElement, ct: ControlType, states: &mut Vec<String>) {
    if let Some(l) =
        cached_state_code(el, UIProperty::IsTogglePatternAvailable, UIProperty::ToggleToggleState)
            .and_then(tree_map::toggle_label)
    {
        states.push(l.to_string());
    }
    if cached_bool(el, UIProperty::IsSelectionItemPatternAvailable).unwrap_or(false)
        && cached_bool(el, UIProperty::SelectionItemIsSelected).unwrap_or(false)
    {
        states.push("selected".to_string());
    }
    if let Some(l) = cached_state_code(
        el,
        UIProperty::IsExpandCollapsePatternAvailable,
        UIProperty::ExpandCollapseExpandCollapseState,
    )
    .and_then(tree_map::expand_label)
    {
        states.push(l.to_string());
    }
    let edit_like = matches!(
        ct,
        ControlType::Edit | ControlType::Document | ControlType::ComboBox
    );
    if edit_like
        && cached_bool(el, UIProperty::IsValuePatternAvailable).unwrap_or(false)
        && cached_bool(el, UIProperty::ValueIsReadOnly).unwrap_or(false)
    {
        states.push("readonly".to_string());
    }
    if el.is_cached_password().unwrap_or(false) {
        states.push("password".to_string());
    }
    // Accelerator (shortcut) key, e.g. "Ctrl+S" — useful actuation hint.
    if let Some(k) = cached_string(el, UIProperty::AcceleratorKey) {
        states.push(format!("shortcut:{k}"));
    }
}

/// Read scroll state from cached Scroll-pattern properties: returns whether the
/// element is scrollable on any axis, pushing `v:NN%`/`h:NN%` states for each
/// scrollable axis (percent of -1 = "no scroll" is skipped).
fn push_scroll_states(el: &UIElement, states: &mut Vec<String>) -> bool {
    if !cached_bool(el, UIProperty::IsScrollPatternAvailable).unwrap_or(false) {
        return false;
    }
    let v = cached_bool(el, UIProperty::ScrollVerticallyScrollable).unwrap_or(false);
    let h = cached_bool(el, UIProperty::ScrollHorizontallyScrollable).unwrap_or(false);
    if let Some(p) = v
        .then(|| cached_f64(el, UIProperty::ScrollVerticalScrollPercent))
        .flatten()
        .filter(|p| *p >= 0.0)
    {
        states.push(format!("v:{}%", p.round() as i64));
    }
    if let Some(p) = h
        .then(|| cached_f64(el, UIProperty::ScrollHorizontalScrollPercent))
        .flatten()
        .filter(|p| *p >= 0.0)
    {
        states.push(format!("h:{}%", p.round() as i64));
    }
    v || h
}

/// Read a slider/spinner/progress value (with min–max context) from cached
/// RangeValue properties. Returns the display string, or None when unavailable.
fn read_range_value(el: &UIElement, ct: ControlType) -> Option<String> {
    if !matches!(
        ct,
        ControlType::Slider | ControlType::Spinner | ControlType::ProgressBar
    ) {
        return None;
    }
    if !cached_bool(el, UIProperty::IsRangeValuePatternAvailable).unwrap_or(false) {
        return None;
    }
    let val = cached_f64(el, UIProperty::RangeValueValue)?;
    match (
        cached_f64(el, UIProperty::RangeValueMinimum),
        cached_f64(el, UIProperty::RangeValueMaximum),
    ) {
        (Some(min), Some(max)) => Some(format!("{} ({}-{})", fmt_num(val), fmt_num(min), fmt_num(max))),
        _ => Some(fmt_num(val)),
    }
}

/// Read one CACHED element into a `RawNode` (children filled by the caller),
/// pushing its live handle into `handles`. Every property read hits the cache
/// populated by `find_first_build_cache` → zero IPC, except the gated edit-value
/// read (a handful of edit-like controls).
fn read_cached(el: &UIElement, handles: &mut Vec<UIElement>) -> RawNode {
    let idx = handles.len();
    let bounds = el
        .get_cached_bounding_rectangle()
        .ok()
        .map(|r| rect_of(&r))
        .unwrap_or(Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
    let ct = el.get_cached_control_type().unwrap_or(ControlType::Custom);
    let name = el
        .get_cached_name()
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            el.get_cached_help_text()
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
    let focusable = el.is_cached_keyboard_focusable().unwrap_or(false);
    let actionable = actionable_from(ct, focusable);
    let protected = el.is_cached_password().ok();
    let value = if protected == Some(false) {
        read_value(el, ct).or_else(|| read_range_value(el, ct))
    } else {
        None
    };
    let mut states = Vec::new();
    if protected == Some(true) {
        states.push("password".to_string());
    }
    if !el.is_cached_enabled().unwrap_or(true) {
        states.push("disabled".to_string());
    }
    if el.has_cached_keyboard_focus().unwrap_or(false) {
        states.push("focused".to_string());
    }
    push_pattern_states(el, ct, &mut states);
    let scrollable = push_scroll_states(el, &mut states);
    let offscreen = is_parked(&bounds) || el.is_cached_offscreen().unwrap_or(false);
    handles.push(el.clone());
    RawNode {
        handle_idx: idx,
        role: ct_to_role(ct).to_string(),
        name,
        value,
        states,
        bounds,
        actionable,
        scrollable,
        offscreen,
        children: Vec::new(),
    }
}

fn live_bool(el: &UIElement, p: UIProperty) -> Option<bool> {
    el.get_property_value(p).ok().and_then(|v| (&v).try_into().ok())
}

/// Read one LIVE element into a `RawNode` (fallback path; one cross-process call
/// per property). Scroll detection is preserved; the richer per-pattern metadata
/// of the cached path is skipped on this rare path.
fn read_live(el: &UIElement, handles: &mut Vec<UIElement>) -> RawNode {
    let idx = handles.len();
    let bounds = el
        .get_bounding_rectangle()
        .ok()
        .map(|r| rect_of(&r))
        .unwrap_or(Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
    let ct = el.get_control_type().unwrap_or(ControlType::Custom);
    let name = el
        .get_name()
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| el.get_help_text().ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
    let focusable = el.is_keyboard_focusable().unwrap_or(false);
    let actionable = actionable_from(ct, focusable);
    let protected = el.is_password().ok();
    let value = if protected == Some(false) { read_value(el, ct) } else { None };
    let mut states = Vec::new();
    if protected == Some(true) {
        states.push("password".to_string());
    }
    if !el.is_enabled().unwrap_or(true) {
        states.push("disabled".to_string());
    }
    if el.has_keyboard_focus().unwrap_or(false) {
        states.push("focused".to_string());
    }
    let scrollable = live_bool(el, UIProperty::IsScrollPatternAvailable).unwrap_or(false)
        && (live_bool(el, UIProperty::ScrollVerticallyScrollable).unwrap_or(false)
            || live_bool(el, UIProperty::ScrollHorizontallyScrollable).unwrap_or(false));
    let offscreen = is_parked(&bounds) || el.is_offscreen().unwrap_or(false);
    handles.push(el.clone());
    RawNode {
        handle_idx: idx,
        role: ct_to_role(ct).to_string(),
        name,
        value,
        states,
        bounds,
        actionable,
        scrollable,
        offscreen,
        children: Vec::new(),
    }
}

/// Recursively build a `RawNode` tree from a CACHED root via `get_cached_children`
/// (zero IPC), capped at `max_depth`.
fn build_raw_cached(
    el: &UIElement,
    depth: usize,
    max_depth: usize,
    handles: &mut Vec<UIElement>,
    depth_capped: &mut bool,
) -> RawNode {
    let mut node = read_cached(el, handles);
    if depth >= max_depth {
        if el.get_cached_children().map(|c| !c.is_empty()).unwrap_or(false) {
            *depth_capped = true;
        }
        return node;
    }
    if let Ok(children) = el.get_cached_children() {
        for child in &children {
            node.children
                .push(build_raw_cached(child, depth + 1, max_depth, handles, depth_capped));
        }
    }
    node
}

/// Distinguish the walker's "no more elements" sentinel (S_OK + null → a
/// `uiautomation::Error` with code 0, `result()` == None) from a genuine COM
/// fault (FAILED HRESULT, `result()` == Some). Masking a real fault as a clean
/// leaf would silently truncate without setting `truncated`.
fn is_end_of_walk(e: &uiautomation::Error) -> bool {
    e.result().is_none()
}

/// Recursively build a `RawNode` tree from a LIVE root via the control-view
/// walker (fallback; per-node live reads), capped at `max_depth`.
fn build_raw_live(
    walker: &UITreeWalker,
    el: &UIElement,
    depth: usize,
    max_depth: usize,
    handles: &mut Vec<UIElement>,
    depth_capped: &mut bool,
) -> RawNode {
    let mut node = read_live(el, handles);
    if depth >= max_depth {
        if walker.get_first_child(el).is_ok() {
            *depth_capped = true;
        }
        return node;
    }
    let mut child = match walker.get_first_child(el) {
        Ok(c) => c,
        Err(e) if is_end_of_walk(&e) => return node,
        Err(_) => {
            *depth_capped = true;
            return node;
        }
    };
    loop {
        node.children
            .push(build_raw_live(walker, &child, depth + 1, max_depth, handles, depth_capped));
        child = match walker.get_next_sibling(&child) {
            Ok(c) => c,
            Err(e) if is_end_of_walk(&e) => break,
            Err(_) => {
                *depth_capped = true;
                break;
            }
        };
    }
    node
}

// ---- window resolution --------------------------------------------------

/// Classify a read/interface-acquisition error before any action dispatch.
/// Display messages do not reliably contain HRESULTs and are not authority to
/// reinterpret an unknown failure as permission denial or missing support.
fn uia_err(op: &str, e: uiautomation::Error) -> A11yError {
    let code = e.code() as u32;
    let message = format!("{op}: UIA error 0x{code:08X}: {e}");
    match code {
        // E_ACCESSDENIED / RPC_E_ACCESS_DENIED.
        0x80070005 | 0x8001011B => A11yError::Permission(message),
        // UIA_E_ELEMENTNOTAVAILABLE / E_HANDLE / invalid window handle.
        0x80040201 | 0x80070006 | 0x80070578 => A11yError::Stale(message),
        _ => A11yError::Backend(message),
    }
}

/// Once a mutating request entered UIA, an error code describes its cause but
/// does not prove that the provider performed no action. Never retry another
/// pattern or authorize synthetic input from this outcome.
fn uia_action_err(op: &str, e: uiautomation::Error) -> A11yError {
    let code = e.code() as u32;
    A11yError::Uncertain(format!(
        "{op}: native UIA request was dispatched but its effect is unknown \
         (0x{code:08X}: {e}); re-run observe and do not automatically retry"
    ))
}

/// Window title via Win32 `GetWindowTextW` (cheap; no UIA round-trip). Empty on
/// failure or no title.
fn window_title(hwnd: HWND) -> Option<String> {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowTextW;
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return None;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, &mut buf);
        if n <= 0 {
            return None;
        }
        let s = String::from_utf16_lossy(&buf[..n as usize]);
        let t = s.trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    }
}

/// Window class name via Win32 `GetClassNameW`.
fn window_class(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    if n <= 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

/// The foreground HWND, rejecting null / our own process / minimized.
fn foreground_hwnd() -> Result<(HWND, u32), A11yError> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return Err(A11yError::NotFound(
                "no foreground window (the desktop may have focus); click a window and retry"
                    .to_string(),
            ));
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == GetCurrentProcessId() {
            return Err(A11yError::NotFound(
                "the foreground window belongs to this app; switch to the target application (or \
                 observe by an explicit pid) and retry"
                    .to_string(),
            ));
        }
        if IsIconic(hwnd).as_bool() {
            return Err(A11yError::NotFound(
                "the foreground window is minimized; restore or focus it first (its controls are \
                 parked off-screen and not actionable while minimized)"
                    .to_string(),
            ));
        }
        Ok((hwnd, pid))
    }
}

/// Human app name (exe basename, without path or `.exe`) for a pid.
fn app_name_from_pid(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        struct Guard(HANDLE);
        impl Drop for Guard {
            fn drop(&mut self) {
                unsafe {
                    let _ = CloseHandle(self.0);
                }
            }
        }
        let _guard = Guard(handle);

        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
            .ok()?;
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        let base = full.rsplit(['\\', '/']).next().unwrap_or(&full);
        let clean = base
            .strip_suffix(".exe")
            .or_else(|| base.strip_suffix(".EXE"))
            .unwrap_or(base);
        if clean.is_empty() {
            None
        } else {
            Some(clean.to_string())
        }
    }
}

/// Get the UIA desktop root, retrying briefly (it transiently fails under load).
fn get_root_with_retry(auto: &UIAutomation) -> Result<UIElement, A11yError> {
    let mut last: Option<uiautomation::Error> = None;
    for attempt in 0u64..3 {
        match auto.get_root_element() {
            Ok(root) => return Ok(root),
            Err(e) => {
                last = Some(e);
                if attempt < 2 {
                    std::thread::sleep(Duration::from_millis(100 * (attempt + 1)));
                }
            }
        }
    }
    Err(uia_err(
        "get_root_element",
        last.expect("the loop records an error on every failing attempt"),
    ))
}

/// The main top-level window of `pid` as a UIA element (prefers the Win32
/// resolution, falls back to the UIA root-children scan).
fn window_for_pid(auto: &UIAutomation, pid: i32) -> Result<UIElement, A11yError> {
    let win = main_window_hwnd(pid as u32)
        .and_then(|hwnd| auto.element_from_handle((hwnd.0 as isize).into()).ok());
    if let Some(el) = win {
        return Ok(el);
    }

    let root = get_root_with_retry(auto)?;
    let cond = auto
        .create_true_condition()
        .map_err(|e| uia_err("create_true_condition", e))?;
    let children = root
        .find_all(TreeScope::Children, &cond)
        .map_err(|e| uia_err("enumerate top-level windows", e))?;
    for c in children {
        if c.get_process_id().map(|p| p as i32 == pid).unwrap_or(false) {
            return Ok(c);
        }
    }
    Err(A11yError::NotFound(format!(
        "no top-level window found for pid {pid} (a UWP/Store app may host its window under \
         ApplicationFrameHost; focus the window and observe the foreground instead)"
    )))
}

// ---- top-level window enumeration (desktop observe) ---------------------

struct FindCtx {
    pid: u32,
    hwnd: Option<HWND>,
}

/// `EnumWindows` callback: record the first visible, top-level (un-owned) window
/// belonging to the target pid, then stop.
unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = unsafe { &mut *(lparam.0 as *mut FindCtx) };
    let mut wpid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut wpid)) };
    if wpid == ctx.pid && unsafe { IsWindowVisible(hwnd) }.as_bool() {
        let owner = unsafe { GetWindow(hwnd, GW_OWNER) };
        let is_top_level = owner.map(|h| h.0.is_null()).unwrap_or(true);
        if is_top_level {
            ctx.hwnd = Some(hwnd);
            return BOOL(0);
        }
    }
    BOOL(1)
}

fn main_window_hwnd(pid: u32) -> Option<HWND> {
    let mut ctx = FindCtx { pid, hwnd: None };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut _ as isize));
    }
    ctx.hwnd
}

struct AppWindowsCtx {
    self_pid: u32,
    out: Vec<(HWND, u32)>,
}

/// `EnumWindows` callback collecting visible, non-minimized, un-owned, titled
/// top-level application windows (in z-order, topmost first), skipping our own
/// process and the desktop shell (`Progman`/`WorkerW`).
unsafe extern "system" fn enum_app_windows(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = unsafe { &mut *(lparam.0 as *mut AppWindowsCtx) };
    if ctx.out.len() >= MAX_WINDOWS {
        return BOOL(0);
    }
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() || unsafe { IsIconic(hwnd) }.as_bool() {
        return BOOL(1);
    }
    let owner = unsafe { GetWindow(hwnd, GW_OWNER) };
    if !owner.map(|h| h.0.is_null()).unwrap_or(true) {
        return BOOL(1); // owned (tool window / dialog child) — skip
    }
    let mut wpid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut wpid)) };
    if wpid == 0 || wpid == ctx.self_pid {
        return BOOL(1);
    }
    if window_title(hwnd).is_none() {
        return BOOL(1); // no title → not a user-facing app window
    }
    let class = window_class(hwnd);
    if class == "Progman" || class == "WorkerW" {
        return BOOL(1); // the desktop itself
    }
    ctx.out.push((hwnd, wpid));
    BOOL(1)
}

/// Enumerate visible top-level application windows (z-order, topmost first),
/// excluding our own process. Bounded by `MAX_WINDOWS`.
fn enumerate_app_windows(self_pid: u32) -> Vec<(HWND, u32)> {
    let mut ctx = AppWindowsCtx {
        self_pid,
        out: Vec::new(),
    };
    unsafe {
        let _ = EnumWindows(Some(enum_app_windows), LPARAM(&mut ctx as *mut _ as isize));
    }
    ctx.out
}

// ---- command handlers ---------------------------------------------------

/// Build one window's `RawNode` subtree (cached path, falling back to the live
/// walker), then re-tag it as a `window` node carrying its title so the semantic
/// renderer prints `window "Title"`. Appends handles into the shared table.
#[allow(clippy::too_many_arguments)]
fn build_window_node(
    auto: &UIAutomation,
    cache: &UICacheRequest,
    cond: &uiautomation::core::UICondition,
    el: &UIElement,
    title: Option<String>,
    opts: &ObserveOpts,
    handles: &mut Vec<UIElement>,
    depth_capped: &mut bool,
) -> Result<RawNode, A11yError> {
    let mut root = match el.find_first_build_cache(TreeScope::Element, cond, cache) {
        Ok(cached_root) => build_raw_cached(&cached_root, 0, opts.max_depth, handles, depth_capped),
        Err(e) => {
            tracing::warn!(
                "UIA cached tree build failed ({e}); falling back to the live control-view walker"
            );
            let walker = auto
                .get_control_view_walker()
                .map_err(|e| uia_err("get_control_view_walker", e))?;
            build_raw_live(&walker, el, 0, opts.max_depth, handles, depth_capped)
        }
    };
    // Re-tag the window root so it renders as `window "Title"` and is excluded
    // from emission (you target controls, not the frame).
    root.role = WINDOW_ROLE.to_string();
    root.name = title.or_else(|| el.get_name().ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
    Ok(root)
}

fn do_observe(opts: &ObserveOpts, state: &mut State) -> Result<Snapshot, A11yError> {
    let self_pid = unsafe { GetCurrentProcessId() };

    // Resolve the windows to capture. pid==Some → just that app's main window;
    // pid==None → the desktop: the foreground window + other visible app windows.
    struct WinTarget {
        el: UIElement,
        title: Option<String>,
        pid: u32,
    }
    let mut targets: Vec<WinTarget> = Vec::new();

    match opts.pid {
        Some(pid) => {
            let el = window_for_pid(&state.automation, pid)?;
            let title = el.get_name().ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            targets.push(WinTarget {
                title,
                el,
                pid: pid as u32,
            });
        }
        None => {
            let mut seen: Vec<isize> = Vec::new();
            // Foreground first (so it numbers first and is the snapshot's pid).
            let fg = foreground_hwnd().ok().and_then(|(hwnd, pid)| {
                state
                    .automation
                    .element_from_handle((hwnd.0 as isize).into())
                    .ok()
                    .map(|el| (hwnd, pid, el))
            });
            if let Some((hwnd, pid, el)) = fg {
                seen.push(hwnd.0 as isize);
                targets.push(WinTarget {
                    title: window_title(hwnd),
                    el,
                    pid,
                });
            }
            // Then the other visible top-level application windows (z-order).
            for (hwnd, pid) in enumerate_app_windows(self_pid) {
                if seen.contains(&(hwnd.0 as isize)) {
                    continue;
                }
                if targets.len() >= MAX_WINDOWS {
                    break;
                }
                if let Ok(el) = state.automation.element_from_handle((hwnd.0 as isize).into()) {
                    targets.push(WinTarget {
                        title: window_title(hwnd),
                        el,
                        pid,
                    });
                }
            }
            if targets.is_empty() {
                return Err(A11yError::NotFound(
                    "no observable application window (the desktop or this app may have focus); \
                     open or focus a target application and retry"
                        .to_string(),
                ));
            }
        }
    }

    // The primary window (foreground / the requested pid) defines the snapshot's
    // pid / app / title.
    let primary_pid = targets[0].pid;
    let window_title_primary = targets[0].title.clone();
    let app_name = app_name_from_pid(primary_pid);

    // Build the desktop forest: one cached IPC per window; per-window failures
    // are logged and skipped rather than failing the whole observe.
    let cache = build_cache_request(&state.automation)?;
    let cond = state
        .automation
        .create_true_condition()
        .map_err(|e| uia_err("create_true_condition", e))?;

    let mut handles: Vec<UIElement> = Vec::new();
    let mut window_by_handle: HashMap<usize, Rc<WindowEntry>> = HashMap::new();
    let mut depth_capped = false;
    let mut window_nodes: Vec<RawNode> = Vec::new();
    for t in &targets {
        let first_handle = handles.len();
        match build_window_node(
            &state.automation,
            &cache,
            &cond,
            &t.el,
            t.title.clone(),
            opts,
            &mut handles,
            &mut depth_capped,
        ) {
            Ok(node) => {
                let window = Rc::new(WindowEntry {
                    el: t.el.clone(),
                    runtime_id: t.el.get_runtime_id().unwrap_or_default(),
                    observed_bounds: node.bounds,
                });
                for hidx in first_handle..handles.len() {
                    window_by_handle.insert(hidx, window.clone());
                }
                window_nodes.push(node);
            }
            Err(e) => tracing::warn!("skipping window during observe: {e}"),
        }
    }
    if window_nodes.is_empty() {
        return Err(A11yError::Backend(
            "failed to capture any window's UI tree (the target may be elevated or closing)"
                .to_string(),
        ));
    }

    let desktop = RawNode::structural(
        DESKTOP_ROLE,
        None,
        Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
        window_nodes,
    );

    let (entries, handle_indices, ref_by_handle, budget_truncated) =
        tree_map::build_entries(&desktop, opts.max_depth, opts.node_budget);
    let truncated = depth_capped || budget_truncated;
    let text = tree_map::render_tree(&desktop, &ref_by_handle);

    // Bump the snapshot generation (wrapping, skipping 0 — the initial sentinel).
    state.gen_counter = state.gen_counter.wrapping_add(1);
    if state.gen_counter == 0 {
        state.gen_counter = 1;
    }
    let generation = SnapshotGen(state.gen_counter);
    state.current_gen = generation;

    // Rebuild ref→handle, capturing each emitted element's RuntimeId for a
    // robust stale check at invoke time (an identity compare beats a string
    // match on the HRESULT).
    state.registry.clear();
    for (entry, &hidx) in entries.iter().zip(handle_indices.iter()) {
        if let Some(h) = handles.get(hidx) {
            let runtime_id = h.get_runtime_id().unwrap_or_default();
            state.registry.insert(
                entry.r#ref,
                RegEntry {
                    el: h.clone(),
                    runtime_id,
                    observed_bounds: entry.bounds,
                    window: window_by_handle.get(&hidx).cloned(),
                },
            );
        }
    }

    Ok(Snapshot {
        generation,
        entries,
        text,
        truncated,
        pid: Some(primary_pid as i32),
        app_name,
        window_title: window_title_primary,
    })
}

fn strict_availability(value: Value) -> Result<bool, A11yError> {
    match value {
        Value::BOOL(available) => Ok(available),
        _ => Err(A11yError::Backend(
            "UIA pattern availability was not a BOOL; support is unknown and no action was dispatched"
                .to_string(),
        )),
    }
}

fn pattern_available(elem: &UIElement, property: UIProperty) -> Result<bool, A11yError> {
    let variant = elem
        .get_property_value(property)
        .map_err(|e| uia_err("read live pattern availability", e))?;
    let value: Value = (&variant)
        .try_into()
        .map_err(|e| uia_err("decode live pattern availability", e))?;
    strict_availability(value)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PressPattern {
    Invoke,
    Toggle,
    SelectionItem,
    ExpandCollapse,
    Legacy,
}

impl PressPattern {
    fn availability(self) -> UIProperty {
        match self {
            Self::Invoke => UIProperty::IsInvokePatternAvailable,
            Self::Toggle => UIProperty::IsTogglePatternAvailable,
            Self::SelectionItem => UIProperty::IsSelectionItemPatternAvailable,
            Self::ExpandCollapse => UIProperty::IsExpandCollapsePatternAvailable,
            Self::Legacy => UIProperty::IsLegacyIAccessiblePatternAvailable,
        }
    }
}

/// Choose one pattern using only live support reads. A missing/unknown read
/// stops selection; only an explicit false authorizes trying the next candidate.
fn select_press_pattern(
    mut available: impl FnMut(PressPattern) -> Result<bool, A11yError>,
) -> Result<Option<PressPattern>, A11yError> {
    for pattern in [
        PressPattern::Invoke,
        PressPattern::Toggle,
        PressPattern::SelectionItem,
        PressPattern::ExpandCollapse,
        PressPattern::Legacy,
    ] {
        if available(pattern)? {
            return Ok(Some(pattern));
        }
    }
    Ok(None)
}

fn require_enabled(elem: &UIElement) -> Result<(), A11yError> {
    if !elem
        .is_enabled()
        .map_err(|e| uia_err("read element enabled state", e))?
    {
        return Err(A11yError::Backend(
            "the element is disabled; no action was dispatched".to_string(),
        ));
    }
    Ok(())
}

fn require_writable(readonly: bool) -> Result<(), A11yError> {
    if readonly {
        Err(A11yError::Backend(
            "the element value is read-only; no action was dispatched".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn checked_range_value(value: &str, minimum: f64, maximum: f64) -> Result<f64, A11yError> {
    let number = value.trim().parse::<f64>().map_err(|_| {
        A11yError::Backend("range value must be a finite number; no action was dispatched".to_string())
    })?;
    if !number.is_finite()
        || !minimum.is_finite()
        || !maximum.is_finite()
        || minimum > maximum
        || number < minimum
        || number > maximum
    {
        return Err(A11yError::Backend(
            "range value or provider bounds are invalid; no action was dispatched".to_string(),
        ));
    }
    Ok(number)
}

/// Select before dispatch, then call exactly one mutator. Acquisition errors
/// are terminal reads; action errors are uncertain and never continue the chain.
fn press_chain(elem: &UIElement) -> Result<&'static str, A11yError> {
    require_enabled(elem)?;
    let pattern = select_press_pattern(|p| pattern_available(elem, p.availability()))?
        .ok_or_else(|| A11yError::Unsupported {
            capability: "semantic activation".to_string(),
            hint: "Live UIA reports no Invoke, Toggle, SelectionItem, ExpandCollapse or Legacy pattern; no action was dispatched."
                .to_string(),
        })?;
    match pattern {
        PressPattern::Invoke => {
            let p = elem.get_pattern::<patterns::UIInvokePattern>()
                .map_err(|e| uia_err("get Invoke pattern", e))?;
            p.invoke().map_err(|e| uia_action_err("Invoke", e))?;
            Ok("invoke")
        }
        PressPattern::Toggle => {
            let p = elem.get_pattern::<patterns::UITogglePattern>()
                .map_err(|e| uia_err("get Toggle pattern", e))?;
            p.toggle().map_err(|e| uia_action_err("Toggle", e))?;
            Ok("toggle")
        }
        PressPattern::SelectionItem => {
            let p = elem.get_pattern::<patterns::UISelectionItemPattern>()
                .map_err(|e| uia_err("get SelectionItem pattern", e))?;
            p.select().map_err(|e| uia_action_err("SelectionItem.Select", e))?;
            Ok("select")
        }
        PressPattern::ExpandCollapse => {
            let p = elem.get_pattern::<patterns::UIExpandCollapsePattern>()
                .map_err(|e| uia_err("get ExpandCollapse pattern", e))?;
            p.expand().map_err(|e| uia_action_err("ExpandCollapse.Expand", e))?;
            Ok("expand")
        }
        PressPattern::Legacy => {
            let p = elem.get_pattern::<patterns::UILegacyIAccessiblePattern>()
                .map_err(|e| uia_err("get LegacyIAccessible pattern", e))?;
            p.do_default_action().map_err(|e| uia_action_err("LegacyIAccessible.DoDefaultAction", e))?;
            Ok("legacy default action")
        }
    }
}

/// Shared retained-ref lookup. Both semantic invocation and pixel resolution
/// validate live identity after this generation/ref check.
fn registered_ref<'a>(
    target: &Target,
    generation: SnapshotGen,
    state: &'a State,
) -> Result<(u32, &'a RegEntry), A11yError> {
    let r = match target {
        Target::Ref(r) => *r,
        Target::Pixel { .. } => {
            return Err(A11yError::Unsupported {
                capability: "pixel targeting".to_string(),
                hint: "Pixel fallback is handled by the computer tool's input layer, not the \
                       accessibility engine."
                    .to_string(),
            });
        }
    };
    if generation != state.current_gen {
        return Err(A11yError::Stale(format!(
            "ref [{r}] is from an older snapshot (the UI may have changed); re-run observe and use \
             a fresh [ref]"
        )));
    }
    let reg = state
        .registry
        .get(&r)
        .ok_or_else(|| A11yError::NotFound(format!("no element [{r}] in the latest snapshot")))?;
    Ok((r, reg))
}

fn validate_live_identity(
    el: &UIElement,
    observed_runtime_id: &[i32],
    description: &str,
) -> Result<(), A11yError> {
    if observed_runtime_id.is_empty() {
        return Err(A11yError::Stale(format!(
            "{description} had no readable runtime identity at observe time; re-run observe"
        )));
    }
    let live = el
        .get_runtime_id()
        .map_err(|e| uia_err(&format!("read {description} runtime identity"), e))?;
    if live != observed_runtime_id {
        return Err(A11yError::Stale(format!(
            "{description} no longer exists or was replaced; re-run observe"
        )));
    }
    Ok(())
}

fn valid_pixel_bounds(bounds: Rect) -> bool {
    !is_parked(&bounds) && bounds.pixel_center().is_some()
}

fn validate_live_bounds(
    el: &UIElement,
    observed: Rect,
    description: &str,
) -> Result<Rect, A11yError> {
    let live = el
        .get_bounding_rectangle()
        .map(|r| rect_of(&r))
        .map_err(|e| uia_err(&format!("read {description} bounds"), e))?;
    if !valid_pixel_bounds(observed) || !valid_pixel_bounds(live) || live != observed {
        return Err(A11yError::Stale(format!(
            "{description} bounds changed or are not valid for pixel input; re-run observe"
        )));
    }
    let offscreen = el
        .is_offscreen()
        .map_err(|e| uia_err(&format!("read {description} visibility"), e))?;
    if offscreen {
        return Err(A11yError::Stale(format!(
            "{description} is no longer on-screen; re-run observe"
        )));
    }
    Ok(live)
}

/// Walk only live raw-view parents, without focusing or invoking anything.
/// The cap also rejects broken/cyclic provider trees without unbounded IPC.
fn is_within_element(
    automation: &UIAutomation,
    element: &UIElement,
    ancestor: &UIElement,
) -> Result<bool, A11yError> {
    let walker = automation
        .get_raw_view_walker()
        .map_err(|e| uia_err("get raw-view walker for pixel target", e))?;
    let mut current = element.clone();
    for _ in 0..256 {
        if automation
            .compare_elements(&current, ancestor)
            .map_err(|e| uia_err("compare pixel target identity", e))?
        {
            return Ok(true);
        }
        current = match walker.get_parent(&current) {
            Ok(parent) => parent,
            Err(e) if is_end_of_walk(&e) => return Ok(false),
            Err(e) => return Err(uia_err("read pixel target parent", e)),
        };
    }
    Err(A11yError::Stale(
        "pixel target ancestry could not be validated within the traversal limit; re-run observe"
            .to_string(),
    ))
}

fn do_resolve_pixel_target(
    target: &Target,
    generation: SnapshotGen,
    state: &State,
) -> Result<Rect, A11yError> {
    let (r, reg) = registered_ref(target, generation, state)?;
    let description = format!("element [{r}]");
    validate_live_identity(&reg.el, &reg.runtime_id, &description)?;
    let window = reg.window.as_ref().ok_or_else(|| {
        A11yError::Stale(format!(
            "element [{r}] has no retained owning window; re-run observe"
        ))
    })?;
    validate_live_identity(&window.el, &window.runtime_id, "owning window")?;
    validate_live_bounds(&window.el, window.observed_bounds, "owning window")?;
    let bounds = validate_live_bounds(&reg.el, reg.observed_bounds, &description)?;
    if !is_within_element(&state.automation, &reg.el, &window.el)? {
        return Err(A11yError::Stale(format!(
            "element [{r}] is no longer in its observed window; re-run observe"
        )));
    }

    // Windows observe includes visible background windows. A read-only hit
    // test preserves those targets while rejecting an unchanged element whose
    // center is now covered by another window/control.
    let (x, y) = bounds.pixel_center().ok_or_else(|| A11yError::Stale(
        "target bounds contain no representable pixel center; re-run observe".to_string(),
    ))?;
    let hit = state
        .automation
        .element_from_point(Point::new(x, y))
        .map_err(|e| uia_err("hit-test pixel target center", e))?;
    if !is_within_element(&state.automation, &hit, &reg.el)? {
        return Err(A11yError::Stale(format!(
            "element [{r}] no longer owns the pointer target at its observed center; re-run observe"
        )));
    }
    // Parent/hit-test reads may themselves take time. Recheck identity and
    // geometry at the end before handing coordinates back to the input layer.
    validate_live_identity(&window.el, &window.runtime_id, "owning window")?;
    validate_live_identity(&reg.el, &reg.runtime_id, &description)?;
    validate_live_bounds(&window.el, window.observed_bounds, "owning window")?;
    validate_live_bounds(&reg.el, reg.observed_bounds, &description)
}

fn do_invoke(
    target: &Target,
    generation: SnapshotGen,
    action: &ElementAction,
    state: &State,
) -> Result<Effect, A11yError> {
    let (r, reg) = registered_ref(target, generation, state)?;

    validate_live_identity(&reg.el, &reg.runtime_id, &format!("element [{r}]"))?;
    let elem = &reg.el;

    match action {
        ElementAction::Press | ElementAction::LeftClick | ElementAction::DoubleClick => {
            let via = press_chain(elem)?;
            Ok(Effect {
                changed: true,
                message: format!("activated element [{r}] via {via}"),
            })
        }
        ElementAction::RightClick => Err(A11yError::Unsupported {
            capability: "right-click via accessibility".to_string(),
            hint: "Right-click is not a UIA action; the computer tool performs a pixel right-click \
                   at the element center."
                .to_string(),
        }),
        ElementAction::Focus => {
            elem.set_focus()
                .map_err(|e| uia_action_err(&format!("focus [{r}]"), e))?;
            Ok(Effect {
                changed: true,
                message: format!("focused element [{r}]"),
            })
        }
        ElementAction::SetValue(v) => {
            require_enabled(elem)?;
            if pattern_available(elem, UIProperty::IsValuePatternAvailable)? {
                let p = elem
                    .get_pattern::<patterns::UIValuePattern>()
                    .map_err(|e| uia_err("get Value pattern", e))?;
                require_writable(p.is_readonly().map_err(|e| uia_err("read Value writability", e))?)?;
                p.set_value(v.as_str())
                    .map_err(|e| uia_action_err(&format!("set value of [{r}]"), e))?;
                return Ok(Effect {
                    changed: true,
                    message: format!("set value of element [{r}]"),
                });
            }
            if pattern_available(elem, UIProperty::IsRangeValuePatternAvailable)? {
                let rng = elem
                    .get_pattern::<patterns::UIRangeValuePattern>()
                    .map_err(|e| uia_err("get RangeValue pattern", e))?;
                require_writable(rng.is_readonly().map_err(|e| uia_err("read RangeValue writability", e))?)?;
                let minimum = rng.get_minimum().map_err(|e| uia_err("read RangeValue minimum", e))?;
                let maximum = rng.get_maximum().map_err(|e| uia_err("read RangeValue maximum", e))?;
                let num = checked_range_value(v, minimum, maximum)?;
                rng.set_value(num)
                    .map_err(|e| uia_action_err(&format!("set range value of [{r}]"), e))?;
                return Ok(Effect {
                    changed: true,
                    message: format!("set element [{r}] to {num}"),
                });
            }
            Err(A11yError::Unsupported {
                capability: "semantic value assignment".to_string(),
                hint: format!("Live UIA reports no Value or RangeValue pattern for [{r}]; no action was dispatched."),
            })
        }
    }
}

fn do_focus(pid: i32) -> Result<Effect, A11yError> {
    let hwnd = main_window_hwnd(pid as u32).ok_or_else(|| {
        A11yError::NotFound(format!("no visible top-level window found for pid {pid}"))
    })?;
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if SetForegroundWindow(hwnd).as_bool() {
            Ok(Effect {
                changed: true,
                message: format!("brought pid {pid} to the front"),
            })
        } else {
            Err(A11yError::Backend(format!(
                "Windows refused to bring pid {pid} to the foreground (foreground-stealing is \
                 restricted by the OS); click the window or use the pixel fallback to focus it"
            )))
        }
    }
}

// ---- actor thread state + command plumbing ------------------------------

/// The captured top-level window retained by each emitted descendant.
struct WindowEntry {
    el: UIElement,
    runtime_id: Vec<i32>,
    observed_bounds: Rect,
}

/// A live UIA handle and its observed identity, bounds and owning window.
struct RegEntry {
    el: UIElement,
    runtime_id: Vec<i32>,
    observed_bounds: Rect,
    window: Option<Rc<WindowEntry>>,
}

struct State {
    gen_counter: u64,
    current_gen: SnapshotGen,
    registry: HashMap<u32, RegEntry>,
    automation: UIAutomation,
}

enum Cmd {
    Observe(ObserveOpts, Sender<Result<Snapshot, A11yError>>),
    ResolvePixelTarget(Target, SnapshotGen, Sender<Result<Rect, A11yError>>),
    Invoke(
        Target,
        SnapshotGen,
        ElementAction,
        Sender<Result<Effect, A11yError>>,
    ),
    Focus(i32, Sender<Result<Effect, A11yError>>),
}

pub struct ActorHandle {
    tx: Mutex<Sender<Cmd>>,
}

impl ActorHandle {
    pub fn spawn() -> Result<Self, A11yError> {
        let (tx, rx) = channel::<Cmd>();
        let (init_tx, init_rx) = channel::<Result<(), A11yError>>();

        std::thread::Builder::new()
            .name("nomi-a11y-windows".to_string())
            .spawn(move || {
                let init = unsafe {
                    let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
                    if hr.is_err() && hr.0 != 0x80010106u32 as i32 {
                        Err(A11yError::Backend(format!(
                            "CoInitializeEx(MTA) failed: HRESULT 0x{:08X}",
                            hr.0
                        )))
                    } else {
                        Ok(())
                    }
                }
                .and_then(|()| {
                    UIAutomation::new_direct().map_err(|e| {
                        A11yError::Backend(format!("failed to create UIAutomation: {e}"))
                    })
                });

                let automation = match init {
                    Ok(a) => {
                        let _ = init_tx.send(Ok(()));
                        a
                    }
                    Err(e) => {
                        let _ = init_tx.send(Err(e));
                        return;
                    }
                };

                let mut state = State {
                    gen_counter: 0,
                    current_gen: SnapshotGen(0),
                    registry: HashMap::new(),
                    automation,
                };

                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        Cmd::Observe(opts, reply) => {
                            let _ = reply.send(do_observe(&opts, &mut state));
                        }
                        Cmd::ResolvePixelTarget(target, generation, reply) => {
                            let _ = reply.send(do_resolve_pixel_target(&target, generation, &state));
                        }
                        Cmd::Invoke(target, generation, action, reply) => {
                            let _ = reply.send(do_invoke(&target, generation, &action, &state));
                        }
                        Cmd::Focus(pid, reply) => {
                            let _ = reply.send(do_focus(pid));
                        }
                    }
                }
            })
            .map_err(|e| A11yError::Backend(format!("failed to start UIA actor thread: {e}")))?;

        init_rx
            .recv()
            .map_err(|_| A11yError::Backend("UIA actor thread died during init".to_string()))??;

        Ok(Self { tx: Mutex::new(tx) })
    }

    fn send(&self, cmd: Cmd) -> Result<(), A11yError> {
        self.tx
            .lock()
            .map_err(|_| A11yError::Backend("UIA actor lock poisoned".to_string()))?
            .send(cmd)
            .map_err(|_| A11yError::Backend("UIA actor thread is gone".to_string()))
    }

    pub fn observe(&self, opts: ObserveOpts) -> Result<Snapshot, A11yError> {
        let (tx, rx) = channel();
        self.send(Cmd::Observe(opts, tx))?;
        rx.recv()
            .map_err(|_| A11yError::Backend("UIA actor dropped the reply".to_string()))?
    }

    pub fn invoke(
        &self,
        target: Target,
        generation: SnapshotGen,
        action: ElementAction,
    ) -> Result<Effect, A11yError> {
        let (tx, rx) = channel();
        self.send(Cmd::Invoke(target, generation, action, tx))?;
        rx.recv()
            .map_err(|_| lost_mutating_reply("invoke"))?
    }

    pub fn resolve_pixel_target(
        &self,
        target: Target,
        generation: SnapshotGen,
    ) -> Result<Rect, A11yError> {
        let (tx, rx) = channel();
        self.send(Cmd::ResolvePixelTarget(target, generation, tx))?;
        rx.recv()
            .map_err(|_| A11yError::Backend("UIA actor dropped the reply".to_string()))?
    }

    pub fn focus_window(&self, pid: i32) -> Result<Effect, A11yError> {
        let (tx, rx) = channel();
        self.send(Cmd::Focus(pid, tx))?;
        rx.recv()
            .map_err(|_| lost_mutating_reply("focus window"))?
    }


}

fn lost_mutating_reply(operation: &str) -> A11yError {
    A11yError::Uncertain(format!(
        "UIA actor accepted the mutating {operation} request but its reply was lost; \
         the effect may have occurred; re-run observe and do not automatically retry"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lost_actor_mutation_reply_is_uncertain_without_claiming_native_dispatch() {
        for operation in ["invoke", "focus window"] {
            let outcome = lost_mutating_reply(operation);
            assert!(matches!(&outcome, A11yError::Uncertain(_)));
            let message = outcome.to_string();
            assert!(message.contains("actor accepted"));
            assert!(message.contains("may have occurred"));
            assert!(!message.contains("native UIA request was dispatched"));
        }
    }

    #[test]
    fn semantic_support_requires_bool_instead_of_coerced_metadata() {
        assert!(strict_availability(Value::BOOL(true)).unwrap());
        assert!(!strict_availability(Value::BOOL(false)).unwrap());
        for value in [
            Value::EMPTY,
            Value::NULL,
            Value::I4(0),
            Value::STRING("false".to_string()),
        ] {
            assert!(matches!(strict_availability(value), Err(A11yError::Backend(_))));
        }
    }

    #[test]
    fn semantic_pattern_selection_preserves_priority_and_stops_on_first_support() {
        let mut queries = Vec::new();
        let selected = select_press_pattern(|pattern| {
            queries.push(pattern);
            Ok(pattern != PressPattern::Invoke)
        }).unwrap();
        assert_eq!(selected, Some(PressPattern::Toggle));
        assert_eq!(queries, [PressPattern::Invoke, PressPattern::Toggle]);

        queries.clear();
        assert_eq!(select_press_pattern(|pattern| {
            queries.push(pattern);
            Ok(false)
        }).unwrap(), None);
        assert_eq!(queries, [
            PressPattern::Invoke,
            PressPattern::Toggle,
            PressPattern::SelectionItem,
            PressPattern::ExpandCollapse,
            PressPattern::Legacy,
        ]);
    }

    #[test]
    fn semantic_pattern_selection_never_treats_unknown_support_as_absent() {
        let mut queries = Vec::new();
        let outcome = select_press_pattern(|pattern| {
            queries.push(pattern);
            if pattern == PressPattern::Invoke {
                Ok(false)
            } else {
                Err(A11yError::Backend("availability read failed".to_string()))
            }
        });
        assert!(matches!(outcome, Err(A11yError::Backend(_))));
        assert_eq!(queries, [PressPattern::Invoke, PressPattern::Toggle]);
    }

    #[test]
    fn uia_error_certainty_depends_on_dispatch_phase_not_only_hresult() {
        for code in [0x80070005u32, 0x8001011B] {
            assert!(matches!(
                uia_err("read", uiautomation::Error::new(code as i32, "opaque error")),
                A11yError::Permission(_)
            ));
        }
        for code in [0x80040201u32, 0x80070006, 0x80070578] {
            assert!(matches!(
                uia_err("read", uiautomation::Error::new(code as i32, "opaque error")),
                A11yError::Stale(_)
            ));
        }
        for code in [0u32, 2, 0x80040204, 0x80004005] {
            assert!(matches!(
                uia_err("read", uiautomation::Error::new(code as i32, "denied")),
                A11yError::Backend(_)
            ));
        }
        for code in [0u32, 2, 0x80070005, 0x80040201, 0x80040204, 0x80131505, 0x80004005] {
            let outcome = uia_action_err("mutator", uiautomation::Error::new(code as i32, "opaque error"));
            assert!(matches!(&outcome, A11yError::Uncertain(_)));
            assert!(outcome.to_string().contains(&format!("0x{code:08X}")));
        }
    }

    #[test]
    fn range_and_readonly_rejections_do_not_authorize_input_fallback() {
        assert_eq!(checked_range_value("-5.5", -10.0, 10.0).unwrap(), -5.5);
        assert_eq!(checked_range_value("10", -10.0, 10.0).unwrap(), 10.0);
        for (value, minimum, maximum) in [
            ("invalid", 0.0, 10.0),
            ("NaN", 0.0, 10.0),
            ("inf", 0.0, 10.0),
            ("11", 0.0, 10.0),
            ("-1", 0.0, 10.0),
            ("1", f64::NAN, 10.0),
            ("1", 0.0, f64::INFINITY),
            ("1", 10.0, 0.0),
        ] {
            assert!(matches!(checked_range_value(value, minimum, maximum), Err(A11yError::Backend(_))));
        }
        assert!(require_writable(false).is_ok());
        assert!(matches!(require_writable(true), Err(A11yError::Backend(_))));
    }

    #[test]
    fn rectangle_dimensions_do_not_overflow_or_add_a_pixel() {
        let wide = rect_of(&uiautomation::types::Rect::new(
            i32::MIN, i32::MIN, i32::MAX, i32::MAX,
        ));
        assert_eq!(wide.w, u32::MAX as f64);
        assert_eq!(wide.h, u32::MAX as f64);
        assert!(rect_of(&uiautomation::types::Rect::new(3, 4, 3, 4)).is_empty());
        assert!(rect_of(&uiautomation::types::Rect::new(3, 4, -3, -4)).is_empty());
    }
}
