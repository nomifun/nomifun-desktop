//! Windows accessibility backend (UI Automation).
//!
//! Threading model (mirrors the macOS backend): every `IUIAutomation` /
//! `IUIAutomationElement` call has COM apartment affinity, so all UIA work is
//! marshaled to a single dedicated actor thread that initializes COM as MTA
//! (`CoInitializeEx(COINIT_MULTITHREADED)`) and is the sole owner of the
//! `UIAutomation` instance and every element handle. The public `WinEngine` is
//! a `Send + Sync` handle that sends commands over a channel and blocks on a
//! per-command reply; raw UIA element handles never cross the actor boundary —
//! only serializable `Snapshot` / `Effect` data does (so `WinEngine` is
//! `Send + Sync` automatically, no `unsafe impl` needed).
//!
//! OCR (`Windows.Media.Ocr`) has no apartment affinity and runs on whatever
//! thread the caller uses (the computer tool calls it from `spawn_blocking`).

use crate::engine::{
    A11yEngine, A11yError, Capabilities, Effect, ElementAction, InputKind, ObserveOpts, Snapshot,
    SnapshotGen, Target,
};

mod actor;
mod ocr;
mod tree_map;

pub use ocr::ocr_screenshot;

pub struct WinEngine {
    inner: actor::ActorHandle,
}

impl WinEngine {
    pub fn start() -> Result<Self, A11yError> {
        let inner = actor::ActorHandle::spawn()?;
        Ok(Self { inner })
    }
}

impl A11yEngine for WinEngine {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            os: "windows".to_string(),
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
