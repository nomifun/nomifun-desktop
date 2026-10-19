//! Current-process windows must never receive AXRaise on the AX actor:
//! Accessibility dispatches that action directly into this AppKit process.
//! Keep all local NSApplication/NSWindow access on the existing main queue;
//! only scalar window identity and metadata cross the queue boundary.

use std::time::{Duration, Instant};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

use nomi_a11y::{Effect, Rect};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSWindow, NSWorkspace};
use xcap::Window;

#[derive(Clone)]
pub(super) struct SelfWindowTarget {
    pub window_id: u32,
    pub pid: i32,
    pub bounds: Rect,
    pub title: Option<String>,
}

fn metadata_matches(target: &SelfWindowTarget, pid: i32, bounds: Rect, title: Option<&str>) -> bool {
    target.pid > 0 && target.pid == pid && target.window_id > 0
        && [target.bounds.x, target.bounds.y, target.bounds.w, target.bounds.h,
            bounds.x, bounds.y, bounds.w, bounds.h].iter().all(|v| v.is_finite())
        && target.bounds.w > 0.0 && target.bounds.h > 0.0 && bounds.w > 0.0 && bounds.h > 0.0
        && (target.bounds.x - bounds.x).abs() <= 1.0
        && (target.bounds.y - bounds.y).abs() <= 1.0
        && (target.bounds.w - bounds.w).abs() <= 1.0
        && (target.bounds.h - bounds.h).abs() <= 1.0
        && target.title.as_deref().is_none_or(|expected| Some(expected) == title)
}

fn verify_current_metadata(target: &SelfWindowTarget) -> Result<(), String> {
    let windows = Window::all().map_err(|_| "Cannot re-read exact current-process window metadata".to_owned())?;
    let window = windows.iter().find(|window| window.id().ok() == Some(target.window_id))
        .ok_or_else(|| "The selected current-process window no longer exists".to_owned())?;
    let pid = window.pid().ok().and_then(|pid| i32::try_from(pid).ok());
    let actual = match (window.x(), window.y(), window.width(), window.height()) {
        (Ok(x), Ok(y), Ok(w), Ok(h)) => Rect { x: f64::from(x), y: f64::from(y), w: f64::from(w), h: f64::from(h) },
        _ => return Err("The exact current-process window bounds are unreadable".into()),
    };
    let title = window.title().ok().filter(|title| !title.is_empty());
    if !metadata_matches(target, pid.unwrap_or(-1), actual, title.as_deref()) {
        return Err("The selected current-process window owner, bounds or title changed before activation".into());
    }
    Ok(())
}

fn focus_matches(
    target_id: u32, target_pid: i32, window_id: isize, key_window_id: Option<isize>,
    app_active: bool, key: bool, minimized: bool, visible: bool, frontmost_pid: Option<i32>,
) -> bool {
    target_id > 0 && target_pid > 0 && u32::try_from(window_id).ok() == Some(target_id)
        && key_window_id == Some(window_id) && app_active && key && !minimized && visible
        && frontmost_pid == Some(target_pid)
}

fn has_exact_focus(app: &NSApplication, window: &NSWindow, target: &SelfWindowTarget) -> bool {
    let frontmost = NSWorkspace::sharedWorkspace().frontmostApplication().map(|app| app.processIdentifier());
    focus_matches(target.window_id, target.pid, window.windowNumber(), app.keyWindow().map(|window| window.windowNumber()),
        app.isActive(), window.isKeyWindow(), window.isMiniaturized(), window.isVisible(), frontmost)
}

fn with_selected_window<T>(target: &SelfWindowTarget, operation: impl FnOnce(&NSApplication, &NSWindow) -> Result<T, String>) -> Result<T, String> {
    let main = MainThreadMarker::new().ok_or_else(|| "Current-process AppKit window access requires the main queue".to_owned())?;
    let app = NSApplication::sharedApplication(main);
    let number = isize::try_from(target.window_id).map_err(|_| "Current-process window number is invalid".to_owned())?;
    let window = app.windowWithWindowNumber(number).ok_or_else(|| "The exact current-process NSWindow is unavailable".to_owned())?;
    if window.windowNumber() != number { return Err("Current-process NSWindow identity changed".into()); }
    operation(&app, &window)
}

pub(super) fn focus(target: SelfWindowTarget, cancellation: &tokio_util::sync::CancellationToken) -> Result<Effect, String> {
    crate::native_control::require_open(cancellation)?;
    if u32::try_from(target.pid).ok() != Some(std::process::id()) {
        return Err("Current-process AppKit focus rejected another process before dispatch".into());
    }
    if MainThreadMarker::new().is_some() {
        return Err("Current-process focus supervisor requires an off-main worker before dispatch".into());
    }
    let submitted = Arc::new(AtomicBool::new(false));
    let submitted_on_main = submitted.clone();
    let selected = target.clone();
    let native_cancellation = cancellation.clone();
    let unchanged = crate::macos_main::run_blocking_controlled(cancellation, move || with_selected_window(&selected, |app, window| {
        verify_current_metadata(&selected)?;
        if has_exact_focus(app, window, &selected) { return Ok(true); }
        crate::native_control::require_open(&native_cancellation)?;
        // Record before the first native mutation, including restore. A lost
        // dispatcher reply must not turn an admitted request into no-effect.
        submitted_on_main.store(true, Ordering::Release);
        if window.isMiniaturized() { window.deminiaturize(None); }
        app.activate();
        window.makeKeyAndOrderFront(None);
        Ok(false)
    })).map_err(|error| if submitted.load(Ordering::Acquire) {
        format!("Current-process focus effect is uncertain after AppKit dispatch: {error}")
    } else if error == crate::native_control::CANCELLED_BEFORE_DISPATCH {
        error
    } else {
        format!("Current-process focus rejected before AppKit mutation: {error}")
    })?;
    if unchanged {
        return Ok(Effect { changed: false, message: format!("the exact current-process window of pid {} already has keyboard focus", target.pid) });
    }

    // The AppKit request is asynchronous. Never sleep/spin the main queue
    // waiting for its own activation notifications. Each check is a separate
    // main-queue task; the existing dispatcher joins any admitted task and
    // cancels an unadmitted slot rather than letting it run late.
    let deadline = Instant::now() + Duration::from_millis(1200);
    loop {
        let selected = target.clone();
        let focused = crate::macos_main::run_blocking(move || with_selected_window(&selected, |app, window| {
            Ok(has_exact_focus(app, window, &selected))
        })).map_err(|error| format!("Current-process focus effect is uncertain after AppKit dispatch: {error}"))?;
        if focused {
            return Ok(Effect { changed: true, message: format!("activated the exact current-process window of pid {} through main-queue AppKit", target.pid) });
        }
        if Instant::now() >= deadline {
            return Err("Current-process focus effect is uncertain: exact frontmost PID and key NSWindow were not confirmed after AppKit dispatch; observe before another action".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> SelfWindowTarget {
        SelfWindowTarget { window_id: 42, pid: 73, bounds: Rect { x: 20.0, y: 30.0, w: 900.0, h: 700.0 }, title: Some("Fixture".into()) }
    }

    #[test]
    fn exact_self_target_rejects_replaced_owner_geometry_title_and_missing_focus() {
        let target = target();
        assert!(metadata_matches(&target, 73, target.bounds, Some("Fixture")));
        assert!(!metadata_matches(&target, 74, target.bounds, Some("Fixture")));
        assert!(!metadata_matches(&target, 73, Rect { x: 25.0, ..target.bounds }, Some("Fixture")));
        assert!(!metadata_matches(&target, 73, target.bounds, Some("Other")));
        assert!(!metadata_matches(&target, 73, Rect { w: f64::NAN, ..target.bounds }, Some("Fixture")));
        assert!(focus_matches(42, 73, 42, Some(42), true, true, false, true, Some(73)));
        for (id, key_id, active, key, minimized, visible, pid) in [
            (43, Some(43), true, true, false, true, Some(73)),
            (42, Some(43), true, true, false, true, Some(73)),
            (42, Some(42), false, true, false, true, Some(73)),
            (42, Some(42), true, false, false, true, Some(73)),
            (42, Some(42), true, true, true, true, Some(73)),
            (42, Some(42), true, true, false, false, Some(73)),
            (42, Some(42), true, true, false, true, Some(74)),
            (42, Some(42), true, true, false, true, None),
        ] { assert!(!focus_matches(42, 73, id, key_id, active, key, minimized, visible, pid)); }
    }

    #[test]
    fn other_process_is_rejected_before_main_queue_or_any_window_access() {
        let mut target = target();
        target.pid = -1;
        assert!(focus(target, &tokio_util::sync::CancellationToken::new()).unwrap_err().contains("another process before dispatch"));
    }
}
