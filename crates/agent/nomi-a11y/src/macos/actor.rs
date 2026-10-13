//! The macOS AX actor: a single dedicated thread that owns every AXUIElement
//! and is the sole caller of the Accessibility C API. The public engine sends
//! commands over a channel and blocks on a per-command reply, so AX handles
//! (which are not `Send` and have thread affinity) never leave this thread.
//!
//! Raw FFI is used (rather than a higher-level AX crate) so the whole backend
//! pins to one CoreFoundation version (0.10, shared with core-graphics 0.25)
//! and we control retain/release precisely. Attribute names are plain CFStrings
//! ("AXRole", "AXTitle", …) so no framework string constants need linking.
//!
//! The actor thread runs a CFRunLoop (polled via `recv_timeout` + a
//! non-blocking `CFRunLoopRunInMode`, so it never hot-spins) and owns an
//! AXObserver on the observed app. Change notifications flip a `dirty` flag
//! used to reject stale actions. Each `observe` walks afresh: application-level
//! subscriptions do not guarantee notification coverage for every descendant.
//! Every mutating command also marks `dirty` before another action is accepted.

// This whole module is FFI against the Accessibility C API; every helper is an
// `unsafe fn` that is only valid on the actor thread. We keep the pre-2024
// "unsafe fn body is unsafe" ergonomics rather than wrapping each FFI call.
#![allow(unsafe_op_in_unsafe_fn)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};

use core_foundation::base::TCFType;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::geometry::{CGPoint, CGSize};

use crate::engine::{
    A11yError, Effect, ElementAction, ElementEntry, ObserveOpts, Rect, Snapshot, SnapshotGen,
    Source, Target,
};
use crate::tree::{format_entries, normalize_role};

// ---- FFI ---------------------------------------------------------------

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRetain(cf: *const c_void) -> *const c_void;
    fn CFRelease(cf: *const c_void);
    fn CFGetTypeID(cf: *const c_void) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(b: *const c_void) -> u8;
    fn CFArrayGetTypeID() -> usize;
    fn CFEqual(left: *const c_void, right: *const c_void) -> u8;
    fn CFArrayGetCount(arr: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(arr: *const c_void, idx: isize) -> *const c_void;
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopRunInMode(mode: CFStringRef, seconds: f64, return_after_source_handled: u8) -> i32;
    fn CFRunLoopAddSource(rl: *mut c_void, source: *const c_void, mode: CFStringRef);
    fn CFRunLoopRemoveSource(rl: *mut c_void, source: *const c_void, mode: CFStringRef);
}

/// Runs only on the actor run loop and marks the retained snapshot stale.
unsafe extern "C" fn observer_callback(
    _observer: *mut c_void,
    _element: *const c_void,
    _notification: CFStringRef,
    refcon: *mut c_void,
) {
    if !refcon.is_null() {
        (*(refcon as *const AtomicBool)).store(true, Ordering::Relaxed);
    }
}

type AXObserverCallback = unsafe extern "C" fn(*mut c_void, *const c_void, CFStringRef, *mut c_void);

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementGetTypeID() -> usize;
    fn AXUIElementCreateSystemWide() -> *const c_void;
    fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
    fn AXUIElementCopyAttributeValue(
        el: *const c_void,
        attr: CFStringRef,
        out: *mut *const c_void,
    ) -> i32;
    fn AXUIElementSetAttributeValue(el: *const c_void, attr: CFStringRef, val: *const c_void)
        -> i32;
    fn AXUIElementIsAttributeSettable(
        el: *const c_void,
        attr: CFStringRef,
        settable: *mut u8,
    ) -> i32;
    fn AXUIElementCopyActionNames(el: *const c_void, out: *mut *const c_void) -> i32;
    fn AXUIElementPerformAction(el: *const c_void, action: CFStringRef) -> i32;
    fn AXUIElementGetPid(el: *const c_void, out: *mut i32) -> i32;
    fn AXUIElementCopyElementAtPosition(
        application: *const c_void,
        x: f32,
        y: f32,
        out: *mut *const c_void,
    ) -> i32;
    fn AXValueGetValue(value: *const c_void, the_type: u32, out: *mut c_void) -> u8;
    fn AXValueGetTypeID() -> usize;
    fn AXUIElementSetMessagingTimeout(el: *const c_void, seconds: f32) -> i32;
    fn AXObserverCreate(
        application: i32,
        callback: AXObserverCallback,
        out: *mut *mut c_void,
    ) -> i32;
    fn AXObserverAddNotification(
        observer: *mut c_void,
        element: *const c_void,
        notification: CFStringRef,
        refcon: *mut c_void,
    ) -> i32;
    fn AXObserverRemoveNotification(
        observer: *mut c_void,
        element: *const c_void,
        notification: CFStringRef,
    ) -> i32;
    fn AXObserverGetRunLoopSource(observer: *mut c_void) -> *const c_void;
}

const AX_VALUE_CGPOINT: u32 = 1;
const AX_VALUE_CGSIZE: u32 = 2;
/// Bound provider traversal as well as the number of emitted elements. A large
/// tree of unnamed/non-actionable nodes must not escape the observe budget.
const MAX_VISITED_NODES: usize = 4096;
const WALK_TIME_BUDGET: Duration = Duration::from_secs(3);

// ---- AxElem: RAII owner of one AXUIElement (thread-confined, !Send) -----

struct AxElem(*const c_void);

impl AxElem {
    /// Take ownership of a +1 reference (from a Create/Copy call).
    unsafe fn from_create(p: *const c_void) -> Option<Self> {
        if p.is_null() {
            None
        } else {
            Some(AxElem(p))
        }
    }
    /// Retain a borrowed (+0) reference and own the new count.
    unsafe fn from_borrowed(p: *const c_void) -> Option<Self> {
        if p.is_null() {
            None
        } else {
            Some(AxElem(CFRetain(p)))
        }
    }
    fn ptr(&self) -> *const c_void {
        self.0
    }
    fn retain(&self) -> AxElem {
        unsafe { AxElem(CFRetain(self.0)) }
    }
}

impl Drop for AxElem {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

// ---- low-level attribute helpers (call only on the actor thread) --------

unsafe fn copy_attr_raw(el: *const c_void, name: &str) -> *const c_void {
    copy_attr_result(el, name).unwrap_or(std::ptr::null())
}

unsafe fn copy_attr_result(el: *const c_void, name: &str) -> Result<*const c_void, A11yError> {
    let attr = CFString::new(name);
    let mut out: *const c_void = std::ptr::null();
    let err = AXUIElementCopyAttributeValue(el, attr.as_concrete_TypeRef(), &mut out);
    if err != 0 {
        Err(ax_error(&format!("read {name}"), err))
    } else {
        Ok(out)
    }
}

unsafe fn copy_str_attr(el: *const c_void, name: &str) -> Option<String> {
    let out = copy_attr_raw(el, name);
    if out.is_null() {
        return None;
    }
    if CFGetTypeID(out) == CFStringGetTypeID() {
        // Take the +1 directly as a CFString and let it release on drop.
        Some(CFString::wrap_under_create_rule(out as CFStringRef).to_string())
    } else {
        CFRelease(out);
        None
    }
}

unsafe fn copy_optional_str_attr(
    el: *const c_void,
    name: &str,
) -> Result<Option<String>, A11yError> {
    let attr = CFString::new(name);
    let mut out = std::ptr::null();
    let err = AXUIElementCopyAttributeValue(el, attr.as_concrete_TypeRef(), &mut out);
    if err == -25205 || err == -25212 {
        return Ok(None);
    }
    if err != 0 {
        return Err(ax_error(&format!("read {name}"), err));
    }
    if out.is_null() {
        return Ok(None);
    }
    if CFGetTypeID(out) != CFStringGetTypeID() {
        CFRelease(out);
        return Err(A11yError::Backend(format!("{name} returned a non-string value")));
    }
    Ok(Some(CFString::wrap_under_create_rule(out as CFStringRef).to_string()))
}

unsafe fn scalar_value_string(out: *const c_void) -> Option<String> {
    if out.is_null() {
        return None;
    }
    let type_id = CFGetTypeID(out);
    let result = if type_id == CFStringGetTypeID() {
        let text = CFString::wrap_under_get_rule(out as CFStringRef).to_string();
        Some(text.chars().take(2000).collect::<String>())
    } else if type_id == CFBooleanGetTypeID() {
        Some(if CFBooleanGetValue(out) == 0 { "0" } else { "1" }.to_string())
    } else if type_id == CFNumber::type_id() {
        CFNumber::wrap_under_get_rule(out as _).to_f64()
            .filter(|value| value.is_finite()).map(|value| value.to_string())
    } else {
        None
    };
    result.filter(|value| !value.trim().is_empty())
}

unsafe fn copy_scalar_attr(el: *const c_void, name: &str) -> Option<String> {
    let out = copy_attr_raw(el, name);
    let result = scalar_value_string(out);
    if !out.is_null() {
        CFRelease(out);
    }
    result
}

unsafe fn copy_bool_attr(el: *const c_void, name: &str) -> Option<bool> {
    let out = copy_attr_raw(el, name);
    if out.is_null() {
        return None;
    }
    let r = if CFGetTypeID(out) == CFBooleanGetTypeID() {
        Some(CFBooleanGetValue(out) != 0)
    } else {
        None
    };
    CFRelease(out);
    r
}

unsafe fn copy_elem_attr(el: *const c_void, name: &str) -> Option<AxElem> {
    let out = copy_attr_raw(el, name);
    if out.is_null() {
        None
    } else if CFGetTypeID(out) == AXUIElementGetTypeID() {
        AxElem::from_create(out)
    } else {
        CFRelease(out);
        None
    }
}

unsafe fn copy_children(el: *const c_void) -> (Vec<AxElem>, bool) {
    copy_elem_array_attr_bounded(el, "AXChildren")
}

unsafe fn copy_elem_array_attr(el: *const c_void, name: &str) -> Vec<AxElem> {
    copy_elem_array_attr_bounded(el, name).0
}

unsafe fn copy_elem_array_attr_bounded(el: *const c_void, name: &str) -> (Vec<AxElem>, bool) {
    let out = copy_attr_raw(el, name);
    if out.is_null() {
        return (Vec::new(), false);
    }
    if CFGetTypeID(out) != CFArrayGetTypeID() {
        CFRelease(out);
        return (Vec::new(), true);
    }
    let n = CFArrayGetCount(out);
    let mut v = Vec::with_capacity((n.max(0) as usize).min(MAX_VISITED_NODES));
    for i in 0..n.min(MAX_VISITED_NODES as isize) {
        let item = CFArrayGetValueAtIndex(out, i);
        if !item.is_null() && CFGetTypeID(item) == AXUIElementGetTypeID() {
            if let Some(e) = AxElem::from_borrowed(item) {
                v.push(e);
            }
        }
    }
    CFRelease(out);
    (v, n > MAX_VISITED_NODES as isize)
}

unsafe fn copy_windows(app: &AxElem) -> Result<Vec<AxElem>, A11yError> {
    let out = copy_attr_result(app.ptr(), "AXWindows")?;
    if out.is_null() {
        return Err(A11yError::NotFound("application has no readable windows".to_string()));
    }
    if CFGetTypeID(out) != CFArrayGetTypeID() {
        CFRelease(out);
        return Err(A11yError::Backend("AXWindows returned a non-array value".to_string()));
    }
    let count = CFArrayGetCount(out);
    if count < 0 || count as usize > MAX_VISITED_NODES {
        CFRelease(out);
        return Err(A11yError::Backend("AXWindows exceeds the safe enumeration limit".to_string()));
    }
    let mut windows = Vec::with_capacity(count as usize);
    for index in 0..count {
        let item = CFArrayGetValueAtIndex(out, index);
        if item.is_null() || CFGetTypeID(item) != AXUIElementGetTypeID() {
            CFRelease(out);
            return Err(A11yError::Backend("AXWindows contains a non-element value".to_string()));
        }
        if let Some(window) = AxElem::from_borrowed(item) {
            windows.push(window);
        }
    }
    CFRelease(out);
    Ok(windows)
}

unsafe fn copy_point(el: *const c_void, name: &str) -> Option<(f64, f64)> {
    let out = copy_attr_raw(el, name);
    if out.is_null() {
        return None;
    }
    if CFGetTypeID(out) != AXValueGetTypeID() {
        CFRelease(out);
        return None;
    }
    let mut p = CGPoint { x: 0.0, y: 0.0 };
    let ok = AXValueGetValue(out, AX_VALUE_CGPOINT, &mut p as *mut _ as *mut c_void);
    CFRelease(out);
    if ok != 0 {
        Some((p.x, p.y))
    } else {
        None
    }
}

unsafe fn copy_size(el: *const c_void, name: &str) -> Option<(f64, f64)> {
    let out = copy_attr_raw(el, name);
    if out.is_null() {
        return None;
    }
    if CFGetTypeID(out) != AXValueGetTypeID() {
        CFRelease(out);
        return None;
    }
    let mut s = CGSize {
        width: 0.0,
        height: 0.0,
    };
    let ok = AXValueGetValue(out, AX_VALUE_CGSIZE, &mut s as *mut _ as *mut c_void);
    CFRelease(out);
    if ok != 0 {
        Some((s.width, s.height))
    } else {
        None
    }
}

unsafe fn copy_actions(el: *const c_void) -> Result<Vec<String>, A11yError> {
    let mut out: *const c_void = std::ptr::null();
    let err = AXUIElementCopyActionNames(el, &mut out);
    decode_action_names_reply(err, out)
}

/// Decode a CopyActionNames reply independently of the AX transport. `out`
/// follows the copy rule and is consumed here, including failed replies.
unsafe fn decode_action_names_reply(
    err: i32,
    out: *const c_void,
) -> Result<Vec<String>, A11yError> {
    let decoded = (|| {
        if err != 0 {
            return Err(A11yError::Backend(format!(
                "read AX action names failed (AXError {err}); action metadata is unavailable"
            )));
        }
        if out.is_null() {
            return Err(A11yError::Backend(
                "read AX action names succeeded without returning an array".to_string(),
            ));
        }
        if CFGetTypeID(out) != CFArrayGetTypeID() {
            return Err(A11yError::Backend(
                "read AX action names returned a value that is not an array".to_string(),
            ));
        }
        let n = CFArrayGetCount(out);
        if n < 0 || n as usize > MAX_VISITED_NODES {
            return Err(A11yError::Backend(
                "read AX action names exceeded the complete-list budget".to_string(),
            ));
        }
        let mut v = Vec::with_capacity(n as usize);
        for i in 0..n {
            let item = CFArrayGetValueAtIndex(out, i);
            if item.is_null() || CFGetTypeID(item) != CFStringGetTypeID() {
                return Err(A11yError::Backend(format!(
                    "read AX action names returned a non-string entry at index {i}"
                )));
            }
            v.push(CFString::wrap_under_get_rule(item as CFStringRef).to_string());
        }
        Ok(v)
    })();
    // Every non-null copy-rule output is released exactly once, even when its
    // status or payload fails validation. Unknown lists never prove absence.
    if !out.is_null() {
        CFRelease(out);
    }
    decoded
}

unsafe fn pid_of(el: *const c_void) -> Option<i32> {
    let mut p = 0i32;
    if AXUIElementGetPid(el, &mut p) == 0 {
        Some(p)
    } else {
        None
    }
}

unsafe fn perform(el: *const c_void, action: &str) -> i32 {
    let a = CFString::new(action);
    AXUIElementPerformAction(el, a.as_concrete_TypeRef())
}

unsafe fn set_string_value(el: *const c_void, val: &str) -> i32 {
    let attr = CFString::new("AXValue");
    let v = CFString::new(val);
    AXUIElementSetAttributeValue(
        el,
        attr.as_concrete_TypeRef(),
        v.as_concrete_TypeRef() as *const c_void,
    )
}

fn is_action_actionable(actions: &[String]) -> bool {
    actions.iter().any(|a| {
        matches!(
            a.as_str(),
            "AXPress" | "AXConfirm" | "AXOpen" | "AXShowMenu" | "AXPick" | "AXIncrement"
                | "AXDecrement"
        )
    })
}

fn is_editable_role(role: Option<&str>) -> bool {
    matches!(role, Some("AXTextField" | "AXTextArea" | "AXComboBox"))
}

fn is_secure_text(role: Option<&str>, subrole: Option<&str>) -> bool {
    role == Some("AXSecureTextField") || subrole == Some("AXSecureTextField")
}

unsafe fn copy_name(el: *const c_void) -> Option<String> {
    copy_str_attr(el, "AXTitle")
        .filter(|s| !s.trim().is_empty())
        .or_else(|| copy_str_attr(el, "AXDescription").filter(|s| !s.trim().is_empty()))
}

unsafe fn copy_observable_value(el: *const c_void, role: Option<&str>) -> (Option<String>, bool) {
    let subrole = copy_str_attr(el, "AXSubrole");
    let secure = is_secure_text(role, subrole.as_deref());
    // Do not even request AXValue from secure controls. Providers should hide
    // it themselves, but the engine must not serialize a misbehaving value.
    let value = if secure {
        None
    } else {
        copy_scalar_attr(el, "AXValue")
    };
    (value, secure)
}

// ---- collection: walk the focused window into numbered entries ----------

struct Collected {
    elem: AxElem,
    role: String,
    name: Option<String>,
    value: Option<String>,
    states: Vec<String>,
    bounds: Rect,
}

unsafe fn walk(
    el: &AxElem,
    depth: usize,
    opts: &ObserveOpts,
    out: &mut Vec<Collected>,
    truncated: &mut bool,
    visited: &mut usize,
    started: Instant,
) {
    if out.len() >= opts.node_budget
        || *visited >= MAX_VISITED_NODES
        || started.elapsed() >= WALK_TIME_BUDGET
    {
        *truncated = true;
        return;
    }
    *visited += 1;
    // Each remote property request is bounded even if the app stops responding.
    // This applies to this element only, not a process-wide system element.
    if AXUIElementSetMessagingTimeout(el.ptr(), 0.25) != 0 {
        *truncated = true;
        return;
    }
    let role = copy_str_attr(el.ptr(), "AXRole");
    let name = copy_name(el.ptr());
    let (value, secure) = copy_observable_value(el.ptr(), role.as_deref());
    let pos = copy_point(el.ptr(), "AXPosition");
    let size = copy_size(el.ptr(), "AXSize");
    let actions = copy_actions(el.ptr()).unwrap_or_default();
    let actionable = is_action_actionable(&actions);
    let enabled = copy_bool_attr(el.ptr(), "AXEnabled").unwrap_or(true);

    if let (Some((x, y)), Some((w, h))) = (pos, size) {
        let bounds = Rect { x, y, w, h };
        if valid_window_bounds(&bounds)
            && (actionable || is_editable_role(role.as_deref()) || secure || name.is_some())
        {
            let mut states = Vec::new();
            if !enabled {
                states.push("disabled".to_string());
            }
            if copy_bool_attr(el.ptr(), "AXFocused").unwrap_or(false) {
                states.push("focused".to_string());
            }
            if secure {
                states.push("password".to_string());
            }
            if matches!(role.as_deref(), Some("AXCheckBox" | "AXRadioButton")) {
                match value.as_deref() {
                    Some("1") => states.push("checked".to_string()),
                    Some("2") => states.push("mixed".to_string()),
                    _ => {}
                }
            }
            if copy_bool_attr(el.ptr(), "AXSelected") == Some(true) {
                states.push("selected".to_string());
            }
            if let Some(expanded) = copy_bool_attr(el.ptr(), "AXExpanded") {
                states.push(if expanded { "expanded" } else { "collapsed" }.to_string());
            }
            out.push(Collected {
                elem: el.retain(),
                role: role.clone().unwrap_or_else(|| "element".to_string()),
                name,
                value,
                states,
                bounds,
            });
        }
    }

    let (children, children_truncated) = copy_children(el.ptr());
    *truncated |= children_truncated;
    if depth >= opts.max_depth {
        *truncated |= !children.is_empty();
        return;
    }
    for child in children {
        if out.len() >= opts.node_budget || *visited >= MAX_VISITED_NODES {
            *truncated = true;
            return;
        }
        walk(&child, depth + 1, opts, out, truncated, visited, started);
    }
}

// ---- actor thread state + command handling ------------------------------

/// A registered AXObserver watching one application for change notifications.
/// Dropping it removes the run-loop source and notifications before the `dirty`
/// flag it points at can be freed (see `State` field order).
struct AxObserver {
    observer: *mut c_void,
    app: AxElem,
    runloop: *mut c_void,
    notifications: Vec<CFString>,
}

impl Drop for AxObserver {
    fn drop(&mut self) {
        unsafe {
            let src = AXObserverGetRunLoopSource(self.observer);
            if !src.is_null() {
                let mode = CFString::new("kCFRunLoopDefaultMode");
                CFRunLoopRemoveSource(self.runloop, src, mode.as_concrete_TypeRef());
            }
            for n in &self.notifications {
                AXObserverRemoveNotification(self.observer, self.app.ptr(), n.as_concrete_TypeRef());
            }
            CFRelease(self.observer as *const c_void);
        }
    }
}

/// Register a change observer for `pid` on the run loop, with `refcon` pointing
/// at the `dirty` flag. Returns `None` (caller then rejects actions) on failure.
unsafe fn register_observer(
    pid: i32,
    app: &AxElem,
    runloop: *mut c_void,
    refcon: *mut c_void,
) -> Option<AxObserver> {
    let mut obs: *mut c_void = std::ptr::null_mut();
    if AXObserverCreate(pid, observer_callback, &mut obs) != 0 || obs.is_null() {
        return None;
    }
    const NOTIFS: &[&str] = &[
        "AXValueChanged",
        "AXUIElementDestroyed",
        "AXFocusedUIElementChanged",
        "AXMainWindowChanged",
        "AXFocusedWindowChanged",
        "AXWindowResized",
        "AXWindowMoved",
        "AXCreated",
        "AXLayoutChanged",
        "AXSelectedChildrenChanged",
        "AXRowCountChanged",
        "AXTitleChanged",
        "AXMenuOpened",
        "AXMenuClosed",
    ];
    let mut notifications = Vec::new();
    for name in NOTIFS {
        let cf = CFString::new(name);
        // Not every notification applies to every app element; ignore failures.
        if AXObserverAddNotification(obs, app.ptr(), cf.as_concrete_TypeRef(), refcon) == 0 {
            notifications.push(cf);
        }
    }
    if notifications.is_empty() {
        // A created observer without subscriptions cannot fence stale actions.
        CFRelease(obs as *const c_void);
        return None;
    }
    let src = AXObserverGetRunLoopSource(obs);
    if src.is_null() {
        CFRelease(obs as *const c_void);
        return None;
    }
    let mode = CFString::new("kCFRunLoopDefaultMode");
    CFRunLoopAddSource(runloop, src, mode.as_concrete_TypeRef());
    Some(AxObserver {
        observer: obs,
        app: app.retain(),
        runloop,
        notifications,
    })
}

/// Metadata from the last fresh walk, used to verify retained handles before
/// an action. It is never reused as the answer to another observe request.
struct CachedWalk {
    opts: ObserveOpts,
    entries: Vec<ElementEntry>,
}

struct State {
    gen_counter: u64,
    current_gen: SnapshotGen,
    registry: HashMap<u32, AxElem>,
    /// This thread's run loop (observer sources are attached to it).
    runloop: *mut c_void,
    /// MUST be declared before `dirty`: dropping the observer removes its
    /// callback source before the `dirty` flag it references is freed.
    observer: Option<AxObserver>,
    /// Boxed for a stable address (the observer's `refcon`). Set true by the
    /// observer callback and by every mutating command; cleared on a fresh walk.
    dirty: Box<AtomicBool>,
    observed_pid: Option<i32>,
    observed_window: Option<AxElem>,
    cached: Option<CachedWalk>,
}

unsafe fn focused_app() -> Result<AxElem, A11yError> {
    let sw = AxElem::from_create(AXUIElementCreateSystemWide()).ok_or_else(|| {
        A11yError::Backend("AXUIElementCreateSystemWide returned null".to_string())
    })?;
    let app = copy_elem_attr(sw.ptr(), "AXFocusedApplication").ok_or_else(|| {
        let app = crate::host_app_label();
        A11yError::Permission(format!(
            "No focused application is readable — Accessibility permission is not in effect for \
             {app}. Grant it in System Settings → Privacy & Security → Accessibility (the entry is \
             named \"{app}\"), then COMPLETELY quit and reopen {app} — macOS does not apply this \
             permission to an already-running process. Computer-use runs inside {app} itself, so \
             do not grant a terminal or editor."
        ))
    })?;
    let err = AXUIElementSetMessagingTimeout(app.ptr(), 0.5);
    if err != 0 {
        return Err(ax_error("configure focused application messaging timeout", err));
    }
    Ok(app)
}

unsafe fn focused_window(app: &AxElem) -> Option<AxElem> {
    copy_elem_attr(app.ptr(), "AXFocusedWindow")
        .or_else(|| copy_elem_attr(app.ptr(), "AXMainWindow"))
        .or_else(|| copy_elem_array_attr(app.ptr(), "AXWindows").into_iter().next())
}

fn do_observe(opts: &ObserveOpts, state: &mut State) -> Result<Snapshot, A11yError> {
    require_accessibility_permission()?;
    unsafe {
        let app = match opts.pid {
            Some(pid) => application_for_pid(pid)?,
            None => focused_app()?,
        };
        let app_pid = pid_of(app.ptr());

        // Register before reading to capture changes that occur during the
        // walk. Failed subscriptions are retried on the next fresh observation.
        if state.observed_pid != app_pid || state.observer.is_none() {
            state.observer = None;
            if let Some(p) = app_pid {
                let refcon = (&*state.dirty as *const AtomicBool) as *mut c_void;
                state.observer = register_observer(p, &app, state.runloop, refcon);
            }
            state.observed_pid = app_pid;
        }
        state.dirty.store(false, Ordering::Relaxed);

        let app_name = copy_str_attr(app.ptr(), "AXTitle");
        let window = focused_window(&app).ok_or_else(|| {
            A11yError::NotFound("the focused application has no readable window".to_string())
        })?;
        let window_title = copy_str_attr(window.ptr(), "AXTitle");

        let mut collected = Vec::new();
        let mut truncated = false;
        let mut visited = 0;
        walk(&window, 0, opts, &mut collected, &mut truncated, &mut visited, Instant::now());

        // Reading order: top-to-bottom, left-to-right.
        collected.sort_by(|a, b| {
            (a.bounds.y.round() as i64, a.bounds.x.round() as i64)
                .cmp(&(b.bounds.y.round() as i64, b.bounds.x.round() as i64))
        });

        state.gen_counter += 1;
        let generation = SnapshotGen(state.gen_counter);
        state.current_gen = generation;
        state.registry.clear();

        let mut entries = Vec::with_capacity(collected.len());
        for (i, c) in collected.into_iter().enumerate() {
            let r = i as u32 + 1;
            state.registry.insert(r, c.elem);
            entries.push(ElementEntry {
                r#ref: r,
                role: normalize_role(&c.role),
                name: c.name,
                value: c.value,
                states: c.states,
                bounds: c.bounds,
                source: Source::A11y,
            });
        }

        state.observed_window = Some(window);
        let mode = CFString::new("kCFRunLoopDefaultMode");
        CFRunLoopRunInMode(mode.as_concrete_TypeRef(), 0.0, 0);

        let text = format_entries(&entries);
        state.cached = Some(CachedWalk {
            opts: opts.clone(),
            entries: entries.clone(),
        });
        Ok(Snapshot {
            generation,
            entries,
            text,
            truncated,
            pid: app_pid,
            app_name,
            window_title,
        })
    }
}

/// Validate a retained ref without performing or focusing anything. Both
/// semantic actions and pointer-target resolution share this exact fence.
fn validated_ref<'a>(
    target: &Target,
    generation: SnapshotGen,
    state: &'a State,
) -> Result<(u32, &'a AxElem, &'a ElementEntry), A11yError> {
    require_accessibility_permission()?;
    let r = match target {
        Target::Ref(r) => *r,
        Target::Pixel { .. } => {
            return Err(A11yError::Unsupported {
                capability: "pixel targeting".to_string(),
                hint: "Pixel fallback is handled by the computer tool's input layer, not the \
                       accessibility engine."
                    .to_string(),
            })
        }
    };
    let cached = state.cached.as_ref().ok_or_else(|| {
        A11yError::Stale(format!(
            "ref [{r}] has no retained snapshot; re-run observe before acting"
        ))
    })?;
    let frontmost_observation = cached.opts.pid.is_none();
    let current_frontmost_pid = if frontmost_observation {
        unsafe {
            let app = focused_app()?;
            pid_of(app.ptr())
        }
    } else {
        None
    };
    validate_invoke_snapshot(
        r,
        generation,
        state.current_gen,
        state.dirty.load(Ordering::Relaxed),
        state.observer.is_some(),
        frontmost_observation,
        state.observed_pid,
        current_frontmost_pid,
    )?;
    let elem = state
        .registry
        .get(&r)
        .ok_or_else(|| A11yError::NotFound(format!("no element [{r}] in the latest snapshot")))?;

    unsafe {
        let entry = cached.entries.iter().find(|entry| entry.r#ref == r)
            .ok_or_else(|| A11yError::Stale(format!("ref [{r}] has no retained metadata")))?;
        let role = copy_str_attr(elem.ptr(), "AXRole");
        let name = copy_name(elem.ptr());
        let (value, _) = copy_observable_value(elem.ptr(), role.as_deref());
        let bounds = copy_point(elem.ptr(), "AXPosition").zip(copy_size(elem.ptr(), "AXSize"))
            .map(|((x, y), (w, h))| Rect { x, y, w, h });
        let app = application_for_pid(state.observed_pid.ok_or_else(|| {
            A11yError::Stale(format!("ref [{r}] has no owning application"))
        })?)?;
        let current_window = focused_window(&app);
        let same_window = current_window.as_ref().zip(state.observed_window.as_ref())
            .is_some_and(|(current, observed)| CFEqual(current.ptr(), observed.ptr()) != 0);
        if !same_window || !live_entry_matches(
            role.as_deref(), name.as_deref(), value.as_deref(), bounds.as_ref(), entry,
        ) {
            return Err(A11yError::Stale(format!(
                "ref [{r}] no longer matches its observed window, role, content or bounds; re-run observe"
            )));
        }
        Ok((r, elem, entry))
    }
}

fn do_resolve_pixel_target(
    target: &Target,
    generation: SnapshotGen,
    state: &State,
) -> Result<Rect, A11yError> {
    let (r, element, entry) = validated_ref(target, generation, state)?;
    let (x, y) = entry.bounds.pixel_center().ok_or_else(|| {
        A11yError::Stale(format!("ref [{r}] has invalid or unrepresentable screen bounds"))
    })?;
    unsafe {
        let system = AxElem::from_create(AXUIElementCreateSystemWide()).ok_or_else(|| {
            A11yError::Backend("AXUIElementCreateSystemWide returned null".to_string())
        })?;
        let mut out = std::ptr::null();
        let err = AXUIElementCopyElementAtPosition(system.ptr(), x as f32, y as f32, &mut out);
        if err != 0 {
            return Err(ax_error("read the gesture point's topmost Accessibility element", err));
        }
        let mut hit = AxElem::from_create(out).ok_or_else(|| {
            A11yError::Stale(format!("ref [{r}] has no Accessibility element at its center"))
        })?;
        // Public AX hit testing follows window z-order. A child of the target
        // is acceptable; a covering sibling window or unrelated element is not.
        for _ in 0..64 {
            if CFEqual(hit.ptr(), element.ptr()) != 0 {
                // Flush changes queued during hit testing, then repeat the
                // shared live fence before handing the point to input. This
                // still cannot atomically lock the external OS scene.
                let mode = CFString::new("kCFRunLoopDefaultMode");
                CFRunLoopRunInMode(mode.as_concrete_TypeRef(), 0.0, 0);
                let (_, _, confirmed) = validated_ref(target, generation, state)?;
                return Ok(confirmed.bounds);
            }
            if AXUIElementSetMessagingTimeout(hit.ptr(), 0.25) != 0 {
                break;
            }
            let Some(parent) = copy_elem_attr(hit.ptr(), "AXParent") else { break; };
            if CFEqual(parent.ptr(), hit.ptr()) != 0 {
                break;
            }
            hit = parent;
        }
    }
    Err(A11yError::Stale(format!(
        "ref [{r}] is covered or no longer owns the gesture point; re-run observe"
    )))
}

fn invocation_may_have_effect(result: &Result<Effect, A11yError>) -> bool {
    matches!(result, Ok(_) | Err(A11yError::Uncertain(_)))
}

fn receive_mutating_reply(receiver: std::sync::mpsc::Receiver<Result<Effect, A11yError>>) -> Result<Effect, A11yError> {
    receiver.recv().map_err(|_| A11yError::Uncertain(
        "the AX actor accepted the mutating request but its reply was lost; the effect may have occurred. Re-run observe and do not retry automatically".into(),
    ))?
}

fn do_invoke(
    target: &Target,
    generation: SnapshotGen,
    action: &ElementAction,
    state: &State,
) -> Result<Effect, A11yError> {
    let (r, elem, _) = validated_ref(target, generation, state)?;
    unsafe {
        let err = match action {
            ElementAction::Press | ElementAction::LeftClick => {
                invoke_semantic_press(action, copy_actions(elem.ptr()), |name| {
                    perform(elem.ptr(), name)
                })?
            }
            ElementAction::DoubleClick => return Err(A11yError::Unsupported {
                capability: "semantic double click".to_string(),
                hint: "Accessibility press is a single action; use the computer input layer for double clicks.".to_string(),
            }),
            ElementAction::RightClick => perform(elem.ptr(), "AXShowMenu"),
            ElementAction::Focus => {
                let attr = CFString::new("AXFocused");
                let t = core_foundation::boolean::CFBoolean::true_value();
                AXUIElementSetAttributeValue(
                    elem.ptr(),
                    attr.as_concrete_TypeRef(),
                    t.as_concrete_TypeRef() as *const c_void,
                )
            }
            ElementAction::SetValue(v) => {
                require_writable_attribute(elem, "AXValue")?;
                set_string_value(elem.ptr(), v)
            }
        };
        if err == 0 {
            Ok(Effect {
                changed: true,
                message: format!("performed {} on element [{r}]", action_label(action)),
            })
        } else {
            Err(ax_mutation_error(&format!("AX action on [{r}]"), err))
        }
    }
}

fn invoke_semantic_press(
    action: &ElementAction,
    available: Result<Vec<String>, A11yError>,
    mut perform_action: impl FnMut(&str) -> i32,
) -> Result<i32, A11yError> {
    let available = available?;
    let name = semantic_press_action(action, &available).ok_or_else(|| {
        A11yError::Unsupported {
            capability: "semantic press".to_string(),
            hint: "This element exposes no press action; use the computer input layer.".to_string(),
        }
    })?;
    Ok(perform_action(name))
}

fn semantic_press_action(action: &ElementAction, available: &[String]) -> Option<&'static str> {
    let candidates: &[&str] = match action {
        // A left click must never submit a field or open/pick a control merely
        // because it exposes a different semantic action. The input layer owns
        // the pixel fallback when a true AXPress is unavailable.
        ElementAction::LeftClick => &["AXPress"],
        ElementAction::Press => &["AXPress", "AXConfirm", "AXOpen", "AXPick"],
        _ => return None,
    };
    candidates.iter().copied().find(|candidate| {
        available.iter().any(|name| name.as_str() == *candidate)
    })
}

fn live_entry_matches(
    role: Option<&str>,
    name: Option<&str>,
    value: Option<&str>,
    bounds: Option<&Rect>,
    entry: &ElementEntry,
) -> bool {
    role.is_some_and(|role| normalize_role(role) == entry.role)
        && name == entry.name.as_deref()
        && value == entry.value.as_deref()
        && bounds == Some(&entry.bounds)
}

fn action_label(action: &ElementAction) -> &'static str {
    match action {
        ElementAction::Press => "press",
        ElementAction::LeftClick => "left click",
        ElementAction::RightClick => "right click",
        ElementAction::DoubleClick => "double click",
        ElementAction::Focus => "focus",
        ElementAction::SetValue(_) => "set value",
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_invoke_snapshot(
    r: u32,
    generation: SnapshotGen,
    current_generation: SnapshotGen,
    dirty: bool,
    observer_active: bool,
    frontmost_observation: bool,
    observed_pid: Option<i32>,
    current_frontmost_pid: Option<i32>,
) -> Result<(), A11yError> {
    if generation != current_generation {
        return Err(A11yError::Stale(format!(
            "ref [{r}] is from an older snapshot (the UI may have changed); re-run observe and \
             use a fresh [ref]"
        )));
    }
    if dirty {
        return Err(A11yError::Stale(format!(
            "ref [{r}] was invalidated by an Accessibility window, focus, layout or value change; \
             re-run observe before acting"
        )));
    }
    if !observer_active {
        return Err(A11yError::Stale(format!(
            "ref [{r}] has no live Accessibility change observer; re-run observe and do not act \
             on an unfenced snapshot"
        )));
    }
    if frontmost_observation && current_frontmost_pid != observed_pid {
        return Err(A11yError::Stale(format!(
            "ref [{r}] belongs to pid {:?}, but the frontmost application is now pid {:?}; \
             re-run observe before acting",
            observed_pid, current_frontmost_pid
        )));
    }
    Ok(())
}

fn do_focus(pid: i32) -> Result<Effect, A11yError> {
    unsafe {
        let app = application_for_pid(pid)?;
        let window = focused_window(&app).ok_or_else(|| {
            A11yError::NotFound(format!("pid {pid} has no readable window"))
        })?;
        activate_window(&app, &window, pid)
    }
}

fn require_accessibility_permission() -> Result<(), A11yError> {
    if unsafe { AXIsProcessTrusted() } == 0 {
        return Err(A11yError::Permission(format!(
            "Accessibility permission is not in effect for {}; grant it in System Settings → \
             Privacy & Security → Accessibility, then quit and reopen the app",
            crate::host_app_label()
        )));
    }
    Ok(())
}

fn ax_error(operation: &str, err: i32) -> A11yError {
    if err == -25211 {
        A11yError::Permission(format!("{operation}: Accessibility API is disabled"))
    } else if err == -25202 {
        A11yError::Stale(format!("{operation}: Accessibility element is no longer valid"))
    } else {
        A11yError::Backend(format!("{operation} failed (AXError {err})"))
    }
}

fn ax_mutation_error(operation: &str, err: i32) -> A11yError {
    // An AXError returned after calling a mutator does not prove the provider
    // applied no effect. Even a lost/destroyed element or a permission failure
    // can be observed after submission. Unsupported is established separately
    // by read-only action/writability checks before reaching this point.
    A11yError::Uncertain(format!(
        "{operation}: the mutating request was admitted to the native AX API, but its reply \
         failed (AXError {err}); the effect is unknown. Re-run observe before deciding another \
         action; no automatic retry is safe"
    ))
}

fn writable_attribute_result(name: &str, err: i32, settable: u8) -> Result<(), A11yError> {
    if (err == 0 && settable == 0) || matches!(err, -25205 | -25208) {
        return Err(A11yError::Unsupported {
            capability: format!("set attribute {name}"),
            hint: "The attribute is not writable; no mutating request was admitted.".to_string(),
        });
    }
    if err != 0 {
        return Err(ax_error(&format!("read writability of {name}"), err));
    }
    Ok(())
}

unsafe fn require_writable_attribute(element: &AxElem, name: &str) -> Result<(), A11yError> {
    let attribute = CFString::new(name);
    let mut settable = 0;
    let err = AXUIElementIsAttributeSettable(element.ptr(), attribute.as_concrete_TypeRef(), &mut settable);
    writable_attribute_result(name, err, settable)
}

unsafe fn application_for_pid(pid: i32) -> Result<AxElem, A11yError> {
    require_accessibility_permission()?;
    if pid <= 0 {
        return Err(A11yError::NotFound(format!("invalid application pid {pid}")));
    }
    let app = AxElem::from_create(AXUIElementCreateApplication(pid))
        .ok_or_else(|| A11yError::NotFound(format!("no app for pid {pid}")))?;
    let err = AXUIElementSetMessagingTimeout(app.ptr(), 0.5);
    if err != 0 {
        return Err(ax_error("configure application messaging timeout", err));
    }
    Ok(app)
}

unsafe fn set_bool_attr(el: &AxElem, name: &str, value: bool) -> Result<(), A11yError> {
    let attr = CFString::new(name);
    let value = if value {
        core_foundation::boolean::CFBoolean::true_value()
    } else {
        core_foundation::boolean::CFBoolean::false_value()
    };
    let err = AXUIElementSetAttributeValue(
        el.ptr(),
        attr.as_concrete_TypeRef(),
        value.as_concrete_TypeRef() as *const c_void,
    );
    if err == 0 { Ok(()) } else { Err(ax_mutation_error(name, err)) }
}

unsafe fn set_bool_if_settable(el: &AxElem, name: &str) -> Result<(), A11yError> {
    let attr = CFString::new(name);
    let mut settable = 0;
    let err = AXUIElementIsAttributeSettable(el.ptr(), attr.as_concrete_TypeRef(), &mut settable);
    if err == -25205 || (err == 0 && settable == 0) {
        return Ok(());
    }
    if err != 0 {
        return Err(ax_error(&format!("read writability of {name}"), err));
    }
    set_bool_attr(el, name, true)
}

unsafe fn activate_window(app: &AxElem, window: &AxElem, pid: i32) -> Result<Effect, A11yError> {
    let err = AXUIElementSetMessagingTimeout(window.ptr(), 0.5);
    if err != 0 {
        return Err(ax_error("configure window messaging timeout", err));
    }
    if copy_bool_attr(window.ptr(), "AXMinimized") == Some(true) {
        set_bool_attr(window, "AXMinimized", false)?;
    }
    let err = perform(window.ptr(), "AXRaise");
    if err != 0 {
        return Err(ax_mutation_error("raise target window", err));
    }
    set_bool_if_settable(window, "AXMain")?;
    set_bool_if_settable(window, "AXFocused")?;
    set_bool_attr(app, "AXFrontmost", true)?;

    // Activation may complete asynchronously. Success requires the actual key
    // window to equal the selected AX object; AXMain alone is not keyboard focus.
    let started = Instant::now();
    loop {
        let focused = copy_elem_attr(app.ptr(), "AXFocusedWindow");
        if copy_bool_attr(app.ptr(), "AXFrontmost") == Some(true)
            && focused.as_ref().is_some_and(|w| CFEqual(w.ptr(), window.ptr()) != 0)
            && copy_bool_attr(window.ptr(), "AXMinimized") != Some(true)
            && focused_app().ok().and_then(|a| pid_of(a.ptr())) == Some(pid)
        {
            return Ok(Effect {
                changed: true,
                message: format!("activated the selected window of pid {pid}"),
            });
        }
        if started.elapsed() >= Duration::from_millis(300) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(A11yError::Backend(format!(
        "pid {pid} did not give the selected window keyboard focus after activation"
    )))
}

fn valid_window_bounds(bounds: &Rect) -> bool {
    [bounds.x, bounds.y, bounds.w, bounds.h].iter().all(|v| v.is_finite())
        && !bounds.is_empty()
}

fn matches_window_metadata(
    bounds: &Rect,
    title: Option<&str>,
    expected_bounds: &Rect,
    expected_title: Option<&str>,
) -> bool {
    valid_window_bounds(bounds)
        && valid_window_bounds(expected_bounds)
        && [
            bounds.x - expected_bounds.x,
            bounds.y - expected_bounds.y,
            bounds.w - expected_bounds.w,
            bounds.h - expected_bounds.h,
        ]
        .iter()
        .all(|delta| delta.abs() <= 2.0)
        && expected_title.filter(|s| !s.is_empty()).is_none_or(|expected| title == Some(expected))
}

fn unique_window_index(matches: &[usize], pid: i32) -> Result<usize, A11yError> {
    match matches {
        [index] => Ok(*index),
        [] => Err(A11yError::NotFound(format!(
            "pid {pid} has no window matching the current title and bounds"
        ))),
        _ => Err(A11yError::Stale(format!(
            "pid {pid} has multiple windows matching the title and bounds; refusing ambiguous activation"
        ))),
    }
}

fn do_focus_named(
    pid: i32,
    expected_bounds: &Rect,
    expected_title: Option<&str>,
) -> Result<Effect, A11yError> {
    if !valid_window_bounds(expected_bounds) {
        return Err(A11yError::NotFound("target window bounds are invalid or empty".to_string()));
    }
    unsafe {
        let app = application_for_pid(pid)?;
        let windows = copy_windows(&app)?;
        let mut matches = Vec::new();
        let started = Instant::now();
        for (index, window) in windows.iter().enumerate() {
            if started.elapsed() >= WALK_TIME_BUDGET {
                return Err(A11yError::Stale("window enumeration exceeded its time budget; retry with current metadata".to_string()));
            }
            let err = AXUIElementSetMessagingTimeout(window.ptr(), 0.5);
            if err != 0 {
                return Err(ax_error("configure enumerated window messaging timeout", err));
            }
            let title = copy_optional_str_attr(window.ptr(), "AXTitle")?;
            if expected_title.filter(|title| !title.is_empty()).is_some_and(|expected| title.as_deref() != Some(expected)) {
                continue;
            }
            let (x, y) = copy_point(window.ptr(), "AXPosition").ok_or_else(|| {
                A11yError::Stale("candidate window position is no longer readable".to_string())
            })?;
            let (w, h) = copy_size(window.ptr(), "AXSize").ok_or_else(|| {
                A11yError::Stale("candidate window size is no longer readable".to_string())
            })?;
            if matches_window_metadata(
                &Rect { x, y, w, h }, title.as_deref(), expected_bounds, expected_title,
            ) {
                matches.push(index);
            }
        }
        let index = unique_window_index(&matches, pid)?;
        activate_window(&app, &windows[index], pid)
    }
}

// ---- command plumbing ---------------------------------------------------

enum Cmd {
    Observe(ObserveOpts, Sender<Result<Snapshot, A11yError>>),
    ResolvePixel(Target, SnapshotGen, Sender<Result<Rect, A11yError>>),
    Invoke(
        Target,
        SnapshotGen,
        ElementAction,
        Sender<Result<Effect, A11yError>>,
    ),
    Focus(i32, Sender<Result<Effect, A11yError>>),
    FocusNamed(i32, Rect, Option<String>, Sender<Result<Effect, A11yError>>),
}

pub struct ActorHandle {
    // mpsc::Sender is not Sync; the Mutex makes the handle Sync (calls are
    // serialized anyway — the tool marks the tool non-concurrency-safe).
    tx: Mutex<Sender<Cmd>>,
}

impl ActorHandle {
    pub fn spawn() -> Result<Self, A11yError> {
        let (tx, rx) = channel::<Cmd>();
        std::thread::Builder::new()
            .name("nomi-a11y-macos".to_string())
            .spawn(move || {
                let runloop = unsafe { CFRunLoopGetCurrent() };
                let mut state = State {
                    gen_counter: 0,
                    current_gen: SnapshotGen(0),
                    registry: HashMap::new(),
                    runloop,
                    observer: None,
                    dirty: Box::new(AtomicBool::new(true)),
                    observed_pid: None,
                    observed_window: None,
                    cached: None,
                };
                let mode = CFString::new("kCFRunLoopDefaultMode");
                loop {
                    // Block for a command. Before there are any observer sources
                    // (first observe not run yet), the run loop has nothing to
                    // wait on, so we must NOT spin on CFRunLoopRunInMode — block
                    // on the channel instead and pump callbacks non-blocking.
                    match rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(cmd) => {
                            // Flush any pending observer callbacks (→ `dirty`)
                            // before handling, so `observe` sees the freshest state.
                            unsafe {
                                CFRunLoopRunInMode(mode.as_concrete_TypeRef(), 0.0, 0);
                            }
                            match cmd {
                                Cmd::Observe(opts, reply) => {
                                    let _ = reply.send(do_observe(&opts, &mut state));
                                }
                                Cmd::ResolvePixel(target, generation, reply) => {
                                    let _ = reply.send(do_resolve_pixel_target(&target, generation, &state));
                                }
                                Cmd::Invoke(target, generation, action, reply) => {
                                    let r = do_invoke(&target, generation, &action, &state);
                                    // A mutating action invalidates the cache even
                                    // before the observer notification arrives.
                                    if invocation_may_have_effect(&r) { state.dirty.store(true, Ordering::Relaxed); }
                                    let _ = reply.send(r);
                                }
                                Cmd::Focus(pid, reply) => {
                                    let r = do_focus(pid);
                                    state.dirty.store(true, Ordering::Relaxed);
                                    let _ = reply.send(r);
                                }
                                Cmd::FocusNamed(pid, bounds, title, reply) => {
                                    let r = do_focus_named(pid, &bounds, title.as_deref());
                                    state.dirty.store(true, Ordering::Relaxed);
                                    let _ = reply.send(r);
                                }
                            }
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                            // Periodically service the observer source so pending
                            // notifications don't pile up while idle.
                            unsafe {
                                CFRunLoopRunInMode(mode.as_concrete_TypeRef(), 0.0, 0);
                            }
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
            })
            .map_err(|e| A11yError::Backend(format!("failed to start AX actor thread: {e}")))?;
        Ok(Self { tx: Mutex::new(tx) })
    }

    fn send(&self, cmd: Cmd) -> Result<(), A11yError> {
        self.tx
            .lock()
            .map_err(|_| A11yError::Backend("AX actor lock poisoned".to_string()))?
            .send(cmd)
            .map_err(|_| A11yError::Backend("AX actor thread is gone".to_string()))
    }

    pub fn observe(&self, opts: ObserveOpts) -> Result<Snapshot, A11yError> {
        let (tx, rx) = channel();
        self.send(Cmd::Observe(opts, tx))?;
        rx.recv()
            .map_err(|_| A11yError::Backend("AX actor dropped the reply".to_string()))?
    }

    pub fn resolve_pixel_target(
        &self,
        target: Target,
        generation: SnapshotGen,
    ) -> Result<Rect, A11yError> {
        let (tx, rx) = channel();
        self.send(Cmd::ResolvePixel(target, generation, tx))?;
        rx.recv()
            .map_err(|_| A11yError::Backend("AX actor dropped the reply".to_string()))?
    }

    pub fn invoke(
        &self,
        target: Target,
        generation: SnapshotGen,
        action: ElementAction,
    ) -> Result<Effect, A11yError> {
        let (tx, rx) = channel();
        self.send(Cmd::Invoke(target, generation, action, tx))?;
        receive_mutating_reply(rx)
    }

    pub fn focus_window(&self, pid: i32) -> Result<Effect, A11yError> {
        let (tx, rx) = channel();
        self.send(Cmd::Focus(pid, tx))?;
        receive_mutating_reply(rx)
    }

    pub fn focus_named_window(
        &self,
        pid: i32,
        expected_bounds: Rect,
        expected_title: Option<String>,
    ) -> Result<Effect, A11yError> {
        let (tx, rx) = channel();
        self.send(Cmd::FocusNamed(pid, expected_bounds, expected_title, tx))?;
        receive_mutating_reply(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_action_names_reply(names: &[&str]) -> Result<Vec<String>, A11yError> {
        let names: Vec<_> = names.iter().map(|name| CFString::new(name)).collect();
        let array = core_foundation::array::CFArray::from_CFTypes(&names);
        unsafe {
            // Give the decoder its own copy-rule reference. No AX transport,
            // permission query, run loop or native input is used by this seam.
            let out = CFRetain(array.as_concrete_TypeRef() as *const c_void);
            decode_action_names_reply(0, out)
        }
    }

    #[test]
    fn action_metadata_read_failure_is_not_confirmed_unsupported() {
        for code in [-25204, -25200, -25208] {
            let reply = unsafe { decode_action_names_reply(code, std::ptr::null()) };
            let mut performed = Vec::new();
            let result = invoke_semantic_press(&ElementAction::LeftClick, reply, |name| {
                performed.push(name.to_string());
                0
            });
            assert!(performed.is_empty(), "a failed read must not admit AXPress");
            assert!(matches!(&result, Err(A11yError::Backend(_))),
                "MAC43 failed SDK action-list read is unknown metadata, not confirmed \
                 absence authorizing a pixel fallback: AXError {code}, result={result:?}");
        }
    }

    #[test]
    fn action_metadata_success_without_array_is_not_confirmed_unsupported() {
        let reply = unsafe { decode_action_names_reply(0, std::ptr::null()) };
        let mut performed = Vec::new();
        let result = invoke_semantic_press(&ElementAction::LeftClick, reply, |name| {
            performed.push(name.to_string());
            0
        });
        assert!(performed.is_empty());
        assert!(matches!(&result, Err(A11yError::Backend(_))),
            "MAC43 successful SDK status without an array does not prove action absence: {result:?}");
    }

    #[test]
    fn action_metadata_wrong_cf_type_is_not_confirmed_unsupported() {
        let malformed = CFString::new("not an action-name array");
        let reply = unsafe {
            let out = CFRetain(malformed.as_concrete_TypeRef() as *const c_void);
            decode_action_names_reply(0, out)
        };
        let mut performed = Vec::new();
        let result = invoke_semantic_press(&ElementAction::LeftClick, reply, |name| {
            performed.push(name.to_string());
            0
        });
        assert!(performed.is_empty());
        assert!(matches!(&result, Err(A11yError::Backend(_))),
            "MAC43 a malformed SDK action-list value does not prove action absence: {result:?}");
    }

    #[test]
    fn action_metadata_confirmed_empty_list_rejects_before_native_press() {
        let mut performed = Vec::new();
        let result = invoke_semantic_press(
            &ElementAction::LeftClick, fake_action_names_reply(&[]), |name| {
                performed.push(name.to_string());
                0
            },
        );
        assert!(matches!(result, Err(A11yError::Unsupported { .. })));
        assert!(performed.is_empty());
    }

    #[test]
    fn action_metadata_confirmed_axpress_admits_one_matching_native_action() {
        let mut performed = Vec::new();
        let result = invoke_semantic_press(
            &ElementAction::LeftClick,
            fake_action_names_reply(&["AXConfirm", "AXPress", "AXShowMenu"]), |name| {
                performed.push(name.to_string());
                0
            },
        );
        assert_eq!(result.unwrap(), 0);
        assert_eq!(performed, vec!["AXPress"]);
    }

    #[test]
    fn action_metadata_malformed_array_never_admits_a_partial_action_list() {
        let press = CFString::new("AXPress");
        let non_string = CFNumber::from(1_i32);
        let arrays = [
            core_foundation::array::CFArray::from_copyable(&[
                press.as_CFTypeRef(), std::ptr::null(),
            ]),
            core_foundation::array::CFArray::from_copyable(&[
                press.as_CFTypeRef(), non_string.as_CFTypeRef(),
            ]),
        ];
        for array in arrays {
            let baseline = array.retain_count();
            let reply = unsafe {
                decode_action_names_reply(0, CFRetain(array.as_concrete_TypeRef() as *const c_void))
            };
            assert_eq!(array.retain_count(), baseline);
            let mut performed = Vec::new();
            let result = invoke_semantic_press(&ElementAction::LeftClick, reply, |name| {
                performed.push(name.to_string());
                0
            });
            assert!(matches!(&result, Err(A11yError::Backend(_))), "{result:?}");
            assert!(performed.is_empty(), "a partial list containing AXPress is not a complete capability proof");
        }
    }

    #[test]
    fn action_metadata_over_budget_list_never_proves_presence_or_absence() {
        for press_index in [0, MAX_VISITED_NODES] {
            let mut names = vec!["AXConfirm"; MAX_VISITED_NODES + 1];
            names[press_index] = "AXPress";
            let mut performed = Vec::new();
            let result = invoke_semantic_press(
                &ElementAction::LeftClick, fake_action_names_reply(&names), |name| {
                    performed.push(name.to_string());
                    0
                },
            );
            assert!(matches!(&result, Err(A11yError::Backend(_))), "{result:?}");
            assert!(performed.is_empty());
        }
    }

    #[test]
    fn action_metadata_exact_budget_remains_a_complete_list() {
        let mut names = vec!["AXConfirm"; MAX_VISITED_NODES];
        names[MAX_VISITED_NODES - 1] = "AXPress";
        let mut performed = Vec::new();
        let result = invoke_semantic_press(
            &ElementAction::LeftClick, fake_action_names_reply(&names), |name| {
                performed.push(name.to_string());
                0
            },
        );
        assert_eq!(result.unwrap(), 0);
        assert_eq!(performed, vec!["AXPress"]);
    }

    #[test]
    fn action_metadata_copy_rule_output_is_released_on_success_and_sdk_failure() {
        let array = core_foundation::array::CFArray::from_CFTypes(&[CFString::new("AXPress")]);
        for code in [0, -25204] {
            let baseline = array.retain_count();
            let result = unsafe {
                decode_action_names_reply(code, CFRetain(array.as_concrete_TypeRef() as *const c_void))
            };
            assert_eq!(array.retain_count(), baseline);
            if code == 0 {
                assert_eq!(result.unwrap(), vec!["AXPress"]);
            } else {
                assert!(matches!(result, Err(A11yError::Backend(_))));
            }
        }
    }

    #[test]
    fn pre_action_unsupported_does_not_invent_a_mutation_dirty_fence() {
        assert!(!invocation_may_have_effect(&Err(A11yError::Unsupported {
            capability: "AXValue".into(), hint: "not writable before dispatch".into(),
        })));
        assert!(!invocation_may_have_effect(&Err(A11yError::Backend("read failed before dispatch".into()))));
        assert!(invocation_may_have_effect(&Err(A11yError::Uncertain("reply lost after dispatch".into()))));
        assert!(invocation_may_have_effect(&Ok(Effect { changed: true, message: "ack".into() })));
    }

    #[test]
    fn accepted_mutating_request_without_actor_reply_is_uncertain() {
        for operation in 0..3 {
            let (sender, commands) = channel();
            let handle = ActorHandle { tx: Mutex::new(sender) };
            let peer = std::thread::spawn(move || {
                let command = commands.recv().unwrap();
                assert!(matches!(command, Cmd::Invoke(_, _, _, _) | Cmd::Focus(_, _) | Cmd::FocusNamed(_, _, _, _)));
                // Drop the accepted command and its reply sender. No native AX
                // implementation or run loop is created by this fake peer.
            });
            let result = match operation {
                0 => handle.invoke(Target::Ref(7), SnapshotGen(9), ElementAction::SetValue("private audit text".into())),
                1 => handle.focus_window(41),
                _ => handle.focus_named_window(41, Rect { x: 0.0, y: 0.0, w: 80.0, h: 20.0 }, None),
            };
            let error = result.unwrap_err();
            assert!(matches!(&error, A11yError::Uncertain(_)));
            assert!(error.to_string().contains("actor accepted"));
            assert!(!error.to_string().contains("private audit text"));
            peer.join().unwrap();
        }
    }

    #[test]
    fn mutating_request_rejected_before_actor_admission_remains_backend_error() {
        let (sender, commands) = channel();
        drop(commands);
        let handle = ActorHandle { tx: Mutex::new(sender) };
        assert!(matches!(handle.invoke(Target::Ref(7), SnapshotGen(9), ElementAction::LeftClick),
            Err(A11yError::Backend(_))));
    }

    #[test]
    fn left_click_never_selects_confirm_open_or_pick() {
        for name in ["AXConfirm", "AXOpen", "AXPick"] {
            let actions = vec![name.to_string()];
            assert_eq!(semantic_press_action(&ElementAction::LeftClick, &actions), None);
            assert_eq!(semantic_press_action(&ElementAction::Press, &actions), Some(name));
        }
        let actions = vec!["AXConfirm".to_string(), "AXPress".to_string()];
        assert_eq!(semantic_press_action(&ElementAction::LeftClick, &actions), Some("AXPress"));
        assert_eq!(semantic_press_action(&ElementAction::Press, &actions), Some("AXPress"));
        assert_eq!(semantic_press_action(&ElementAction::DoubleClick, &actions), None);
    }

    #[test]
    fn scalar_values_preserve_numeric_state_and_bound_text_without_gui() {
        let checked = core_foundation::boolean::CFBoolean::true_value();
        let number = CFNumber::from(0.25_f64);
        let text = CFString::new(&"汉".repeat(2001));
        unsafe {
            assert_eq!(scalar_value_string(checked.as_concrete_TypeRef() as _), Some("1".into()));
            assert_eq!(scalar_value_string(number.as_concrete_TypeRef() as _), Some("0.25".into()));
            assert_eq!(scalar_value_string(text.as_concrete_TypeRef() as _).unwrap().chars().count(), 2000);
            assert_eq!(scalar_value_string(std::ptr::null()), None);
        }
    }

    #[test]
    fn permission_revocation_and_invalid_handles_do_not_enable_pixel_fallback() {
        assert!(matches!(ax_error("press", -25211), A11yError::Permission(_)));
        assert!(matches!(ax_error("press", -25202), A11yError::Stale(_)));
        for error in [-25204, -25200, -25202, -25211, -25206] {
            let uncertain = ax_mutation_error("press", error);
            assert!(matches!(&uncertain, A11yError::Uncertain(_)));
            assert!(uncertain.to_string().contains("effect is unknown"));
            assert!(uncertain.to_string().contains("admitted to the native AX API"));
            assert!(uncertain.to_string().contains("no automatic retry"));
            assert!(uncertain.to_string().contains(&format!("AXError {error}")));
        }
        assert!(matches!(ax_error("read value", -25204), A11yError::Backend(_)));
        assert!(matches!(ax_error("press", -25206), A11yError::Backend(_)));
    }

    #[test]
    fn value_fallback_requires_read_only_proof_before_dispatch() {
        assert!(writable_attribute_result("AXValue", 0, 1).is_ok());
        for (err, writable) in [(0, 0), (-25205, 0), (-25208, 0)] {
            assert!(matches!(writable_attribute_result("AXValue", err, writable),
                Err(A11yError::Unsupported { .. })));
        }
        assert!(matches!(writable_attribute_result("AXValue", -25211, 0),
            Err(A11yError::Permission(_))));
        assert!(matches!(writable_attribute_result("AXValue", -25202, 0),
            Err(A11yError::Stale(_))));
        assert!(matches!(writable_attribute_result("AXValue", -25204, 0),
            Err(A11yError::Backend(_))));
    }

    #[test]
    fn window_matching_rejects_changed_title_geometry_and_invalid_bounds() {
        let expected = Rect { x: -600.0, y: 32.0, w: 800.0, h: 600.0 };
        assert!(matches_window_metadata(&expected, Some("Editor"), &expected, Some("Editor")));
        let rounded = Rect { x: -599.0, ..expected };
        assert!(matches_window_metadata(&rounded, Some("Editor"), &expected, Some("Editor")));
        assert!(!matches_window_metadata(&expected, Some("Other"), &expected, Some("Editor")));
        assert!(!matches_window_metadata(&expected, None, &expected, Some("Editor")));
        assert!(!matches_window_metadata(
            &Rect { x: -500.0, ..expected }, Some("Editor"), &expected, Some("Editor"),
        ));
        for invalid in [
            Rect { w: 0.0, ..expected },
            Rect { x: f64::NAN, ..expected },
            Rect { h: f64::INFINITY, ..expected },
        ] {
            assert!(!matches_window_metadata(&invalid, None, &expected, None));
            assert!(!matches_window_metadata(&expected, None, &invalid, None));
        }
    }

    #[test]
    fn window_matching_requires_one_candidate_before_activation() {
        assert_eq!(unique_window_index(&[4], 7).unwrap(), 4);
        assert!(matches!(unique_window_index(&[], 7), Err(A11yError::NotFound(_))));
        assert!(matches!(unique_window_index(&[0, 1], 7), Err(A11yError::Stale(_))));
    }

    #[test]
    fn secure_and_unnamed_editable_controls_are_identified() {
        assert!(is_secure_text(Some("AXTextField"), Some("AXSecureTextField")));
        assert!(is_secure_text(Some("AXSecureTextField"), None));
        assert!(!is_secure_text(Some("AXTextField"), Some("AXSearchField")));
        assert!(is_editable_role(Some("AXTextField")));
        assert!(is_editable_role(Some("AXTextArea")));
        assert!(!is_editable_role(Some("AXGroup")));
        assert_eq!(action_label(&ElementAction::SetValue("secret".to_string())), "set value");
    }

    #[test]
    fn retained_target_requires_the_same_live_content_and_bounds() {
        let entry = ElementEntry {
            r#ref: 1,
            role: "textfield".to_string(),
            name: Some("Name".to_string()),
            value: Some("Alice".to_string()),
            states: Vec::new(),
            bounds: Rect { x: 100.0, y: 200.0, w: 200.0, h: 24.0 },
            source: Source::A11y,
        };
        assert!(live_entry_matches(
            Some("AXTextField"), Some("Name"), Some("Alice"), Some(&entry.bounds), &entry,
        ));
        assert!(!live_entry_matches(
            Some("AXTextField"), Some("Name"), Some("Bob"), Some(&entry.bounds), &entry,
        ));
        assert!(!live_entry_matches(
            Some("AXButton"), Some("Name"), Some("Alice"), Some(&entry.bounds), &entry,
        ));
        assert!(!live_entry_matches(None, Some("Name"), Some("Alice"), Some(&entry.bounds), &entry));
        let moved = Rect { x: 101.0, ..entry.bounds };
        assert!(!live_entry_matches(
            Some("AXTextField"), Some("Name"), Some("Alice"), Some(&moved), &entry,
        ));
    }

    #[test]
    fn invoke_rejects_dirty_unobserved_and_replaced_frontmost_snapshots() {
        let validate = |dirty, observer_active, observed_pid, current_pid| {
            validate_invoke_snapshot(
                7,
                SnapshotGen(3),
                SnapshotGen(3),
                dirty,
                observer_active,
                true,
                observed_pid,
                current_pid,
            )
        };
        assert!(validate(true, true, Some(11), Some(11))
            .unwrap_err()
            .to_string()
            .contains("invalidated"));
        assert!(validate(false, false, Some(11), Some(11))
            .unwrap_err()
            .to_string()
            .contains("no live Accessibility change observer"));
        assert!(validate(false, true, Some(11), Some(12))
            .unwrap_err()
            .to_string()
            .contains("frontmost application"));
        assert!(validate(false, true, Some(11), None)
            .unwrap_err()
            .to_string()
            .contains("frontmost application"));
        assert!(validate(false, true, Some(11), Some(11)).is_ok());
    }

    #[test]
    fn explicit_pid_snapshot_ignores_other_frontmost_app_but_not_generation_or_dirty_state() {
        assert!(validate_invoke_snapshot(
            7,
            SnapshotGen(3),
            SnapshotGen(3),
            false,
            true,
            false,
            Some(11),
            None,
        )
        .is_ok());
        assert!(validate_invoke_snapshot(
            7,
            SnapshotGen(2),
            SnapshotGen(3),
            false,
            true,
            false,
            Some(11),
            None,
        )
        .is_err());
        assert!(validate_invoke_snapshot(
            7,
            SnapshotGen(3),
            SnapshotGen(3),
            true,
            true,
            false,
            Some(11),
            None,
        )
        .is_err());
    }
}
