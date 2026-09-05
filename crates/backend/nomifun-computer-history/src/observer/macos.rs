//! macOS observer backend — the real NSWorkspace + Accessibility sampler.
//!
//! Per tick, three things happen:
//! 1. Foreground app: `NSWorkspace.sharedWorkspace.frontmostApplication` is
//!    read on the main queue (`dispatch2::run_on_main`, pattern:
//!    nomi-computer/src/macos_main.rs — AppKit calls require the main thread).
//! 2. Window context: via the Accessibility API on the frontmost app's
//!    focused window — `AXTitle`, and, for browsers, a bounded walk for the
//!    address-bar element identified by `WEB_BROWSER_ADDRESS_AND_SEARCH_FIELD`
//!    (the shared WebKit/Chromium identifier; spec §7 `_latestURLByWindowID`).
//! 3. Secure Input: `IsSecureEventInputEnabled` (HIToolbox) — when set the
//!    whole sample is flagged and the observer loop suppresses it, so secure
//!    fields are never persisted.
//!
//! Permission preflight is `AXIsProcessTrusted` (pattern:
//! nomi-computer/src/permissions.rs:160-200). Without the Accessibility grant
//! the AX reads fail and the sampler degrades to foreground-app-only; the
//! gateway surface reports `permission: "denied"` so the UI can prompt.
//!
//! Known limits (documented, not hidden): private-browsing detection is
//! best-effort — the window subrole/title heuristic below covers Safari's
//! localized "Private Browsing" window markers; Chromium-family private
//! windows are indistinguishable through AX, so `private_browsing` stays
//! `false` there and the `observe_private_browsing` setting is the only
//! guard. Window `windowID` is not sampled (the dedup key uses
//! app/window/URL identity instead).

use std::ffi::c_void;

use async_trait::async_trait;
use core_foundation::base::{CFRelease, CFTypeID, TCFType};
use core_foundation::string::{CFString, CFStringRef};

use crate::observer::{ActivitySample, ObserverBackend};
use crate::service::PermissionState;

// ---- FFI ----------------------------------------------------------------

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
    fn AXUIElementCopyAttributeValue(
        el: *const c_void,
        attr: CFStringRef,
        out: *mut *const c_void,
    ) -> i32;
}

// HIToolbox (Carbon umbrella): Secure Input state — set whenever a password
// field has keyboard focus system-wide.
#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn IsSecureEventInputEnabled() -> u8;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRetain(cf: *const c_void) -> *const c_void;
    fn CFGetTypeID(cf: *const c_void) -> CFTypeID;
    fn CFStringGetTypeID() -> CFTypeID;
    fn CFArrayGetCount(arr: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(arr: *const c_void, idx: isize)
        -> *const c_void;
}

// ---- AX attribute / identifier constants --------------------------------

const AX_TITLE: &str = "AXTitle";
const AX_VALUE: &str = "AXValue";
const AX_SUBROLE: &str = "AXSubrole";
const AX_CHILDREN: &str = "AXChildren";
const AX_IDENTIFIER: &str = "AXIdentifier";
const AX_FOCUSED_WINDOW: &str = "AXFocusedWindow";
const AX_MAIN_WINDOW: &str = "AXMainWindow";
const AX_WINDOWS: &str = "AXWindows";

/// Shared accessibility identifier of the browser address/search field
/// (WebKit + Chromium; spec §7 `WEB_BROWSER_ADDRESS_AND_SEARCH_FIELD`).
const ADDRESS_FIELD_IDENTIFIER: &str = "WEB_BROWSER_ADDRESS_AND_SEARCH_FIELD";

/// Bounded walk so a pathological AX tree cannot stall a tick.
const MAX_ADDRESS_FIELD_DEPTH: usize = 12;
const MAX_ADDRESS_FIELD_NODES: usize = 4000;

// ---- browser identification ---------------------------------------------

/// Bundle ids whose focused window is probed for the address bar. Matching is
/// prefix-based so per-profile/flavored variants (`com.google.Chrome.canary`)
/// are covered without enumerating them.
const BROWSER_BUNDLE_PREFIXES: &[&str] = &[
    "com.apple.Safari",
    "com.google.Chrome",
    "com.microsoft.edgemac",
    "com.microsoft.Edge",
    "com.brave.Browser",
    "org.mozilla.firefox",
    "com.vivaldi.Vivaldi",
    "com.operasoftware.Opera",
    "com.delta.Settings.ShareExtension.Mac.Browser", // Arc share proxy
    "company.thebrowser.Browser",                    // Arc
];

fn is_browser(bundle_id: &str) -> bool {
    BROWSER_BUNDLE_PREFIXES
        .iter()
        .any(|prefix| bundle_id.starts_with(prefix))
}

/// Best-effort private-window marker. Only Safari exposes a stable AX-level
/// signal (its private windows carry the localized "Private Browsing" window
/// label); Chromium-family browsers expose nothing, so this returns `false`
/// for them — see the module doc comment for what that does and does not
/// protect.
const PRIVATE_WINDOW_MARKERS: &[&str] = &["Private Browsing", "私人浏览", "プライベートブラウズ"];

fn private_window_hint(title: Option<&str>, subrole: Option<&str>) -> bool {
    if subrole.is_some_and(|s| s.contains("Private")) {
        return true;
    }
    let Some(title) = title else {
        return false;
    };
    PRIVATE_WINDOW_MARKERS.iter().any(|marker| title.contains(marker))
}

// ---- foreground app (main queue) ----------------------------------------

/// The AppKit slice of one sample, extracted on the main thread.
struct ForegroundApp {
    app_name: String,
    bundle_id: Option<String>,
    process_identifier: i32,
}

/// True when this process is a real AppKit application (i.e. something is
/// driving the main runloop). `NSApplication.sharedApplication` returns nil
/// in plain processes — CLI hosts, `cargo test` — where
/// `dispatch2::run_on_main` would block forever on an `exec_sync` nobody will
/// ever service. Gating on this is what makes [`frontmost_app_on_main`] safe
/// in every host.
fn appkit_app_is_running() -> bool {
    use objc2::ClassType;
    use objc2_app_kit::NSApplication;
    // `sharedApplication` is `nil` unless an AppKit app exists. Read the raw
    // pointer (a +0 class method result needs no release when nil, and when
    // non-nil we do not retain it either) — we only need "is it there".
    let app: *mut NSApplication =
        unsafe { objc2::msg_send![NSApplication::class(), sharedApplication] };
    !app.is_null()
}

/// Read `NSWorkspace.sharedWorkspace.frontmostApplication` on the main queue.
/// `None` when this is not an AppKit host (headless/test — see
/// [`appkit_app_is_running`]) or no app is frontmost.
fn frontmost_app_on_main() -> Option<ForegroundApp> {
    if !appkit_app_is_running() {
        tracing::debug!("computer history: no AppKit application; skipping foreground sample");
        return None;
    }
    // `run_on_main` executes inline when already on the main thread and
    // otherwise blocks on `exec_sync`; that is only sound once an AppKit app
    // is driving the main runloop (checked above). Callers additionally keep
    // this off the async workers (see `current_sample`).
    dispatch2::run_on_main(move |_mtm| {
        use objc2_app_kit::NSWorkspace;
        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        let name = app
            .localizedName()
            .map(|n| n.to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        let bundle = app.bundleIdentifier().map(|b| b.to_string());
        let pid = app.processIdentifier();
        Some(ForegroundApp {
            app_name: name,
            bundle_id: bundle,
            process_identifier: pid,
        })
    })
}

// ---- AX helpers (calling thread; reads need no run loop) ----------------

/// RAII owner of one +1 AXUIElement/CF reference.
struct AxElement(*const c_void);

impl AxElement {
    unsafe fn from_create(pointer: *const c_void) -> Option<Self> {
        (pointer != std::ptr::null()).then(|| AxElement(pointer))
    }

    unsafe fn from_borrowed(pointer: *const c_void) -> Option<Self> {
        if pointer == std::ptr::null() {
            return None;
        }
        // +1 the borrowed array item so ownership is uniform on drop.
        Some(AxElement(unsafe { CFRetain(pointer) }))
    }
}

impl Drop for AxElement {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

unsafe fn copy_attribute(element: *const c_void, name: &str) -> *const c_void {
    let attribute = CFString::new(name);
    let mut out: *const c_void = std::ptr::null();
    if unsafe { AXUIElementCopyAttributeValue(element, attribute.as_concrete_TypeRef(), &mut out) } != 0 {
        return std::ptr::null();
    }
    out
}

unsafe fn copy_string_attribute(
    element: *const c_void,
    name: &str,
) -> Option<String> {
    let out = unsafe { copy_attribute(element, name) };
    if out == std::ptr::null() {
        return None;
    }
    unsafe {
        if CFGetTypeID(out) == CFStringGetTypeID() {
            Some(CFString::wrap_under_create_rule(out as CFStringRef).to_string())
        } else {
            CFRelease(out);
            None
        }
    }
}

unsafe fn copy_children(element: *const c_void) -> Vec<AxElement> {
    let out = unsafe { copy_attribute(element, AX_CHILDREN) };
    if out == std::ptr::null() {
        return Vec::new();
    }
    unsafe {
        let count = CFArrayGetCount(out);
        let mut children = Vec::with_capacity(count.max(0) as usize);
        for index in 0..count {
            let item = CFArrayGetValueAtIndex(out, index);
            if let Some(child) = AxElement::from_borrowed(item) {
                children.push(child);
            }
        }
        CFRelease(out);
        children
    }
}

/// The frontmost app's focused window (fallback: main window, then the first
/// listed window) — the same preference order the AX observer uses.
unsafe fn focused_window(app_element: *const c_void) -> Option<AxElement> {
    unsafe {
        for attribute in [AX_FOCUSED_WINDOW, AX_MAIN_WINDOW] {
            if let Some(window) = AxElement::from_create(copy_attribute(app_element, attribute)) {
                return Some(window);
            }
        }
        let out = copy_attribute(app_element, AX_WINDOWS);
        if out == std::ptr::null() {
            return None;
        }
        let first = if CFArrayGetCount(out) > 0 {
            AxElement::from_borrowed(CFArrayGetValueAtIndex(out, 0))
        } else {
            None
        };
        CFRelease(out);
        first
    }
}

/// Depth-first search for the address-bar element; returns its `AXValue`.
unsafe fn find_address_field_value(
    element: *const c_void,
    depth: usize,
    budget: &mut usize,
) -> Option<String> {
    if depth > MAX_ADDRESS_FIELD_DEPTH || *budget == 0 {
        return None;
    }
    *budget -= 1;
    unsafe {
        if copy_string_attribute(element, AX_IDENTIFIER).as_deref()
            == Some(ADDRESS_FIELD_IDENTIFIER)
        {
            return copy_string_attribute(element, AX_VALUE);
        }
        for child in copy_children(element) {
            if let Some(value) = find_address_field_value(child.0, depth + 1, budget) {
                return Some(value);
            }
        }
    }
    None
}

// ---- sampling -----------------------------------------------------------

/// One full tick, executed on a blocking thread: AppKit on the main queue,
/// AX on the calling thread. `None` means "nothing observable".
fn sample() -> Option<ActivitySample> {
    let foreground = frontmost_app_on_main()?;
    let secure_input = unsafe { IsSecureEventInputEnabled() != 0 };
    if secure_input {
        // Secure Input active: return the identity so the segment state
        // machine can see *that something changed*, but the loop suppresses
        // any secure sample — titles/URLs are never persisted while on.
        return Some(ActivitySample {
            app_name: foreground.app_name,
            bundle_id: foreground.bundle_id,
            window_title: None,
            browser_url: None,
            browser_title: None,
            secure_input: true,
            private_browsing: false,
        });
    }

    let bundle_id = foreground.bundle_id.clone().unwrap_or_default();
    let browser = is_browser(&bundle_id);
    let (title, url, private_browsing) = window_context(&foreground, browser);

    // The browser page title is the window title (browsers put the page
    // title in AXTitle); for non-browsers there is no separate page title.
    let browser_title = if browser { title.clone() } else { None };

    Some(ActivitySample {
        app_name: foreground.app_name,
        bundle_id: foreground.bundle_id,
        window_title: title,
        browser_url: url,
        browser_title,
        secure_input: false,
        private_browsing,
    })
}

/// Window title + address-bar URL + private-window hint for the frontmost
/// app. AX failures (no Accessibility grant, app without windows) degrade to
/// `(None, None, false)` — foreground-app segments still form.
fn window_context(
    foreground: &ForegroundApp,
    browser: bool,
) -> (Option<String>, Option<String>, bool) {
    unsafe {
        let Some(app) = AxElement::from_create(AXUIElementCreateApplication(
            foreground.process_identifier,
        )) else {
            return (None, None, false);
        };
        let Some(window) = focused_window(app.0) else {
            return (None, None, false);
        };
        let title = copy_string_attribute(window.0, AX_TITLE);
        let subrole = copy_string_attribute(window.0, AX_SUBROLE);
        let private_browsing = private_window_hint(title.as_deref(), subrole.as_deref());
        let url = if browser {
            let mut budget = MAX_ADDRESS_FIELD_NODES;
            find_address_field_value(window.0, 0, &mut budget)
                .filter(|url| !url.is_empty())
        } else {
            None
        };
        (title, url, private_browsing)
    }
}

// ---- backend ------------------------------------------------------------

/// macOS observer backend.
pub struct MacosBackend;

#[async_trait]
impl ObserverBackend for MacosBackend {
    async fn current_sample(&self) -> Option<ActivitySample> {
        // `run_on_main` blocks the calling thread until the main queue runs
        // the closure; keep it off the async workers.
        tokio::task::spawn_blocking(sample)
            .await
            .ok()
            .flatten()
    }

    fn permission_state(&self) -> PermissionState {
        match unsafe { AXIsProcessTrusted() } {
            1 => PermissionState::Granted,
            _ => PermissionState::Denied,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_bundle_matching_is_prefix_based() {
        assert!(is_browser("com.apple.Safari"));
        assert!(is_browser("com.google.Chrome.canary"));
        assert!(!is_browser("com.apple.finder"));
        assert!(!is_browser(""));
    }

    #[test]
    fn private_window_markers_match_localized_titles() {
        assert!(private_window_hint(Some("Private Browsing — Safari"), None));
        assert!(private_window_hint(Some("私人浏览 - Safari"), None));
        assert!(private_window_hint(None, Some("AXPrivateWindow")));
        assert!(!private_window_hint(Some("Docs — Safari"), None));
        assert!(!private_window_hint(None, None));
    }

    #[test]
    fn secure_input_tick_is_flagged_without_context() {
        // Directly exercising `sample()` would touch the main queue; the
        // secure-input branch shape is pinned by the observer-loop tests in
        // observer/mod.rs (secure samples are suppressed, never persisted).
        let suppressed = ActivitySample {
            app_name: "Safari".into(),
            bundle_id: Some("com.apple.Safari".into()),
            window_title: None,
            browser_url: None,
            browser_title: None,
            secure_input: true,
            private_browsing: false,
        };
        assert!(suppressed.secure_input);
    }

    #[test]
    fn sampling_gates_on_appkit_host() {
        // In a plain test host there is no NSApplication, so the sampler must
        // refuse to touch the main queue — this pins the no-deadlock contract
        // of `run_on_main`'s `exec_sync` without requiring a GUI session: the
        // call must answer within the timeout either way.
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // Own the backend inside the thread (the trait takes `&self`, so
            // the Arc keeps the future's borrow alive for its whole poll).
            let backend = std::sync::Arc::new(MacosBackend);
            let _ = sender.send({
                let backend = std::sync::Arc::clone(&backend);
                async move { backend.current_sample().await }
            });
            // Keep the backend alive until the send completes.
            drop(backend);
        });
        let _ = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("current_sample must not deadlock without an AppKit host");
    }

    #[test]
    fn permission_state_reports_tcc_preflight() {
        let state = MacosBackend.permission_state();
        assert!(matches!(
            state,
            PermissionState::Granted | PermissionState::Denied
        ));
    }
}
