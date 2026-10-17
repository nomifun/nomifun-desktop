//! Platform-neutral fallback backend: window enumeration and best-effort
//! focusing via xcap (cross-platform). This is NOT the Windows-OS backend —
//! the file was historically named `windows.rs`, which collided conceptually
//! with the future per-OS UI-Automation submodule; it is renamed to make clear
//! it is the generic xcap/enigo fallback used on every platform.
//!
//! xcap 0.9 has no focus/activate API. Windows therefore uses Win32 and macOS
//! uses the Accessibility actor to identify and activate the selected window.
//! Linux retains the explicitly reported click-to-raise fallback. Window
//! x/y/width/height from xcap are global logical points on macOS.

use xcap::Window;

#[cfg(target_os = "macos")]
#[path = "macos_self_focus.rs"]
mod macos_self_focus;

// The click-to-raise focus fallback (and only that) uses synthetic input; on
// Windows and macOS activate the real window through native APIs.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
use crate::input;

/// A snapshot of one window's metadata.
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub app_name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub is_focused: bool,
}

/// Enumerate windows (front-to-back z order). Blocking: call from
/// spawn_blocking. On macOS, xcap window metadata checks can touch AppKit
/// (`NSWorkspace`), so enumeration is dispatched to the main queue.
pub fn list_windows() -> Result<Vec<WindowInfo>, String> {
    crate::macos_main::run_blocking(list_windows_inner)
}

fn list_windows_inner() -> Result<Vec<WindowInfo>, String> {
    let windows = Window::all().map_err(|e| format!("Failed to enumerate windows: {e}"))?;
    let mut infos = Vec::with_capacity(windows.len());
    for w in &windows {
        // Skip windows whose metadata cannot be read instead of failing the
        // whole listing.
        let Ok(id) = w.id() else { continue };
        infos.push(WindowInfo {
            id,
            title: w.title().unwrap_or_default(),
            app_name: w.app_name().unwrap_or_default(),
            x: w.x().unwrap_or(0),
            y: w.y().unwrap_or(0),
            width: w.width().unwrap_or(0),
            height: w.height().unwrap_or(0),
            is_focused: w.is_focused().unwrap_or(false),
        });
    }
    Ok(infos)
}

/// Render a window list as human-readable text for the LLM.
pub fn format_window_list(windows: &[WindowInfo]) -> String {
    if windows.is_empty() {
        return "No windows found.".to_string();
    }
    let mut out = String::from("Windows (front to back):\n");
    for w in windows {
        let focus = if w.is_focused { " [focused]" } else { "" };
        out.push_str(&format!(
            "- id={} app={:?} title={:?} at ({}, {}) size {}x{}{}\n",
            w.id, w.app_name, w.title, w.x, w.y, w.width, w.height, focus
        ));
    }
    out
}

/// Find a window by id. Blocking: call from spawn_blocking.
pub fn find_window(window_id: u32) -> Result<WindowInfo, String> {
    let windows = list_windows()?;
    windows
        .into_iter()
        .find(|w| w.id == window_id)
        .ok_or_else(|| {
            format!(
                "Window {window_id} not found. Use the list_windows action to get current ids."
            )
        })
}

/// Bring a window to the foreground.
///
/// On Windows this uses the real activation API (`SetForegroundWindow` on the
/// exact target HWND), preserving OS foreground policy and verifying native
/// acknowledgement before subsequent `type`/`key` input.
/// On macOS, match the window's current owner, title and bounds in the
/// Accessibility tree and verify native focus. Linux uses click-to-raise.
pub async fn focus_window(window_id: u32) -> Result<String, String> {
    #[cfg(not(target_os = "macos"))]
    let info = tokio::task::spawn_blocking(move || find_window(window_id))
        .await
        .map_err(|e| format!("Window lookup task failed: {e}"))??;

    #[cfg(target_os = "windows")]
    {
        tokio::task::spawn_blocking(move || nomi_a11y::focus_windows_window(window_id))
            .await
            .map_err(|e| format!("Focus task failed: {e}"))?
            .map_err(windows_focus_error_message)?;
        Ok(format!(
            "Activated window {window_id} ({:?} — {:?}) via the Windows foreground API. \
             Take a screenshot to verify.",
            info.app_name, info.title
        ))
    }

    #[cfg(target_os = "macos")]
    {
        tokio::task::spawn_blocking(move || focus_window_macos(window_id))
            .await
            .map_err(|e| format!("Focus task failed: {e}"))?
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if info.width == 0 || info.height == 0 {
            return Err(format!(
                "Window {window_id} ({:?}) has zero size; it may be minimized or hidden. \
                 Cannot focus it by clicking.",
                info.title
            ));
        }

        let cx = info.x + info.width as i32 / 2;
        let cy = info.y + info.height as i32 / 2;
        input::click(cx, cy, enigo::Button::Left, 1).await?;
        Ok(format!(
            "Clicked the center of window {window_id} ({:?} — {:?}) at ({cx}, {cy}) to focus it. \
             Note: the platform exposes no direct focus API, so this is a click-to-raise fallback; \
             the click may interact with whatever is at the window center. \
             Take a screenshot to verify the result.",
            info.app_name, info.title
        ))
    }
}

#[cfg(target_os = "macos")]
fn focus_window_macos(window_id: u32) -> Result<String, String> {
    if crate::permissions::accessibility_granted() == Some(false) {
        return Err(format!(
            "Accessibility permission is not granted, so the window cannot be activated. {}",
            crate::permissions::accessibility_hint_detailed(),
        ));
    }
    // Re-read metadata for this exact current CGWindowID. Invented origins or
    // dimensions could resolve a different window in the AX tree, so none of
    // the generic display-list fallbacks are appropriate here.
    let (pid, bounds, title) = crate::macos_main::run_blocking(move || {
        let windows = Window::all().map_err(|e| format!("Failed to enumerate windows: {e}"))?;
        let window = windows.iter().find(|window| window.id().ok() == Some(window_id))
            .ok_or_else(|| format!("Window {window_id} no longer exists; run list_windows again."))?;
        let metadata_error = |e| format!("Cannot identify macOS window {window_id} safely: {e}");
        let pid = i32::try_from(window.pid().map_err(metadata_error)?)
            .ok().filter(|pid| *pid > 0)
            .ok_or_else(|| format!("Window {window_id} has no valid owner process"))?;
        let bounds = nomi_a11y::Rect {
            x: f64::from(window.x().map_err(metadata_error)?),
            y: f64::from(window.y().map_err(metadata_error)?),
            w: f64::from(window.width().map_err(metadata_error)?),
            h: f64::from(window.height().map_err(metadata_error)?),
        };
        if bounds.w <= 0.0 || bounds.h <= 0.0 {
            return Err(format!("Window {window_id} has no readable non-zero bounds"));
        }
        let title = window.title().ok().filter(|title| !title.is_empty());
        Ok((pid, bounds, title))
    })?;
    if u32::try_from(pid).ok() == Some(std::process::id()) {
        let effect = macos_self_focus::focus(macos_self_focus::SelfWindowTarget { window_id, pid, bounds, title })
            .map_err(|error| format!("Could not activate current-process macOS window {window_id}: {error}"))?;
        return Ok(format!("Activated window {window_id} via macOS AppKit. {}", effect.message));
    }
    let effect = nomi_a11y::focus_named_window(pid, bounds, title)
        .map_err(|e| format!("Could not activate macOS window {window_id}: {e}"))?;
    Ok(format!("Activated window {window_id} via macOS Accessibility. {}", effect.message))
}

#[cfg(target_os = "windows")]
pub(crate) const WINDOWS_FOCUS_NOT_APPLIED: &str = "COMPUTER_FOCUS_NOT_APPLIED: Windows denied SetForegroundWindow before any restore, show, UIA or synthetic input. The OS foreground lock requires a real activation condition, such as the user activating NomiFun. No native focus effect was applied; acquire a fresh observation before a new call.";
#[cfg(windows)]
pub(crate) const WINDOWS_FOCUS_TARGET_MISSING: &str = "COMPUTER_FOCUS_NOT_APPLIED: The native target window no longer exists. No activation, restore, show, UIA or synthetic input was requested; acquire a fresh observation before a new call.";

#[cfg(windows)]
pub(crate) fn windows_focus_error_message(error: nomi_a11y::WindowsFocusError) -> String {
    match error {
        nomi_a11y::WindowsFocusError::TargetMissing => WINDOWS_FOCUS_TARGET_MISSING.into(),
        nomi_a11y::WindowsFocusError::NotApplied => WINDOWS_FOCUS_NOT_APPLIED.into(),
        error => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_empty_window_list() {
        assert_eq!(format_window_list(&[]), "No windows found.");
    }

    #[test]
    fn format_window_list_includes_fields() {
        let windows = vec![WindowInfo {
            id: 42,
            title: "Inbox".to_string(),
            app_name: "Mail".to_string(),
            x: 10,
            y: 20,
            width: 800,
            height: 600,
            is_focused: true,
        }];
        let text = format_window_list(&windows);
        assert!(text.contains("id=42"));
        assert!(text.contains("Mail"));
        assert!(text.contains("Inbox"));
        assert!(text.contains("800x600"));
        assert!(text.contains("[focused]"));
    }

    // Requires a real window server session.
    #[test]
    #[ignore]
    fn list_windows_real() {
        let windows = list_windows().expect("should enumerate windows");
        // There is at least a desktop-level window in a real session.
        assert!(!windows.is_empty());
    }
}
