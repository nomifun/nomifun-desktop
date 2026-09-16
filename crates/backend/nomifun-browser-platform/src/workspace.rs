//! Conversation lifetime authority. Agent turns borrow the same persistent runtime.

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{
    run_guard::{
        BrowserRunCoordinator, BrowserRunGuard, BrowserRunSnapshot, NativeInputGate,
        RunAdmissionError,
    },
    runtime::{
        BrowserProfile, BrowserRuntime, BrowserRuntimeFactory, BrowserRuntimeSnapshot,
        BrowserTabCommand, BrowserWorkspaceKey, CreateBrowserRuntime, WorkspaceError,
    },
};

/// Exists before a native runtime: accepting an Agent run locks future tabs too.
struct RuntimeSlot {
    factory: Arc<dyn BrowserRuntimeFactory>,
    request: CreateBrowserRuntime,
    inner: Mutex<SlotState>,
}

struct SlotState {
    runtime: Option<Arc<dyn BrowserRuntime>>,
    locked: bool,
    closed: bool,
}

impl RuntimeSlot {
    async fn ensure(&self) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
        let mut state = self.inner.lock().await;
        if state.closed {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        if let Some(runtime) = &state.runtime {
            return Ok(runtime.clone());
        }
        let mut request = self.request.clone();
        request.user_input_enabled = !state.locked;
        let runtime = self.factory.create(request).await?;
        state.runtime = Some(runtime.clone());
        Ok(runtime)
    }

    async fn close(&self) -> Result<(), WorkspaceError> {
        let mut state = self.inner.lock().await;
        state.closed = true;
        if let Some(runtime) = &state.runtime {
            // On error retain the native runtime so explicit shutdown can retry.
            runtime.close().await?;
        }
        state.runtime = None;
        Ok(())
    }
}

#[async_trait]
impl NativeInputGate for RuntimeSlot {
    async fn lock_user_input(&self) -> Result<(), RunAdmissionError> {
        let mut state = self.inner.lock().await;
        state.locked = true;
        if let Some(runtime) = &state.runtime {
            runtime.lock_user_input().await?;
        }
        Ok(())
    }
    async fn release_pressed_input(&self) -> Result<(), RunAdmissionError> {
        let state = self.inner.lock().await;
        if let Some(runtime) = &state.runtime {
            runtime.release_pressed_input().await?;
        }
        Ok(())
    }
    async fn unlock_user_input(&self) -> Result<(), RunAdmissionError> {
        let mut state = self.inner.lock().await;
        if !state.closed {
            if let Some(runtime) = &state.runtime {
                runtime.unlock_user_input().await?;
            }
        }
        state.locked = false;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BrowserWorkspaceSnapshot {
    pub conversation_id: String,
    pub run: BrowserRunSnapshot,
    pub runtime: Option<BrowserRuntimeSnapshot>,
}

pub struct BrowserWorkspace {
    key: BrowserWorkspaceKey,
    provider_lock: std::sync::Mutex<Option<String>>,
    slot: Arc<RuntimeSlot>,
    coordinator: Arc<BrowserRunCoordinator>,
    closing: AtomicBool,
}

impl BrowserWorkspace {
    pub fn runtime_generation(&self)->u64 {self.slot.request.runtime_generation}
    pub async fn has_active_run(&self)->bool {self.coordinator.has_active_run().await}
    pub async fn screenshot(self: &Arc<Self>, run: &BrowserRunGuard, tab_id: Option<String>) -> Result<crate::runtime::BrowserScreenshot, WorkspaceError> {
        let workspace = self.clone();
        self.coordinator.agent_operation(run, move |cancel| async move {
            Ok(async {
                if workspace.closing.load(Ordering::Acquire) { return Err(WorkspaceError::WorkspaceClosed); }
                let runtime = workspace.slot.ensure().await?;
                runtime.automation().ok_or(WorkspaceError::UnsupportedAction)?.screenshot(tab_id, cancel).await
            }.await)
        }).await?
    }

    pub async fn agent_snapshot(
        self: &Arc<Self>,
        run: &BrowserRunGuard,
    ) -> Result<BrowserWorkspaceSnapshot, WorkspaceError> {
        let workspace = self.clone();
        self.coordinator
            .agent_operation(run, move |_| async move { Ok(workspace.snapshot().await) })
            .await?
    }
    pub fn run_changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.coordinator.subscribe()
    }

    pub async fn runtime_changes(
        &self,
    ) -> Result<tokio::sync::watch::Receiver<u64>, WorkspaceError> {
        self.slot
            .ensure()
            .await?
            .changes()
            .ok_or(WorkspaceError::NativeUnavailable)
    }
    pub async fn observe(
        self: &Arc<Self>,
        run: &BrowserRunGuard,
        tab_id: Option<String>,
    ) -> Result<crate::runtime::BrowserObservation, WorkspaceError> {
        let workspace = self.clone();
        self.coordinator
            .agent_operation(run, move |cancel| async move {
                Ok(async {
                    if workspace.closing.load(Ordering::Acquire) {
                        return Err(WorkspaceError::WorkspaceClosed);
                    }
                    let runtime = workspace.slot.ensure().await?;
                    runtime
                        .automation()
                        .ok_or(WorkspaceError::UnsupportedAction)?
                        .observe(tab_id, cancel)
                        .await
                }
                .await)
            })
            .await?
    }

    pub async fn act(
        self: &Arc<Self>,
        run: &BrowserRunGuard,
        action: crate::runtime::BrowserAction,
    ) -> Result<crate::runtime::BrowserActionResult, WorkspaceError> {
        let workspace = self.clone();
        self.coordinator
            .agent_operation(run, move |cancel| async move {
                Ok(async {
                    if workspace.closing.load(Ordering::Acquire) {
                        return Err(WorkspaceError::WorkspaceClosed);
                    }
                    let runtime = workspace.slot.ensure().await?;
                    runtime
                        .automation()
                        .ok_or(WorkspaceError::UnsupportedAction)?
                        .act(action, cancel)
                        .await
                }
                .await)
            })
            .await?
    }
    pub fn key(&self) -> &BrowserWorkspaceKey {
        &self.key
    }
    pub async fn evaluate(self: &Arc<Self>, run: &BrowserRunGuard, request: crate::runtime::BrowserEvaluation)
        -> Result<crate::runtime::BrowserEvaluationResult, WorkspaceError> {
        let workspace = self.clone();
        self.coordinator.agent_operation(run, move |cancel| async move {
            Ok(async {
                if workspace.closing.load(Ordering::Acquire) { return Err(WorkspaceError::WorkspaceClosed); }
                let runtime = workspace.slot.ensure().await?;
                runtime.automation().ok_or(WorkspaceError::UnsupportedAction)?.evaluate(request, cancel).await
            }.await)
        }).await?
    }
    pub async fn respond_dialog(self: &Arc<Self>, run: &BrowserRunGuard, reply: crate::runtime::BrowserDialogReply)
        -> Result<crate::runtime::BrowserActionResult, WorkspaceError> {
        let workspace = self.clone();
        self.coordinator.agent_operation(run, move |cancel| async move {
            Ok(async {
                if workspace.closing.load(Ordering::Acquire) { return Err(WorkspaceError::WorkspaceClosed); }
                let runtime = workspace.slot.ensure().await?;
                runtime.automation().ok_or(WorkspaceError::UnsupportedAction)?.respond_dialog(reply, cancel).await
            }.await)
        }).await?
    }
    pub async fn upload(
        self: &Arc<Self>, run: &BrowserRunGuard,
        element: crate::runtime::BrowserElementRef,
        scope: Arc<crate::uploads::BrowserUploadScope>, paths: Vec<String>,
    ) -> Result<crate::runtime::BrowserActionResult, WorkspaceError> {
        let workspace=self.clone();
        self.coordinator.agent_operation(run,move |cancel|async move {
            Ok(async {
                if workspace.closing.load(Ordering::Acquire) {return Err(WorkspaceError::WorkspaceClosed);}
                let runtime=workspace.slot.ensure().await?;
                let automation=runtime.automation().ok_or(WorkspaceError::UnsupportedAction)?;
                let files=scope.prepare(paths,cancel.clone()).await?;
                automation.upload(element,files,cancel).await
            }.await)
        }).await?
    }
    pub async fn download(
        self: &Arc<Self>, run: &BrowserRunGuard,
        element: crate::runtime::BrowserElementRef,
        scope: Arc<crate::downloads::BrowserDownloadScope>,
    ) -> Result<crate::runtime::BrowserActionResult, WorkspaceError> {
        let workspace=self.clone();
        self.coordinator.agent_operation(run,move |cancel|async move {
            Ok(async {
                if workspace.closing.load(Ordering::Acquire) {return Err(WorkspaceError::WorkspaceClosed);}
                let runtime=workspace.slot.ensure().await?;
                let automation=runtime.automation().ok_or(WorkspaceError::UnsupportedAction)?;
                let file=scope.prepare()?;
                automation.download(element,file,cancel).await
            }.await)
        }).await?
    }

    pub async fn snapshot(&self) -> Result<BrowserWorkspaceSnapshot, WorkspaceError> {
        let state = self.slot.inner.lock().await;
        let runtime = match &state.runtime {
            Some(runtime) => Some(runtime.snapshot().await?),
            None => None,
        };
        drop(state);
        Ok(BrowserWorkspaceSnapshot {
            conversation_id: self.key.conversation_id.clone(),
            run: self.coordinator.snapshot().await,
            runtime,
        })
    }

    /// Only AgentSession's trusted lifecycle owner receives this in-process guard.
    pub async fn begin_run(self: &Arc<Self>) -> Result<BrowserRunGuard, WorkspaceError> {
        if self.closing.load(Ordering::Acquire) {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        let run = self.coordinator.begin().await?;
        if self.closing.load(Ordering::Acquire) {
            self.coordinator.finish(&run).await?;
            return Err(WorkspaceError::WorkspaceClosed);
        }
        Ok(run)
    }

    pub async fn finish_run(&self, run: &BrowserRunGuard) -> Result<(), WorkspaceError> {
        self.coordinator.finish(run).await?;
        Ok(())
    }

    pub async fn settle_run(&self, run: &BrowserRunGuard) -> Result<(), WorkspaceError> {
        self.coordinator.settle(run).await?;
        Ok(())
    }

    /// Host-authenticated desktop layout only. This never changes run authority.
    pub async fn set_surface(
        &self,
        bounds: crate::runtime::BrowserSurfaceBounds,
        visible: bool,
        layout_cancel: CancellationToken,
    ) -> Result<(), WorkspaceError> {
        if layout_cancel.is_cancelled() {
            return Ok(());
        }
        if !visible {
            // Detaching a closed/never-opened pane must not create a runtime.
            // If cleanup is in flight, retain its handle and hide what remains.
            let runtime = self.slot.inner.lock().await.runtime.clone();
            return match runtime {
                Some(runtime) => {
                    runtime
                        .surface()
                        .ok_or(WorkspaceError::NativeUnavailable)?
                        .set_surface(bounds, false, layout_cancel)
                        .await
                }
                None => Ok(()),
            };
        }
        if self.closing.load(Ordering::Acquire) {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        let runtime = self.slot.ensure().await?;
        runtime
            .surface()
            .ok_or(WorkspaceError::NativeUnavailable)?
            .set_surface(bounds, visible, layout_cancel)
            .await
    }

    pub async fn user_command(
        self: &Arc<Self>,
        command: BrowserTabCommand,
    ) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        if self.closing.load(Ordering::Acquire) {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        let workspace = self.clone();
        self.coordinator
            .user_operation(move || async move {
                Ok(workspace.execute(command, CancellationToken::new()).await)
            })
            .await?
    }

    pub async fn agent_command(
        self: &Arc<Self>,
        run: &BrowserRunGuard,
        command: BrowserTabCommand,
    ) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        if matches!(command, BrowserTabCommand::Permission { .. } | BrowserTabCommand::Dialog { .. } | BrowserTabCommand::CancelDownload { .. } | BrowserTabCommand::OpenExternal { .. } | BrowserTabCommand::CloseAll { .. } | BrowserTabCommand::OpenDownloads { .. } | BrowserTabCommand::ClearSiteData { .. }) {
            return Err(WorkspaceError::UnsupportedAction);
        }
        if self.closing.load(Ordering::Acquire) {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        let workspace = self.clone();
        self.coordinator
            .agent_operation(run, move |cancel| async move {
                Ok(workspace.execute(command, cancel).await)
            })
            .await?
    }

    async fn execute(
        &self,
        command: BrowserTabCommand,
        cancel: CancellationToken,
    ) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        if self.closing.load(Ordering::Acquire) {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        let runtime = if let BrowserTabCommand::CloseAll { runtime_generation } | BrowserTabCommand::OpenDownloads { runtime_generation } | BrowserTabCommand::ClearSiteData { runtime_generation } = &command {
            if *runtime_generation != self.slot.request.runtime_generation {
                return Err(WorkspaceError::StaleTarget);
            }
            let state=self.slot.inner.lock().await;
            if state.closed { return Err(WorkspaceError::WorkspaceClosed); }
            state.runtime.clone().ok_or(WorkspaceError::TabNotFound)?
        } else { self.slot.ensure().await? };
        if cancel.is_cancelled() {
            return Err(RunAdmissionError::Cancelled.into());
        }
        runtime.execute(command, cancel).await
    }

    /// Ingress closes immediately; native destruction is serialized behind any
    /// in-flight run operation. Failure retains authority for another close.
    pub async fn close(self: &Arc<Self>) -> Result<(), WorkspaceError> {
        self.closing.store(true, Ordering::Release);
        let workspace = self.clone();
        self.coordinator
            .close_runtime(move || async move { workspace.slot.close().await })
            .await?
    }

    async fn close_idle(self: &Arc<Self>)->Result<(),WorkspaceError> {
        let workspace=self.clone();
        self.coordinator.close_idle_runtime(move ||async move {
            workspace.closing.store(true,Ordering::Release);
            workspace.slot.close().await
        }).await?
    }

    /// Positive destruction evidence, not merely a closing flag or an error.
    /// A failed native close retains the runtime and returns false here.
    #[cfg(test)]
    async fn native_close_proven(&self)->bool {
        let Ok(state)=self.slot.inner.try_lock() else {return false;};
        state.closed && state.runtime.is_none()
    }
}

pub struct BrowserWorkspaceService {
    factory: Arc<dyn BrowserRuntimeFactory>,
    workspaces: Mutex<BTreeMap<BrowserWorkspaceKey, Arc<BrowserWorkspace>>>,
    next_generation: AtomicU64,
    stopping: AtomicBool,
}

impl BrowserWorkspaceService {
    pub async fn get(&self, key: &BrowserWorkspaceKey) -> Option<Arc<BrowserWorkspace>> {
        self.workspaces.lock().await.get(key).cloned()
    }
    pub fn new(factory: Arc<dyn BrowserRuntimeFactory>) -> Self {
        Self {
            factory,
            workspaces: Mutex::new(BTreeMap::new()),
            next_generation: AtomicU64::new(1),
            stopping: AtomicBool::new(false),
        }
    }

    /// The authenticated host supplies user/Conversation/profile and exact provider.
    /// ensure does not launch a browser; opening the first tab does.
    pub async fn ensure(
        &self,
        key: BrowserWorkspaceKey,
        provider_lock: String,
        profile: BrowserProfile,
    ) -> Result<Arc<BrowserWorkspace>, WorkspaceError> {
        if provider_lock.is_empty() {
            return Err(WorkspaceError::ProviderChanged);
        }
        self.ensure_bound(key, Some(provider_lock), profile).await
    }

    pub async fn ensure_user(
        &self,
        key: BrowserWorkspaceKey,
        profile: BrowserProfile,
    ) -> Result<Arc<BrowserWorkspace>, WorkspaceError> {
        self.ensure_bound(key, None, profile).await
    }

    async fn ensure_bound(
        &self,
        key: BrowserWorkspaceKey,
        provider_lock: Option<String>,
        profile: BrowserProfile,
    ) -> Result<Arc<BrowserWorkspace>, WorkspaceError> {
        let mut workspaces = self.workspaces.lock().await;
        if self.stopping.load(Ordering::Acquire) {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        if let Some(workspace) = workspaces.get(&key) {
            if workspace.closing.load(Ordering::Acquire) {
                return Err(WorkspaceError::WorkspaceClosed);
            }
            if let Some(provider) = &provider_lock {
                let mut bound = workspace
                    .provider_lock
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if bound.as_ref().is_some_and(|current| current != provider) {
                    return Err(WorkspaceError::ProviderChanged);
                }
                *bound = Some(provider.clone());
            }
            return Ok(workspace.clone());
        }
        let slot = Arc::new(RuntimeSlot {
            factory: self.factory.clone(),
            request: CreateBrowserRuntime {
                key: key.clone(),
                runtime_generation: self.next_generation.fetch_add(1, Ordering::Relaxed),
                profile,
                user_input_enabled: true,
            },
            inner: Mutex::new(SlotState {
                runtime: None,
                locked: false,
                closed: false,
            }),
        });
        let workspace = Arc::new(BrowserWorkspace {
            key: key.clone(),
            provider_lock: std::sync::Mutex::new(provider_lock),
            coordinator: BrowserRunCoordinator::new(slot.clone()),
            slot,
            closing: AtomicBool::new(false),
        });
        workspaces.insert(key, workspace.clone());
        Ok(workspace)
    }

    pub async fn close(&self, key: &BrowserWorkspaceKey) -> Result<(), WorkspaceError> {
        let workspace = self.workspaces.lock().await.get(key).cloned();
        if let Some(workspace) = workspace {
            workspace.close().await?;
            let mut workspaces = self.workspaces.lock().await;
            if workspaces
                .get(key)
                .is_some_and(|current| Arc::ptr_eq(current, &workspace))
            {
                workspaces.remove(key);
            }
        }
        Ok(())
    }

    /// Infrastructure for an explicitly confirmed human browser rebuild.
    /// The application must also retire its idle cached Agent before exposing
    /// a replacement Workspace; this is not an Agent or public unlock API.
    pub async fn close_idle(self: &Arc<Self>,key:BrowserWorkspaceKey,expected_generation:u64)->Result<(),WorkspaceError> {
        let service=self.clone();
        tokio::spawn(async move {
            let workspace=service.workspaces.lock().await.get(&key).cloned();
            if let Some(workspace)=workspace {
                if workspace.runtime_generation()!=expected_generation {return Err(WorkspaceError::StaleTarget);}
                workspace.close_idle().await?;
                let mut workspaces=service.workspaces.lock().await;
                if workspaces.get(&key).is_some_and(|current|Arc::ptr_eq(current,&workspace)) {workspaces.remove(&key);}
            }
            Ok(())
        }).await.map_err(|_|WorkspaceError::Admission(RunAdmissionError::WorkerFailed))?
    }

    pub async fn shutdown(&self) -> Result<(), WorkspaceError> {
        self.stopping.store(true, Ordering::Release);
        let keys: Vec<_> = self.workspaces.lock().await.keys().cloned().collect();
        let mut failure = None;
        for key in keys {
            if let Err(error) = self.close(&key).await {
                failure = Some(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests;
