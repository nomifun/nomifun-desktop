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
use std::sync::Mutex;
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

use uiautomation::controls::ControlType;
use uiautomation::core::UICacheRequest;
use uiautomation::types::{TreeScope, UIProperty};
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

/// Read an Edit/Document/RichEdit control's text via the Text pattern (capped
/// at the provider), for controls whose Value pattern is read-only/empty.
fn text_via_text_pattern(elem: &UIElement) -> Option<String> {
    const MAX: i32 = 2000; // UTF-16 code units; the cap is applied provider-side
    let pat = elem.get_pattern::<patterns::UITextPattern>().ok()?;
    let range = pat.get_document_range().ok()?;
    let text = range.get_text(MAX).ok()?;
    let t = text.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

/// Read an element's editable text value (only for edit-like controls). Prefers
/// the Value pattern, falling back to the Text pattern for RichEdit/Document.
fn read_value(elem: &UIElement, ct: ControlType) -> Option<String> {
    if !matches!(
        ct,
        ControlType::Edit | ControlType::Document | ControlType::ComboBox
    ) {
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
        w: (r.get_right() - left) as f64,
        h: (r.get_bottom() - top) as f64,
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
    let value = read_value(el, ct).or_else(|| read_range_value(el, ct));
    let mut states = Vec::new();
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
        .filter(|s| !s.is_empty());
    let focusable = el.is_keyboard_focusable().unwrap_or(false);
    let actionable = actionable_from(ct, focusable);
    let value = read_value(el, ct);
    let mut states = Vec::new();
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

/// Map a `uiautomation` error to an `A11yError`, surfacing UIPI (integrity)
/// access denials as a clear, actionable message instead of a raw HRESULT.
fn uia_err(op: &str, e: uiautomation::Error) -> A11yError {
    let msg = e.to_string();
    let low = msg.to_lowercase();
    if low.contains("0x80070005") || low.contains("access is denied") || low.contains("denied") {
        A11yError::Backend(format!(
            "{op}: access denied. The target window is likely running as administrator \
             (higher integrity); UI Automation cannot read or act on an elevated process from a \
             non-elevated one. Restart this app as administrator to interact with it — this is \
             never done automatically."
        ))
    } else {
        A11yError::Backend(format!("{op}: {msg}"))
    }
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
    let mut depth_capped = false;
    let mut window_nodes: Vec<RawNode> = Vec::new();
    for t in &targets {
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
            Ok(node) => window_nodes.push(node),
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
                },
            );
        }
    }

    Ok(Snapshot {
        generation,
        entries,
        overlay: None, // the computer tool captures the screenshot + draws the overlay
        text,
        truncated,
        pid: Some(primary_pid as i32),
        app_name,
        window_title: window_title_primary,
    })
}

/// Try a chain of activation patterns for a generic "press/click". Returns the
/// name of the pattern that fired (only when BOTH `get_pattern` AND the action
/// succeed). None → the tool layer falls back to a pixel click.
fn press_chain(elem: &UIElement) -> Option<&'static str> {
    if elem
        .get_pattern::<patterns::UIInvokePattern>()
        .ok()
        .and_then(|p| p.invoke().ok())
        .is_some()
    {
        return Some("invoke");
    }
    if elem
        .get_pattern::<patterns::UITogglePattern>()
        .ok()
        .and_then(|p| p.toggle().ok())
        .is_some()
    {
        return Some("toggle");
    }
    if elem
        .get_pattern::<patterns::UISelectionItemPattern>()
        .ok()
        .and_then(|p| p.select().ok())
        .is_some()
    {
        return Some("select");
    }
    if elem
        .get_pattern::<patterns::UIExpandCollapsePattern>()
        .ok()
        .and_then(|p| p.expand().ok())
        .is_some()
    {
        return Some("expand");
    }
    if elem
        .get_pattern::<patterns::UILegacyIAccessiblePattern>()
        .ok()
        .and_then(|p| p.do_default_action().ok())
        .is_some()
    {
        return Some("legacy default action");
    }
    None
}

/// True if a `uiautomation` error is `UIA_E_ELEMENTNOTAVAILABLE` (0x80040201).
fn is_element_unavailable(e: &uiautomation::Error) -> bool {
    e.to_string().to_lowercase().contains("0x80040201")
}

/// Within-generation liveness: the ref is from the current snapshot, but the
/// underlying control may have been destroyed/replaced. Re-read its RuntimeId
/// and compare to the captured one; a mismatch or an unavailable error means the
/// element is stale → the model should re-observe.
fn is_stale(reg: &RegEntry) -> bool {
    match reg.el.get_runtime_id() {
        Ok(rid) => !reg.runtime_id.is_empty() && rid != reg.runtime_id,
        Err(e) => is_element_unavailable(&e),
    }
}

fn do_invoke(
    target: &Target,
    generation: SnapshotGen,
    action: &ElementAction,
    state: &State,
) -> Result<Effect, A11yError> {
    let r = match target {
        Target::Ref(r) => *r,
        Target::Selector(_) => {
            return Err(A11yError::Unsupported {
                capability: "selector targeting".to_string(),
                hint: "Resolve a selector against the latest observe() result and act by [ref]; \
                       direct selector actuation is not implemented."
                    .to_string(),
            });
        }
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

    if is_stale(reg) {
        return Err(A11yError::Stale(format!(
            "element [{r}] no longer exists or was replaced (the UI changed since the last \
             observe); re-run observe and use a fresh [ref]"
        )));
    }
    let elem = &reg.el;

    match action {
        ElementAction::Press | ElementAction::LeftClick | ElementAction::DoubleClick => {
            match press_chain(elem) {
                Some(via) => Ok(Effect {
                    changed: true,
                    message: format!("activated element [{r}] via {via}"),
                }),
                None => Err(A11yError::Backend(format!(
                    "element [{r}] exposes no actionable UIA pattern \
                     (Invoke/Toggle/SelectionItem/ExpandCollapse/LegacyDefaultAction all failed); \
                     fall back to a pixel click at its center"
                ))),
            }
        }
        ElementAction::RightClick => Err(A11yError::Unsupported {
            capability: "right-click via accessibility".to_string(),
            hint: "Right-click is not a UIA action; the computer tool performs a pixel right-click \
                   at the element center."
                .to_string(),
        }),
        ElementAction::Focus => {
            elem.set_focus()
                .map_err(|e| uia_err(&format!("focus [{r}]"), e))?;
            Ok(Effect {
                changed: true,
                message: format!("focused element [{r}]"),
            })
        }
        ElementAction::SetValue(v) => {
            if elem
                .get_pattern::<patterns::UIValuePattern>()
                .and_then(|p| p.set_value(v.as_str()))
                .is_ok()
            {
                return Ok(Effect {
                    changed: true,
                    message: format!("set value of element [{r}]"),
                });
            }
            let as_num = v.trim().parse::<f64>().ok();
            let range_pat = elem.get_pattern::<patterns::UIRangeValuePattern>().ok();
            if let (Some(num), Some(rng)) = (as_num, range_pat) {
                rng.set_value(num)
                    .map_err(|e| uia_err(&format!("set range value of [{r}]"), e))?;
                return Ok(Effect {
                    changed: true,
                    message: format!("set element [{r}] to {num}"),
                });
            }
            Err(A11yError::Backend(format!(
                "element [{r}] does not support setting a value (no Value or RangeValue pattern); \
                 fall back to focusing the field and typing"
            )))
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

/// A registry entry: the live UIA handle for a `[ref]` plus the RuntimeId it had
/// at capture time (for a robust stale check at invoke).
struct RegEntry {
    el: UIElement,
    runtime_id: Vec<i32>,
}

struct State {
    gen_counter: u64,
    current_gen: SnapshotGen,
    registry: HashMap<u32, RegEntry>,
    automation: UIAutomation,
}

enum Cmd {
    Observe(ObserveOpts, Sender<Result<Snapshot, A11yError>>),
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
            .map_err(|_| A11yError::Backend("UIA actor dropped the reply".to_string()))?
    }

    pub fn focus_window(&self, pid: i32) -> Result<Effect, A11yError> {
        let (tx, rx) = channel();
        self.send(Cmd::Focus(pid, tx))?;
        rx.recv()
            .map_err(|_| A11yError::Backend("UIA actor dropped the reply".to_string()))?
    }
}
