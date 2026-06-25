//! Linux AT-SPI2 backend.
//!
//! AT-SPI2 is a D-Bus protocol; the `atspi` crate is a pure-Rust (zbus) async
//! client. We mirror the macOS actor: a dedicated thread owns a current-thread
//! tokio runtime + the `AccessibilityConnection`, and the synchronous
//! `A11yEngine` methods `block_on` async AT-SPI calls via a command channel.
//!
//! Status: skeleton (reports honest capabilities; observe/invoke wired in
//! `actor.rs`). Compiled only on Linux.

use crate::engine::{
    A11yEngine, A11yError, Capabilities, Effect, ElementAction, ObserveOpts, OcrLine, Snapshot,
    SnapshotGen, Target,
};

mod actor;

pub struct LinuxEngine {
    inner: actor::ActorHandle,
}

impl LinuxEngine {
    pub fn start() -> Result<Self, A11yError> {
        let inner = actor::ActorHandle::spawn()?;
        Ok(Self { inner })
    }
}

impl A11yEngine for LinuxEngine {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn observe(&self, opts: &ObserveOpts) -> Result<Snapshot, A11yError> {
        self.inner.observe(opts.clone())
    }
    fn invoke(
        &self,
        target: &Target,
        generation: SnapshotGen,
        action: ElementAction,
    ) -> Result<Effect, A11yError> {
        self.inner.invoke(target.clone(), generation, action)
    }
    fn focus_window(&self, pid: i32) -> Result<Effect, A11yError> {
        self.inner.focus_window(pid)
    }
}

/// Linux has no OS-native OCR (unlike macOS Vision / Windows.Media.Ocr). The
/// tool layer handles this `Unsupported` gracefully (it just skips OCR fusion).
/// A `tesseract`-backed path could be added behind a cargo feature later.
pub fn ocr_screenshot(_img: &image::RgbaImage, _langs: &[String]) -> Result<Vec<OcrLine>, A11yError> {
    Err(A11yError::Unsupported {
        capability: "OCR".to_string(),
        hint: "Linux has no built-in OCR engine; accessibility-tree targeting still works, and \
               a11y-thin content falls back to pixel actions."
            .to_string(),
    })
}
