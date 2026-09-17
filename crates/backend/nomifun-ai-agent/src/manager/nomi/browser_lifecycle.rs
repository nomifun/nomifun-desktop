//! Provider-neutral Browser Resource lifecycle for one Nomi turn.

use nomifun_browser_platform::{
    attached_browser::AuthorizedAttachedBrowserTurn,
    bound_resource::BoundBrowserProviderResource,
    run_guard::BrowserRunGuard,
    workspace::BrowserResource,
};
use nomifun_common::AppError;
use std::sync::{Arc, Mutex};

pub(super) struct ManagedBrowserTurn {
    resource: Arc<BrowserResource>,
    guard: BrowserRunGuard,
}

#[derive(Clone)]
pub(super) enum BrowserTurn {
    Managed(Arc<ManagedBrowserTurn>),
    AttachedChrome(Arc<AuthorizedAttachedBrowserTurn>),
}

#[derive(Clone, Default)]
pub(super) struct BrowserTurnSlot(Arc<Mutex<Option<BrowserTurn>>>);

fn error(value: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("Browser Resource lifecycle failed: {value}"))
}

impl BrowserTurnSlot {
    pub(super) fn current(&self) -> Result<BrowserTurn, AppError> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .ok_or_else(|| AppError::Conflict("The Browser turn is no longer active.".into()))
    }

    pub async fn begin(&self, resource: &BoundBrowserProviderResource) -> Result<(), AppError> {
        if self.0.lock().unwrap_or_else(|error| error.into_inner()).is_some() {
            return Err(AppError::Conflict(
                "A prior Browser run has not finished cleanup.".into(),
            ));
        }
        let turn = match resource {
            BoundBrowserProviderResource::Managed(resource) => {
                let guard = resource.begin_run().await.map_err(error)?;
                guard.require_explicit_finish();
                BrowserTurn::Managed(Arc::new(ManagedBrowserTurn {
                    resource: Arc::clone(resource),
                    guard,
                }))
            }
            BoundBrowserProviderResource::AttachedChrome(resource) => {
                BrowserTurn::AttachedChrome(resource.begin_run().await.map_err(error)?)
            }
        };
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = Some(turn);
        Ok(())
    }

    pub fn cancel(&self) {
        if let Some(turn) = self
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
        {
            match turn {
                BrowserTurn::Managed(turn) => turn.guard.cancel(),
                BrowserTurn::AttachedChrome(turn) => turn.cancel(),
            }
        }
    }

    pub async fn settle(&self) -> Result<(), AppError> {
        let turn = self
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        match turn {
            Some(BrowserTurn::Managed(turn)) => {
                turn.resource.settle_run(&turn.guard).await.map_err(error)
            }
            Some(BrowserTurn::AttachedChrome(turn)) => turn.settle().await.map_err(error),
            None => Ok(()),
        }
    }

    pub async fn finish(&self) -> Result<(), AppError> {
        let turn = self
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let Some(turn) = turn else { return Ok(()); };
        match &turn {
            BrowserTurn::Managed(turn) => {
                turn.resource.finish_run(&turn.guard).await.map_err(error)?;
            }
            BrowserTurn::AttachedChrome(turn) => turn.finish().await.map_err(error)?,
        }
        let mut slot = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let same = match (slot.as_ref(), &turn) {
            (Some(BrowserTurn::Managed(active)), BrowserTurn::Managed(completed)) => {
                Arc::ptr_eq(active, completed)
            }
            (
                Some(BrowserTurn::AttachedChrome(active)),
                BrowserTurn::AttachedChrome(completed),
            ) => Arc::ptr_eq(active, completed),
            _ => false,
        };
        if same {
            slot.take();
        }
        Ok(())
    }
}

impl ManagedBrowserTurn {
    pub async fn evaluate(
        &self,
        request: nomifun_browser_platform::runtime::BrowserEvaluation,
    ) -> Result<
        nomifun_browser_platform::runtime::BrowserEvaluationResult,
        nomifun_browser_platform::runtime::WorkspaceError,
    > {
        self.resource.evaluate(&self.guard, request).await
    }

    pub async fn download(
        &self,
        element: nomifun_browser_platform::runtime::BrowserElementRef,
        scope: Arc<nomifun_browser_platform::downloads::BrowserDownloadScope>,
    ) -> Result<
        nomifun_browser_platform::runtime::BrowserActionResult,
        nomifun_browser_platform::runtime::WorkspaceError,
    > {
        self.resource.download(&self.guard, element, scope).await
    }

    pub async fn respond_dialog(
        &self,
        reply: nomifun_browser_platform::runtime::BrowserDialogReply,
    ) -> Result<
        nomifun_browser_platform::runtime::BrowserActionResult,
        nomifun_browser_platform::runtime::WorkspaceError,
    > {
        self.resource.respond_dialog(&self.guard, reply).await
    }

    pub async fn upload(
        &self,
        element: nomifun_browser_platform::runtime::BrowserElementRef,
        scope: Arc<nomifun_browser_platform::uploads::BrowserUploadScope>,
        paths: Vec<String>,
    ) -> Result<
        nomifun_browser_platform::runtime::BrowserActionResult,
        nomifun_browser_platform::runtime::WorkspaceError,
    > {
        self.resource.upload(&self.guard, element, scope, paths).await
    }

    pub async fn screenshot(
        &self,
        tab_id: Option<String>,
    ) -> Result<
        nomifun_browser_platform::runtime::BrowserScreenshot,
        nomifun_browser_platform::runtime::WorkspaceError,
    > {
        self.resource.screenshot(&self.guard, tab_id).await
    }

    pub async fn observe(
        &self,
        tab_id: Option<String>,
    ) -> Result<
        nomifun_browser_platform::runtime::BrowserObservation,
        nomifun_browser_platform::runtime::WorkspaceError,
    > {
        self.resource.observe(&self.guard, tab_id).await
    }

    pub async fn act(
        &self,
        action: nomifun_browser_platform::runtime::BrowserAction,
    ) -> Result<
        nomifun_browser_platform::runtime::BrowserActionResult,
        nomifun_browser_platform::runtime::WorkspaceError,
    > {
        self.resource.act(&self.guard, action).await
    }

    pub async fn command(
        &self,
        command: nomifun_browser_platform::runtime::BrowserTabCommand,
    ) -> Result<
        nomifun_browser_platform::runtime::BrowserRuntimeSnapshot,
        nomifun_browser_platform::runtime::WorkspaceError,
    > {
        self.resource.agent_command(&self.guard, command).await
    }

    pub async fn tabs(
        &self,
    ) -> Result<
        Option<nomifun_browser_platform::runtime::BrowserRuntimeSnapshot>,
        nomifun_browser_platform::runtime::WorkspaceError,
    > {
        Ok(self.resource.agent_snapshot(&self.guard).await?.runtime)
    }
}
