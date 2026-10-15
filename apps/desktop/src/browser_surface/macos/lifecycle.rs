//! Tauri's existing event thread owns WK/AppKit creation and destruction.
//! No browser framework is downloaded, preloaded, or separately pumped.
use std::{panic::{AssertUnwindSafe, catch_unwind}, sync::{Arc, Mutex}};
use nomifun_browser_macos::engine::Engine;
#[derive(Default)]
struct Gate { started: bool, closing: bool }
pub(crate) struct DeferredEngine {
    identity_policy: nomifun_browser_macos::BrowserIdentityPolicy,
    gate: Mutex<Gate>,
    result: tokio::sync::watch::Sender<Option<Result<Option<Arc<Engine>>, String>>>,
}
impl Default for DeferredEngine {
    fn default() -> Self {
        Self::new(nomifun_browser_macos::BrowserIdentityPolicy::default())
    }
}
impl DeferredEngine {
    pub(crate) fn new(identity_policy: nomifun_browser_macos::BrowserIdentityPolicy) -> Self {
        Self { identity_policy, gate: Mutex::new(Gate::default()), result: tokio::sync::watch::channel(None).0 }
    }
    pub(crate) async fn get(self: &Arc<Self>, _app: &tauri::AppHandle) -> Result<Arc<Engine>, String> {
        let begin = {
            let mut gate = self.gate.lock().unwrap();
            if gate.closing { return Err("WK host is closing".into()); }
            let begin = !gate.started;
            gate.started = true;
            begin
        };
        if begin {
            let owner = self.clone();
            // Tao pauses queued Task callbacks while an original callback is
            // inside a nested modal. Use the same public main-loop dispatcher
            // as native termination so shutdown can receive this exact reply.
            if let Err(error) = crate::macos_appkit_main_loop::post_to_main(move || {
                // The dispatcher contains callback panics, but containment
                // alone would leave settled() waiting forever. Publish an
                // explicit unverified error for both get and shutdown instead.
                let result = catch_unwind(AssertUnwindSafe(|| {
                    let closing = owner.gate.lock()
                        .map_err(|_| "WK initialization gate is poisoned; cleanup is unverified".to_owned())?.closing;
                    if closing { Ok(None) } else { Engine::initialize_with_identity_policy(owner.identity_policy).map(Some) }
                })).unwrap_or_else(|_| Err("WK initialization panicked; cleanup is unverified".to_owned()));
                owner.result.send_replace(Some(result));
            }) {
                self.result.send_replace(Some(Err(format!("WK main-thread dispatch failed: {error}"))));
            }
        }
        let engine = self.settled().await?.ok_or("WK host closed before initialization")?;
        if self.gate.lock().unwrap().closing { return Err("WK host is closing".into()); }
        Ok(engine)
    }
    async fn settled(&self) -> Result<Option<Arc<Engine>>, String> {
        let mut result = self.result.subscribe();
        loop {
            if let Some(value) = result.borrow_and_update().clone() { return value; }
            result.changed().await.map_err(|_| "WK initialization response was lost")?;
        }
    }
    pub(crate) async fn shutdown(&self) -> Result<(), String> {
        let started = { let mut gate = self.gate.lock().unwrap(); gate.closing = true; gate.started };
        if started { if let Some(engine) = self.settled().await? { engine.shutdown().await?; } }
        Ok(())
    }
}
