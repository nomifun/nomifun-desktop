//! Pre-backend desktop preparation. Projects the canonical supplier observations
//! into one thin startup surface; it never commits a JavaScript Runtime authority.
use nomifun_runtime::{ToolId, ToolPreparationSnapshot};
use serde::Serialize;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tauri::{Emitter, Manager};
use tokio::sync::Notify;

pub(crate) const EVENT: &str = "desktop-startup://state-changed";
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StartupStage {
    Preparing,
    Failed,
    StartingBackend,
    Ready,
    Quitting,
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct StartupSnapshot {
    pub revision: u64,
    pub stage: StartupStage,
    pub preparation: ToolPreparationSnapshot,
}

pub(crate) struct StartupState {
    snapshot: Mutex<StartupSnapshot>,
    retry: Notify,
    cancelled: Notify,
    quit: AtomicBool,
    main_loaded: AtomicBool,
}
impl StartupState {
    pub(crate) fn new() -> Self {
        Self {
            snapshot: Mutex::new(StartupSnapshot {
                revision: 0,
                stage: StartupStage::Preparing,
                preparation: nomifun_runtime::tool_preparation_snapshot(ToolId::Node),
            }),
            retry: Notify::new(),
            cancelled: Notify::new(),
            quit: AtomicBool::new(false),
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
        if !self.quit.load(Ordering::Acquire) || stage == StartupStage::Quitting {
            state.stage = stage;
            state.revision += 1;
        }
        state.clone()
    }
    fn progress(&self, next: ToolPreparationSnapshot) -> Option<StartupSnapshot> {
        let mut state = self
            .snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.stage != StartupStage::Preparing
            || self.quit.load(Ordering::Acquire)
            || next.revision <= state.preparation.revision
        {
            return None;
        }
        state.preparation = next;
        state.revision += 1;
        Some(state.clone())
    }
    fn failed(&self) -> StartupSnapshot {
        let preparation = nomifun_runtime::tool_preparation_snapshot(ToolId::Node);
        let mut state = self
            .snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.quit.load(Ordering::Acquire) {
            state.preparation = preparation;
            state.stage = StartupStage::Failed;
            state.revision += 1;
        }
        state.clone()
    }
    fn request_retry(&self) -> Result<StartupSnapshot, String> {
        let mut state = self
            .snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.stage != StartupStage::Failed
            || self.quit.load(Ordering::Acquire)
            || !state
                .preparation
                .error
                .as_ref()
                .is_some_and(|error| error.retryable)
        {
            return Err("This preparation cannot be retried in its current state.".into());
        }
        state.stage = StartupStage::Preparing;
        state.revision += 1;
        let next = state.clone();
        drop(state);
        self.retry.notify_one();
        Ok(next)
    }
    pub(crate) fn request_quit(&self) -> StartupSnapshot {
        self.quit.store(true, Ordering::Release);
        let snapshot = self.set_stage(StartupStage::Quitting);
        self.cancelled.notify_waiters();
        self.retry.notify_one();
        nomifun_runtime::cancel_tool_preparation(ToolId::Node);
        snapshot
    }
    async fn cancellation(&self) {
        let notified = self.cancelled.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.quit.load(Ordering::Acquire) {
            return;
        }
        notified.await;
    }
    pub(crate) fn preparing(&self) -> bool {
        matches!(
            self.snapshot().stage,
            StartupStage::Preparing | StartupStage::Failed
        )
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

/// This runs only on the existing backend worker. Setup/the event loop never
/// waits for networking. A failed preparation stays here, before any backend
/// services/database cleanup authority can be created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreparationOutcome {
    Prepared,
    Cancelled,
    CleanupUnproven,
}
fn cancelled_outcome(state: &StartupState) -> PreparationOutcome {
    if state
        .snapshot()
        .preparation
        .error
        .as_ref()
        .is_some_and(|error| error.code == "COMPONENT_CLEANUP_UNPROVEN")
    {
        PreparationOutcome::CleanupUnproven
    } else {
        PreparationOutcome::Cancelled
    }
}
pub(crate) async fn prepare_node(
    app: tauri::AppHandle,
    state: Arc<StartupState>,
) -> PreparationOutcome {
    loop {
        if state.quit.load(Ordering::Acquire) {
            return cancelled_outcome(&state);
        }
        emit(&app, state.set_stage(StartupStage::Preparing));
        let mut observations = nomifun_runtime::subscribe_tool_preparation(ToolId::Node);
        let observing_state = state.clone();
        let observing_app = app.clone();
        let observer = tokio::spawn(async move {
            loop {
                let progress = observations.borrow_and_update().clone();
                if let Some(next) = observing_state.progress(progress) {
                    emit(&observing_app, next);
                }
                if observations.changed().await.is_err() {
                    break;
                }
            }
        });
        let preparation = nomifun_runtime::prepare_tool(ToolId::Node);
        tokio::pin!(preparation);
        // Poll preparation first so a producer exists before a concurrent quit
        // cancels it. Await its terminal cleanup rather than dropping ownership.
        let result = tokio::select! {biased;
            result=&mut preparation=>result,
            _=state.cancellation()=>{
                nomifun_runtime::cancel_tool_preparation(ToolId::Node);
                (&mut preparation).await
            }
        };
        observer.abort();
        let _ = observer.await;
        if result.is_err() {
            // Read the canonical terminal observation before treating quit as
            // verified. Shared(Cleanup) must remain an unproven handoff.
            let latest = nomifun_runtime::tool_preparation_snapshot(ToolId::Node);
            let terminal = {
                let mut current = state
                    .snapshot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                current.preparation = latest;
                current.revision += 1;
                current.clone()
            };
            emit(&app, terminal);
        }
        if state.quit.load(Ordering::Acquire) {
            return cancelled_outcome(&state);
        }
        match result {
            Ok(_) => {
                if let Some(next) =
                    state.progress(nomifun_runtime::tool_preparation_snapshot(ToolId::Node))
                {
                    emit(&app, next);
                }
                emit(&app, state.set_stage(StartupStage::StartingBackend));
                return if state.quit.load(Ordering::Acquire) {
                    cancelled_outcome(&state)
                } else {
                    PreparationOutcome::Prepared
                };
            }
            Err(error) => {
                tracing::warn!(%error,"desktop startup component preparation failed");
                emit(&app, state.failed());
                let retry = state.retry.notified();
                if state.quit.load(Ordering::Acquire) {
                    return cancelled_outcome(&state);
                }
                retry.await;
            }
        }
    }
}

#[tauri::command]
pub(crate) fn desktop_startup_status(
    state: tauri::State<'_, Arc<StartupState>>,
) -> StartupSnapshot {
    state.snapshot()
}
#[tauri::command]
pub(crate) fn desktop_startup_retry(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<StartupState>>,
) -> Result<StartupSnapshot, String> {
    let next = state.request_retry()?;
    emit(&app, next.clone());
    Ok(next)
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
    use nomifun_runtime::{ToolPreparationFailure, ToolPreparationPhase};
    fn state_with_failure() -> StartupState {
        let state = StartupState::new();
        {
            let mut current = state.snapshot.lock().unwrap();
            current.stage = StartupStage::Failed;
            current.preparation.error = Some(ToolPreparationFailure {
                code: "DOWNLOAD_FAILED".into(),
                message: "offline".into(),
                retryable: true,
            });
        }
        state
    }
    #[test]
    fn retry_is_single_flight_and_not_a_runtime_selection() {
        let state = state_with_failure();
        assert!(state.request_retry().is_ok());
        assert!(state.request_retry().is_err());
        assert_eq!(state.snapshot().preparation.tool, ToolId::Node);
        state.set_stage(StartupStage::StartingBackend);
        assert!(state.request_retry().is_err());
    }
    #[test]
    fn old_supplier_events_cannot_undo_failure_backend_or_quit() {
        let state = state_with_failure();
        let mut progress = state.snapshot().preparation;
        progress.revision += 1;
        progress.phase = ToolPreparationPhase::Downloading;
        assert!(state.progress(progress.clone()).is_none());
        state.request_retry().unwrap();
        assert!(state.progress(progress.clone()).is_some());
        assert!(state.progress(progress.clone()).is_none());
        state.set_stage(StartupStage::StartingBackend);
        progress.revision += 1;
        assert!(state.progress(progress.clone()).is_none());
        state.quit.store(true, Ordering::Release);
        state.set_stage(StartupStage::Quitting);
        state.set_stage(StartupStage::Ready);
        assert_eq!(state.snapshot().stage, StartupStage::Quitting);
    }
    #[test]
    fn unproven_component_cleanup_cannot_be_reported_as_a_clean_cancellation() {
        let state = state_with_failure();
        {
            let mut current = state.snapshot.lock().unwrap();
            current.preparation.error = Some(ToolPreparationFailure {
                code: "COMPONENT_CLEANUP_UNPROVEN".into(),
                message: "cleanup".into(),
                retryable: false,
            });
        }
        assert!(state.request_retry().is_err());
        state.quit.store(true, Ordering::Release);
        assert_eq!(
            cancelled_outcome(&state),
            PreparationOutcome::CleanupUnproven
        );
        {
            state.snapshot.lock().unwrap().preparation.error = Some(ToolPreparationFailure {
                code: "PREPARATION_CANCELLED".into(),
                message: "cancelled".into(),
                retryable: true,
            });
        }
        assert_eq!(cancelled_outcome(&state), PreparationOutcome::Cancelled);
    }

    #[tokio::test]
    async fn cancellation_before_wait_and_retry_before_wait_are_not_lost() {
        let state = state_with_failure();
        state.request_retry().unwrap();
        tokio::time::timeout(std::time::Duration::from_millis(50), state.retry.notified())
            .await
            .unwrap();
        state.quit.store(true, Ordering::Release);
        tokio::time::timeout(std::time::Duration::from_millis(50), state.cancellation())
            .await
            .unwrap();
    }
}
