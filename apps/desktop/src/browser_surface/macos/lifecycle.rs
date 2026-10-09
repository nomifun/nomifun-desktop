//! Tauri's existing event thread owns WK/AppKit creation and destruction.
//! No browser framework is downloaded, preloaded, or separately pumped.
use std::sync::{Arc, Mutex};
use nomifun_browser_macos::engine::Engine;
#[derive(Default)]
struct Gate { started: bool, closing: bool }
pub(crate) struct DeferredEngine {
    gate: Mutex<Gate>,
    result: tokio::sync::watch::Sender<Option<Result<Option<Arc<Engine>>, String>>>,
}
impl Default for DeferredEngine {
    fn default() -> Self {
        Self { gate: Mutex::new(Gate::default()), result: tokio::sync::watch::channel(None).0 }
    }
}
impl DeferredEngine {
    pub(crate) async fn get(self: &Arc<Self>, app: &tauri::AppHandle) -> Result<Arc<Engine>, String> {
        let begin = {
            let mut gate = self.gate.lock().unwrap();
            if gate.closing { return Err("WK host is closing".into()); }
            let begin = !gate.started;
            gate.started = true;
            begin
        };
        if begin {
            let owner = self.clone();
            if let Err(error) = app.run_on_main_thread(move || {
                let closing = owner.gate.lock().unwrap().closing;
                owner.result.send_replace(Some(if closing { Ok(None) } else { Engine::initialize().map(Some) }));
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
