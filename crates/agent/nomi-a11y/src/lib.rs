//! Cross-platform accessibility-tree + Set-of-Marks engine for Nomi computer-use.
//!
//! The platform-neutral layer (engine trait/types, selector grammar, tree
//! model + filtering, Set-of-Marks overlay) compiles on every target. Per-OS
//! backends live behind `#[cfg(target_os = …)]`:
//!   - macOS: AXUIElement via a dedicated CFRunLoop actor thread (implemented).
//!   - Windows: UI Automation via a dedicated MTA actor thread (implemented).
//!   - Linux: AT-SPI2 over D-Bus (implemented).
//!
//! Backends report honest `Capabilities`; unimplemented operations return
//! `A11yError::Unsupported { capability, hint }` (never panic, never a silent
//! no-op) so the agent can route around them.

pub mod engine;
pub mod overlay;
pub mod selector;
pub mod tree;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
mod linux;

pub use engine::{
    A11yEngine, A11yError, Capabilities, Effect, ElementAction, ElementEntry, ElementId,
    InputKind, ObserveOpts, OcrLine, Rect, Snapshot, SnapshotGen, Source, Target,
};

use std::sync::{Arc, RwLock};

/// Process-wide label for the host application, woven into permission-error
/// guidance so the message names the *actual* app the user must grant (and
/// restart) instead of a generic "this app". On a desktop host that ambiguity
/// is actively harmful: computer-use runs IN-PROCESS inside the host app, but a
/// model reading "this app" reliably misattributes it to the terminal/editor it
/// imagines is hosting the session and sends the user to grant the wrong
/// process. The host sets this once at startup (the desktop shell sets
/// "NomiFun"); library/headless embeddings leave it unset and get "this app".
static HOST_APP_LABEL: RwLock<Option<String>> = RwLock::new(None);

/// Set the host-application label used in permission-error guidance (e.g.
/// "NomiFun"). Last writer wins; call once early in host startup. A poisoned
/// lock is ignored — the default ("this app") is a safe fallback, never a panic.
pub fn set_host_app_label(label: impl Into<String>) {
    if let Ok(mut guard) = HOST_APP_LABEL.write() {
        *guard = Some(label.into());
    }
}

/// The host-application label for permission guidance, or `"this app"` when the
/// host has not set one. Always returns an owned, non-empty string.
pub fn host_app_label() -> String {
    HOST_APP_LABEL
        .read()
        .ok()
        .and_then(|g| g.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "this app".to_string())
}

/// Construct the platform's accessibility engine, or report why it is
/// unavailable. The returned engine is `Send + Sync` and its methods are
/// synchronous (call them from `spawn_blocking`); macOS marshals every call to
/// a single CFRunLoop actor thread internally.
pub fn create_engine() -> Result<Arc<dyn A11yEngine>, A11yError> {
    #[cfg(target_os = "macos")]
    {
        let engine = macos::MacEngine::start()?;
        Ok(Arc::new(engine))
    }
    #[cfg(target_os = "windows")]
    {
        let engine = windows::WinEngine::start()?;
        Ok(Arc::new(engine))
    }
    #[cfg(target_os = "linux")]
    {
        let engine = linux::LinuxEngine::start()?;
        Ok(Arc::new(engine))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Err(A11yError::Unsupported {
            capability: "accessibility engine".to_string(),
            hint: "The accessibility-tree backend is implemented on macOS, Windows, and Linux. \
                   Pixel-based computer-use still works."
                .to_string(),
        })
    }
}

#[cfg(test)]
mod host_label_tests {
    use super::{host_app_label, set_host_app_label};

    // The only test in this crate that touches the process-global label, so its
    // steps observe each other deterministically under the parallel runner.
    #[test]
    fn label_defaults_then_reflects_set_and_ignores_empty() {
        assert_eq!(host_app_label(), "this app", "default before any host sets it");
        set_host_app_label("NomiFun");
        assert_eq!(host_app_label(), "NomiFun");
        // An empty label is ignored so a mis-set never blanks the guidance.
        set_host_app_label("");
        assert_eq!(host_app_label(), "this app");
    }
}

/// Recognize on-screen text in a screenshot via the OS OCR engine (macOS:
/// Vision.framework `VNRecognizeTextRequest`, on-device, with CJK support).
/// `langs` are BCP-47 hints (e.g. `["zh-Hans", "en-US"]`). Bounds are in the
/// image's pixel space (top-left origin). Used to fuse text into Set-of-Marks
/// where the accessibility tree is thin. Returns `Unsupported` off macOS.
pub fn ocr_screenshot(img: &image::RgbaImage, langs: &[String]) -> Result<Vec<OcrLine>, A11yError> {
    #[cfg(target_os = "macos")]
    {
        macos::ocr_screenshot(img, langs)
    }
    #[cfg(target_os = "windows")]
    {
        windows::ocr_screenshot(img, langs)
    }
    #[cfg(target_os = "linux")]
    {
        linux::ocr_screenshot(img, langs)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (img, langs);
        Err(A11yError::Unsupported {
            capability: "OCR".to_string(),
            hint: "OCR fusion is implemented on macOS (Vision.framework) and Windows \
                   (Windows.Media.Ocr)."
                .to_string(),
        })
    }
}
