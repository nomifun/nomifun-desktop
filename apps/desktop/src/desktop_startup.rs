//! Lightweight desktop loading surface while the embedded backend starts.
//! Optional external runtimes are resolved by the features that use them.
use serde::Serialize;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tauri::{Emitter, Manager};

pub(crate) const EVENT: &str = "desktop-startup://state-changed";
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StartupStage {
    StartingBackend,
    Ready,
    Quitting,
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct StartupSnapshot {
    pub revision: u64,
    pub stage: StartupStage,
}

pub(crate) struct StartupState {
    snapshot: Mutex<StartupSnapshot>,
    main_loaded: AtomicBool,
}
impl StartupState {
    pub(crate) fn new() -> Self {
        Self {
            snapshot: Mutex::new(StartupSnapshot {
                revision: 0,
                stage: StartupStage::StartingBackend,
            }),
            main_loaded: AtomicBool::new(false),
        }
    }
    pub(crate) fn snapshot(&self) -> StartupSnapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
    fn set_stage(&self, stage: StartupStage) -> StartupSnapshot {
        let mut state = self
            .snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.stage != StartupStage::Quitting || stage == StartupStage::Quitting {
            state.stage = stage;
            state.revision += 1;
        }
        state.clone()
    }
    pub(crate) fn request_quit(&self) -> StartupSnapshot {
        self.set_stage(StartupStage::Quitting)
    }
    pub(crate) fn starting(&self) -> bool {
        self.snapshot().stage == StartupStage::StartingBackend
    }
}
fn emit(app: &tauri::AppHandle, snapshot: StartupSnapshot) {
    let _ = app.emit(EVENT, snapshot);
}
pub(crate) fn begin_quit(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<Arc<StartupState>>() {
        emit(app, state.request_quit());
    }
}

#[tauri::command]
pub(crate) fn desktop_startup_status(
    state: tauri::State<'_, Arc<StartupState>>,
) -> StartupSnapshot {
    state.snapshot()
}
#[tauri::command]
pub(crate) fn desktop_startup_quit(app: tauri::AppHandle) {
    begin_quit(&app);
    app.exit(0);
}

pub(crate) fn install_window(app: &tauri::AppHandle) -> anyhow::Result<()> {
    let mut window = tauri::WebviewWindowBuilder::new(
        app,
        "startup",
        tauri::WebviewUrl::App("startup.html".into()),
    )
    .title("NomiFun")
    .inner_size(1280.0, 832.0)
    .min_inner_size(880.0, 600.0)
    .initialization_script("window.isTauri = true;");
    #[cfg(target_os = "macos")]
    {
        if let Some(store) = app.try_state::<crate::MacosWebviewDataStore>() {
            if let Some(identifier) = store.0 {
                window = window.data_store_identifier(identifier);
            }
        }
    }
    window.build()?;
    Ok(())
}
pub(crate) fn main_loaded(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<Arc<StartupState>>() {
        state.main_loaded.store(true, Ordering::Release);
    }
    handoff(app);
}
pub(crate) fn backend_ready(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<Arc<StartupState>>() {
        emit(app, state.set_stage(StartupStage::Ready));
    }
    handoff(app);
}
fn handoff(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<Arc<StartupState>>() else {
        return;
    };
    if state.snapshot().stage != StartupStage::Ready || !state.main_loaded.load(Ordering::Acquire) {
        return;
    }
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
    }
    if let Some(startup) = app.get_webview_window("startup") {
        let _ = startup.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_tracks_backend_readiness_without_external_runtime_preparation() {
        let state = StartupState::new();
        assert!(state.starting());
        assert_eq!(state.snapshot().revision, 0);
        let ready = state.set_stage(StartupStage::Ready);
        assert_eq!(ready.stage, StartupStage::Ready);
        assert_eq!(ready.revision, 1);
        assert!(!state.starting());
    }
    #[test]
    fn late_backend_readiness_cannot_undo_quit() {
        let state = StartupState::new();
        let quitting = state.request_quit();
        state.set_stage(StartupStage::Ready);
        assert_eq!(state.snapshot().stage, StartupStage::Quitting);
        assert_eq!(state.snapshot().revision, quitting.revision);
    }
}
