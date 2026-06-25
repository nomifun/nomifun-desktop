//! macOS accessibility backend (AXUIElement).
//!
//! Threading model: AXUIElement / AXObserver have CFRunLoop / main-thread
//! affinity, so all AX calls are marshaled to a single dedicated actor thread
//! that owns a CFRunLoop and is the sole caller of the AX APIs. The public
//! `MacEngine` is a `Send + Sync` handle that sends commands to that actor and
//! blocks on a reply channel. Raw `AXUIElement` handles never cross the actor
//! boundary — only serializable `Snapshot` / `Effect` data does.
//!
//! Status: the actor scaffolding + capabilities are in place; the AX tree walk
//! and actuation are wired in `actor.rs` (see below). This module is compiled
//! only on macOS.

use crate::engine::{
    A11yEngine, A11yError, Capabilities, Effect, ElementAction, InputKind, ObserveOpts, Snapshot,
    SnapshotGen, Target,
};

pub struct MacEngine {
    inner: actor::ActorHandle,
}

impl MacEngine {
    pub fn start() -> Result<Self, A11yError> {
        let inner = actor::ActorHandle::spawn()?;
        Ok(Self { inner })
    }
}

impl A11yEngine for MacEngine {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            os: "macos".to_string(),
            tree_read: true,
            screenshot: true,
            semantic_action: true,
            synthetic_input: InputKind::Native,
            window_management: true,
        }
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

mod actor;
mod ocr;

pub use ocr::ocr_screenshot;
