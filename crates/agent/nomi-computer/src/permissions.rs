//! Best-effort OS permission diagnostics for screen capture and input.
//!
//! macOS gates screen capture behind "Screen Recording" and input synthesis
//! behind "Accessibility". This module provides PROACTIVE status probing and
//! prompting (via TCC APIs) plus the legacy reactive hints/heuristics as
//! corroborating fallbacks. On non-macOS platforms the status calls report
//! "unknown" (`None`) and requests are no-ops.

/// A point-in-time snapshot of the two TCC permissions computer-use needs.
/// `None` for a field means the platform cannot report it (non-macOS).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionStatus {
    pub accessibility: Option<bool>,
    pub screen_recording: Option<bool>,
}

/// Live Accessibility (input synthesis / future a11y-tree) grant state.
/// `None` where the platform has no such gate.
pub fn accessibility_granted() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        Some(macos_tcc::accessibility_trusted())
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Live Screen Recording (screenshot) grant state. `None` off macOS.
pub fn screen_recording_granted() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        Some(macos_tcc::screen_capture_granted())
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Snapshot both permissions at once (for a host/UI permission panel).
pub fn permission_status() -> PermissionStatus {
    PermissionStatus {
        accessibility: accessibility_granted(),
        screen_recording: screen_recording_granted(),
    }
}

/// Trigger the OS Accessibility prompt (macOS shows the system dialog and the
/// Settings deep-link). Returns the post-call grant state. No-op → `true`
/// where the platform has no such gate.
pub fn request_accessibility() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos_tcc::request_accessibility()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Trigger the OS Screen Recording prompt (macOS). Returns the post-call grant
/// state. Note: macOS caches the grant per process, so a freshly-granted
/// permission typically requires an app relaunch to take effect.
pub fn request_screen_recording() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos_tcc::request_screen_capture()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

fn live_status_note(granted: Option<bool>, name: &str) -> String {
    match granted {
        Some(false) => format!(" (checked just now: {name} is NOT granted)"),
        Some(true) => format!(" (checked just now: {name} is granted)"),
        None => String::new(),
    }
}

/// `screen_capture_hint()` plus the live grant state when known.
pub fn screen_capture_hint_detailed() -> String {
    format!(
        "{}{}",
        screen_capture_hint(),
        live_status_note(screen_recording_granted(), "Screen Recording")
    )
}

/// `accessibility_hint()` plus the live grant state when known.
pub fn accessibility_hint_detailed() -> String {
    format!(
        "{}{}",
        accessibility_hint(),
        live_status_note(accessibility_granted(), "Accessibility")
    )
}

/// Guidance appended to screen-capture failures.
pub fn screen_capture_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "If this keeps failing, grant Screen Recording permission in \
         System Settings → Privacy & Security → Screen Recording, then restart this app."
    } else {
        "Check that a display is connected and the app is allowed to capture the screen."
    }
}

/// Guidance appended to input-synthesis failures.
pub fn accessibility_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "If this keeps failing, grant Accessibility permission in \
         System Settings → Privacy & Security → Accessibility, then restart this app."
    } else {
        "Check that the app is allowed to control the mouse and keyboard."
    }
}

/// Verify a captured frame is usable. macOS without Screen Recording can
/// "succeed" but return an all-black frame; flag that. The authoritative TCC
/// preflight lives at the capture call site (`screen::capture_screen`); this
/// stays a pure, environment-independent heuristic so it remains unit-testable
/// and also catches edge cases where the preflight and the real capture
/// disagree (e.g. the macOS 26 "responsible process" drift). Always Ok off
/// macOS.
pub fn screenshot_permission_check(img: &image::RgbaImage) -> Result<(), String> {
    if cfg!(target_os = "macos") && looks_all_black(img) {
        return Err(format!(
            "Screenshot came back entirely black, which usually means the \
             Screen Recording permission is missing or stale. {}",
            screen_capture_hint_detailed()
        ));
    }
    Ok(())
}

/// macOS TCC FFI: proactive Accessibility / Screen Recording status + prompt.
#[cfg(target_os = "macos")]
mod macos_tcc {
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::string::{CFString, CFStringRef};

    // HIServices (umbrella: ApplicationServices). `Boolean` is `unsigned char`.
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> u8;
        fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> u8;
        static kAXTrustedCheckOptionPrompt: CFStringRef;
    }

    // CoreGraphics screen-capture access (C `bool`).
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }

    pub fn accessibility_trusted() -> bool {
        unsafe { AXIsProcessTrusted() != 0 }
    }

    /// Probe Accessibility AND show the system prompt + Settings deep-link when
    /// not yet granted.
    pub fn request_accessibility() -> bool {
        unsafe {
            let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
            let opts = CFDictionary::from_CFType_pairs(&[(
                key.as_CFType(),
                CFBoolean::true_value().as_CFType(),
            )]);
            AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef()) != 0
        }
    }

    pub fn screen_capture_granted() -> bool {
        unsafe { CGPreflightScreenCaptureAccess() }
    }

    pub fn request_screen_capture() -> bool {
        unsafe { CGRequestScreenCaptureAccess() }
    }
}

/// True if every sampled pixel is (near-)black. Samples a grid rather than
/// every pixel to keep this cheap on Retina-sized captures.
pub fn looks_all_black(img: &image::RgbaImage) -> bool {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return true;
    }
    let step_x = (w / 64).max(1);
    let step_y = (h / 64).max(1);
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            let p = img.get_pixel(x, y);
            if p[0] > 2 || p[1] > 2 || p[2] > 2 {
                return false;
            }
            x += step_x;
        }
        y += step_y;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn all_black_image_detected() {
        let img = RgbaImage::from_pixel(64, 64, Rgba([0, 0, 0, 255]));
        assert!(looks_all_black(&img));
    }

    #[test]
    fn near_black_noise_still_counts_as_black() {
        let img = RgbaImage::from_pixel(64, 64, Rgba([1, 2, 1, 255]));
        assert!(looks_all_black(&img));
    }

    #[test]
    fn single_bright_pixel_is_not_black() {
        let mut img = RgbaImage::from_pixel(64, 64, Rgba([0, 0, 0, 255]));
        // Place it on the sampling grid origin so the sparse scan sees it.
        img.put_pixel(0, 0, Rgba([255, 255, 255, 255]));
        assert!(!looks_all_black(&img));
    }

    #[test]
    fn empty_image_counts_as_black() {
        let img = RgbaImage::new(0, 0);
        assert!(looks_all_black(&img));
    }

    #[test]
    fn permission_check_passes_on_normal_image() {
        let img = RgbaImage::from_pixel(8, 8, Rgba([120, 40, 200, 255]));
        assert!(screenshot_permission_check(&img).is_ok());
    }

    #[test]
    fn hints_are_nonempty() {
        assert!(!screen_capture_hint().is_empty());
        assert!(!accessibility_hint().is_empty());
        // Detailed variants embed live status and must still be non-empty.
        assert!(!screen_capture_hint_detailed().is_empty());
        assert!(!accessibility_hint_detailed().is_empty());
    }

    #[test]
    fn permission_status_does_not_panic_and_is_consistent() {
        // Calls the real TCC APIs on macOS (read-only, safe); no-op elsewhere.
        let s = permission_status();
        assert_eq!(s.accessibility, accessibility_granted());
        assert_eq!(s.screen_recording, screen_recording_granted());
        #[cfg(target_os = "macos")]
        {
            assert!(s.accessibility.is_some());
            assert!(s.screen_recording.is_some());
        }
        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(s.accessibility, None);
            assert_eq!(s.screen_recording, None);
        }
    }
}
