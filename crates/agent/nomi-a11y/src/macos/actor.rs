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
//! AXObserver on the frontmost app. Change notifications flip a `dirty` flag so
//! `observe` re-serves the cached snapshot when nothing has changed and
//! re-walks the tree otherwise. Every mutating command also marks `dirty`, so
//! the cache is never stale after one of our own actions. OCR/vision fusion
//! lives one layer up (the computer tool fuses `nomi_a11y::ocr_screenshot`).

// This whole module is FFI against the Accessibility C API; every helper is an
// `unsafe fn` that is only valid on the actor thread. We keep the pre-2024
// "unsafe fn body is unsafe" ergonomics rather than wrapping each FFI call.
#![allow(unsafe_op_in_unsafe_fn)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

use core_foundation::base::TCFType;
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
    fn CFArrayGetCount(arr: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(arr: *const c_void, idx: isize) -> *const c_void;
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopRunInMode(mode: CFStringRef, seconds: f64, return_after_source_handled: u8) -> i32;
    fn CFRunLoopAddSource(rl: *mut c_void, source: *const c_void, mode: CFStringRef);
    fn CFRunLoopRemoveSource(rl: *mut c_void, source: *const c_void, mode: CFStringRef);
}

/// AXObserver notification callback: flips the `dirty` flag (passed as `refcon`)
/// so the next `observe` re-walks instead of re-serving a stale snapshot. Runs
/// on the actor thread (the run loop that owns the observer source).
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
    fn AXUIElementCreateSystemWide() -> *const c_void;
    fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
    fn AXUIElementCopyAttributeValue(
        el: *const c_void,
        attr: CFStringRef,
        out: *mut *const c_void,
    ) -> i32;
    fn AXUIElementSetAttributeValue(el: *const c_void, attr: CFStringRef, val: *const c_void)
        -> i32;
    fn AXUIElementCopyActionNames(el: *const c_void, out: *mut *const c_void) -> i32;
    fn AXUIElementPerformAction(el: *const c_void, action: CFStringRef) -> i32;
    fn AXUIElementGetPid(el: *const c_void, out: *mut i32) -> i32;
    fn AXValueGetValue(value: *const c_void, the_type: u32, out: *mut c_void) -> u8;
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
    let attr = CFString::new(name);
    let mut out: *const c_void = std::ptr::null();
    let err = AXUIElementCopyAttributeValue(el, attr.as_concrete_TypeRef(), &mut out);
    if err != 0 {
        std::ptr::null()
    } else {
        out
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
    AxElem::from_create(copy_attr_raw(el, name))
}

unsafe fn copy_children(el: *const c_void) -> Vec<AxElem> {
    let out = copy_attr_raw(el, "AXChildren");
    if out.is_null() {
        return Vec::new();
    }
    let n = CFArrayGetCount(out);
    let mut v = Vec::with_capacity(n.max(0) as usize);
    for i in 0..n {
        let item = CFArrayGetValueAtIndex(out, i);
        if let Some(e) = AxElem::from_borrowed(item) {
            v.push(e);
        }
    }
    CFRelease(out);
    v
}

unsafe fn copy_point(el: *const c_void, name: &str) -> Option<(f64, f64)> {
    let out = copy_attr_raw(el, name);
    if out.is_null() {
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

unsafe fn copy_actions(el: *const c_void) -> Vec<String> {
    let mut out: *const c_void = std::ptr::null();
    let err = AXUIElementCopyActionNames(el, &mut out);
    if err != 0 || out.is_null() {
        return Vec::new();
    }
    let n = CFArrayGetCount(out);
    let mut v = Vec::new();
    for i in 0..n {
        let item = CFArrayGetValueAtIndex(out, i);
        if !item.is_null() && CFGetTypeID(item) == CFStringGetTypeID() {
            v.push(CFString::wrap_under_get_rule(item as CFStringRef).to_string());
        }
    }
    CFRelease(out);
    v
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
) {
    if out.len() >= opts.node_budget {
        *truncated = true;
        return;
    }
    let role = copy_str_attr(el.ptr(), "AXRole");
    let name = copy_str_attr(el.ptr(), "AXTitle")
        .or_else(|| copy_str_attr(el.ptr(), "AXDescription"))
        .filter(|s| !s.trim().is_empty());
    let value = copy_str_attr(el.ptr(), "AXValue").filter(|s| !s.trim().is_empty());
    let pos = copy_point(el.ptr(), "AXPosition");
    let size = copy_size(el.ptr(), "AXSize");
    let actions = copy_actions(el.ptr());
    let actionable = is_action_actionable(&actions);
    let enabled = copy_bool_attr(el.ptr(), "AXEnabled").unwrap_or(true);

    if let (Some((x, y)), Some((w, h))) = (pos, size) {
        let bounds = Rect { x, y, w, h };
        if !bounds.is_empty() && (actionable || name.is_some()) {
            let mut states = Vec::new();
            if !enabled {
                states.push("disabled".to_string());
            }
            if copy_bool_attr(el.ptr(), "AXFocused").unwrap_or(false) {
                states.push("focused".to_string());
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

    if depth >= opts.max_depth {
        return;
    }
    for child in copy_children(el.ptr()) {
        if out.len() >= opts.node_budget {
            *truncated = true;
            return;
        }
        walk(&child, depth + 1, opts, out, truncated);
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
/// at the `dirty` flag. Returns `None` (caller then never caches) on failure.
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

/// The last walk, kept so repeated `observe`s on an unchanged window re-serve
/// instead of re-walking the tree.
struct CachedWalk {
    entries: Vec<ElementEntry>,
    app_name: Option<String>,
    window_title: Option<String>,
    pid: Option<i32>,
    truncated: bool,
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
    cached: Option<CachedWalk>,
}

unsafe fn focused_app() -> Result<AxElem, A11yError> {
    let sw = AxElem::from_create(AXUIElementCreateSystemWide()).ok_or_else(|| {
        A11yError::Backend("AXUIElementCreateSystemWide returned null".to_string())
    })?;
    copy_elem_attr(sw.ptr(), "AXFocusedApplication").ok_or_else(|| {
        let app = crate::host_app_label();
        A11yError::Permission(format!(
            "No focused application is readable — Accessibility permission is not in effect for \
             {app}. Grant it in System Settings → Privacy & Security → Accessibility (the entry is \
             named \"{app}\"), then COMPLETELY quit and reopen {app} — macOS does not apply this \
             permission to an already-running process. Computer-use runs inside {app} itself, so \
             do not grant a terminal or editor."
        ))
    })
}

unsafe fn focused_window(app: &AxElem) -> Option<AxElem> {
    copy_elem_attr(app.ptr(), "AXFocusedWindow")
        .or_else(|| copy_elem_attr(app.ptr(), "AXMainWindow"))
        .or_else(|| copy_children(app.ptr()).into_iter().next())
}

fn do_observe(opts: &ObserveOpts, state: &mut State) -> Result<Snapshot, A11yError> {
    unsafe {
        let app = match opts.pid {
            Some(pid) => AxElem::from_create(AXUIElementCreateApplication(pid))
                .ok_or_else(|| A11yError::NotFound(format!("no app for pid {pid}")))?,
            None => focused_app()?,
        };
        let app_pid = pid_of(app.ptr());

        // Cache re-serve: frontmost app unchanged, an observer is watching it,
        // and nothing has dirtied the snapshot since the last walk. (Explicit-pid
        // observes always re-walk.)
        if opts.pid.is_none()
            && app_pid.is_some()
            && state.observed_pid == app_pid
            && state.observer.is_some()
            && !state.dirty.load(Ordering::Relaxed)
        {
            if let Some(c) = &state.cached {
                return Ok(Snapshot {
                    generation: state.current_gen,
                    entries: c.entries.clone(),
                    overlay: None,
                    text: format_entries(&c.entries),
                    truncated: c.truncated,
                    pid: c.pid,
                    app_name: c.app_name.clone(),
                    window_title: c.window_title.clone(),
                });
            }
        }

        let app_name = copy_str_attr(app.ptr(), "AXTitle");
        let window = focused_window(&app).ok_or_else(|| {
            A11yError::NotFound("the focused application has no readable window".to_string())
        })?;
        let window_title = copy_str_attr(window.ptr(), "AXTitle");

        let mut collected = Vec::new();
        let mut truncated = false;
        walk(&window, 0, opts, &mut collected, &mut truncated);

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

        // (Re)register the change observer if the frontmost app changed.
        if state.observed_pid != app_pid {
            state.observer = None; // drop the old observer first (removes its source)
            if let Some(p) = app_pid {
                let refcon = (&*state.dirty as *const AtomicBool) as *mut c_void;
                state.observer = register_observer(p, &app, state.runloop, refcon);
            }
            state.observed_pid = app_pid;
        }
        state.dirty.store(false, Ordering::Relaxed);

        let text = format_entries(&entries);
        state.cached = Some(CachedWalk {
            entries: entries.clone(),
            app_name: app_name.clone(),
            window_title: window_title.clone(),
            pid: app_pid,
            truncated,
        });
        Ok(Snapshot {
            generation,
            entries,
            overlay: None, // the tool captures the screenshot + draws the overlay
            text,
            truncated,
            pid: app_pid,
            app_name,
            window_title,
        })
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
                       direct selector actuation is not yet implemented."
                    .to_string(),
            })
        }
        Target::Pixel { .. } => {
            return Err(A11yError::Unsupported {
                capability: "pixel targeting".to_string(),
                hint: "Pixel fallback is handled by the computer tool's input layer, not the \
                       accessibility engine."
                    .to_string(),
            })
        }
    };
    if generation != state.current_gen {
        return Err(A11yError::Stale(format!(
            "ref [{r}] is from an older snapshot (the UI may have changed); re-run observe and \
             use a fresh [ref]"
        )));
    }
    let elem = state
        .registry
        .get(&r)
        .ok_or_else(|| A11yError::NotFound(format!("no element [{r}] in the latest snapshot")))?;

    unsafe {
        let err = match action {
            ElementAction::Press | ElementAction::LeftClick | ElementAction::DoubleClick => {
                perform(elem.ptr(), "AXPress")
            }
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
            ElementAction::SetValue(v) => set_string_value(elem.ptr(), v),
        };
        if err == 0 {
            Ok(Effect {
                changed: true,
                message: format!("performed {action:?} on element [{r}]"),
            })
        } else {
            Err(A11yError::Backend(format!(
                "AX action on [{r}] failed (AXError {err}); the element may be a web view \
                 (AXWebArea) that ignores AXPress — fall back to a pixel click"
            )))
        }
    }
}

fn do_focus(pid: i32) -> Result<Effect, A11yError> {
    unsafe {
        let app = AxElem::from_create(AXUIElementCreateApplication(pid))
            .ok_or_else(|| A11yError::NotFound(format!("no app for pid {pid}")))?;
        let attr = CFString::new("AXFrontmost");
        let t = core_foundation::boolean::CFBoolean::true_value();
        let err = AXUIElementSetAttributeValue(
            app.ptr(),
            attr.as_concrete_TypeRef(),
            t.as_concrete_TypeRef() as *const c_void,
        );
        if let Some(win) = focused_window(&app) {
            let _ = perform(win.ptr(), "AXRaise");
        }
        if err == 0 {
            Ok(Effect {
                changed: true,
                message: format!("brought pid {pid} to the front"),
            })
        } else {
            Err(A11yError::Backend(format!(
                "could not activate pid {pid} (AXError {err})"
            )))
        }
    }
}

// ---- command plumbing ---------------------------------------------------

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
                                Cmd::Invoke(target, generation, action, reply) => {
                                    let r = do_invoke(&target, generation, &action, &state);
                                    // A mutating action invalidates the cache even
                                    // before the observer notification arrives.
                                    state.dirty.store(true, Ordering::Relaxed);
                                    let _ = reply.send(r);
                                }
                                Cmd::Focus(pid, reply) => {
                                    let r = do_focus(pid);
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

    pub fn invoke(
        &self,
        target: Target,
        generation: SnapshotGen,
        action: ElementAction,
    ) -> Result<Effect, A11yError> {
        let (tx, rx) = channel();
        self.send(Cmd::Invoke(target, generation, action, tx))?;
        rx.recv()
            .map_err(|_| A11yError::Backend("AX actor dropped the reply".to_string()))?
    }

    pub fn focus_window(&self, pid: i32) -> Result<Effect, A11yError> {
        let (tx, rx) = channel();
        self.send(Cmd::Focus(pid, tx))?;
        rx.recv()
            .map_err(|_| A11yError::Backend("AX actor dropped the reply".to_string()))?
    }
}
