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

use std::sync::Arc;

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
