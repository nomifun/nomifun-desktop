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
    A11yEngine, A11yError, Capabilities, Effect, ElementAction, ObserveOpts, Snapshot,
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
