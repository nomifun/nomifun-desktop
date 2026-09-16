//! The Nomi turn owner coordinates native input with its authoritative terminal.

use nomifun_browser_platform::{run_guard::BrowserRunGuard, workspace::BrowserWorkspace};
use nomifun_common::AppError;
use std::sync::{Arc, Mutex};

pub(super) struct NativeBrowserTurn {
    workspace: Arc<BrowserWorkspace>,
    guard: BrowserRunGuard,
}

#[derive(Clone, Default)]
pub(super) struct NativeBrowserTurnSlot(Arc<Mutex<Option<Arc<NativeBrowserTurn>>>>);

fn error(value: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("Native browser lifecycle failed: {value}"))
}

pub(super) async fn settle_turns(native: &NativeBrowserTurnSlot, system: &crate::system_browser::SystemBrowserTurnSlot) -> Result<(), AppError> {
    let native = native.settle().await;
    let system = system.settle().await;
    native.and(system)
}

pub(super) async fn finish_turns(native: &NativeBrowserTurnSlot, system: &crate::system_browser::SystemBrowserTurnSlot) -> Result<(), AppError> {
    let native = native.finish().await;
    let system = system.finish().await;
    native.and(system)
}

impl NativeBrowserTurnSlot {
    pub(super) fn current(
        &self,
    ) -> Result<Arc<NativeBrowserTurn>, nomifun_browser_platform::runtime::WorkspaceError> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .ok_or(nomifun_browser_platform::run_guard::RunAdmissionError::StaleRun.into())
    }
}

impl NativeBrowserTurn {
    pub async fn evaluate(&self, request: nomifun_browser_platform::runtime::BrowserEvaluation)
        -> Result<nomifun_browser_platform::runtime::BrowserEvaluationResult, nomifun_browser_platform::runtime::WorkspaceError> {
        self.workspace.evaluate(&self.guard, request).await
    }
    pub async fn download(&self, element: nomifun_browser_platform::runtime::BrowserElementRef, scope: Arc<nomifun_browser_platform::downloads::BrowserDownloadScope>) -> Result<nomifun_browser_platform::runtime::BrowserActionResult, nomifun_browser_platform::runtime::WorkspaceError> {
        self.workspace.download(&self.guard, element, scope).await
    }
    pub async fn respond_dialog(&self, reply: nomifun_browser_platform::runtime::BrowserDialogReply)
        -> Result<nomifun_browser_platform::runtime::BrowserActionResult, nomifun_browser_platform::runtime::WorkspaceError> {
        self.workspace.respond_dialog(&self.guard, reply).await
    }
    pub async fn upload(&self,element:nomifun_browser_platform::runtime::BrowserElementRef,scope:Arc<nomifun_browser_platform::uploads::BrowserUploadScope>,paths:Vec<String>) -> Result<nomifun_browser_platform::runtime::BrowserActionResult,nomifun_browser_platform::runtime::WorkspaceError> {
        self.workspace.upload(&self.guard,element,scope,paths).await
    }
    pub async fn screenshot(&self, tab_id: Option<String>) -> Result<nomifun_browser_platform::runtime::BrowserScreenshot, nomifun_browser_platform::runtime::WorkspaceError> {
        self.workspace.screenshot(&self.guard, tab_id).await
    }

    pub async fn observe(
        &self,
        tab_id: Option<String>,
    ) -> Result<
        nomifun_browser_platform::runtime::BrowserObservation,
        nomifun_browser_platform::runtime::WorkspaceError,
    > {
        self.workspace.observe(&self.guard, tab_id).await
    }

    pub async fn act(
        &self,
        action: nomifun_browser_platform::runtime::BrowserAction,
    ) -> Result<
        nomifun_browser_platform::runtime::BrowserActionResult,
        nomifun_browser_platform::runtime::WorkspaceError,
    > {
        self.workspace.act(&self.guard, action).await
    }

    pub async fn command(
        &self,
        command: nomifun_browser_platform::runtime::BrowserTabCommand,
    ) -> Result<
        nomifun_browser_platform::runtime::BrowserRuntimeSnapshot,
        nomifun_browser_platform::runtime::WorkspaceError,
    > {
        self.workspace.agent_command(&self.guard, command).await
    }

    pub async fn tabs(
        &self,
    ) -> Result<
        Option<nomifun_browser_platform::runtime::BrowserRuntimeSnapshot>,
        nomifun_browser_platform::runtime::WorkspaceError,
    > {
        Ok(self.workspace.agent_snapshot(&self.guard).await?.runtime)
    }
}

impl NativeBrowserTurnSlot {
    pub async fn begin(&self, workspace: Arc<BrowserWorkspace>) -> Result<(), AppError> {
        if self.0.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
            return Err(AppError::Conflict(
                "A prior browser run has not finished cleanup.".into(),
            ));
        }
        let guard = workspace.begin_run().await.map_err(error)?;
        guard.require_explicit_finish();
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(Arc::new(NativeBrowserTurn { workspace, guard }));
        Ok(())
    }

    pub fn cancel(&self) {
        if let Some(turn) = self.0.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            turn.guard.cancel();
        }
    }

    pub async fn settle(&self) -> Result<(), AppError> {
        let turn = self.0.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(turn) = turn {
            turn.workspace
                .settle_run(&turn.guard)
                .await
                .map_err(error)?;
        }
        Ok(())
    }

    pub async fn finish(&self) -> Result<(), AppError> {
        let turn = self.0.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(turn) = turn {
            turn.workspace
                .finish_run(&turn.guard)
                .await
                .map_err(error)?;
            let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if slot
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, &turn))
            {
                slot.take();
            }
        }
        Ok(())
    }
}
