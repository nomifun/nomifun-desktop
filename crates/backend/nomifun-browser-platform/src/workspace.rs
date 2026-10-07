//! Domain-owned managed browser. Authenticated users and exactly authorized
//! Agent turns borrow one physical runtime per canonical Session.

use std::{
    collections::{BTreeMap, BTreeSet},
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
    product::{
        BrowserCapabilityAction, BrowserProviderKind, BrowserSessionAuthority,
    },
    run_guard::{
        BrowserRunCoordinator, BrowserRunGuard, BrowserRunSnapshot, NativeInputGate,
        RunAdmissionError,
    },
    runtime::{
        BrowserProfile, BrowserProfilePersistence,
        BrowserProfileStore, BrowserResourceKey, BrowserRuntime, BrowserRuntimeFactory,
        BrowserRuntimeSnapshot, BrowserTabCommand, CreateBrowserRuntime, WorkspaceError,
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
pub struct BrowserResourceSnapshot {
    pub agent_session_id: String,
    pub resource_binding_id: String,
    pub provider_id: String,
    pub provider_kind: BrowserProviderKind,
    pub allowed_actions: BTreeSet<String>,
    pub run: BrowserRunSnapshot,
    pub runtime: Option<BrowserRuntimeSnapshot>,
}

/// A user surface describes the physical managed browser, never Agent grants.
#[derive(Clone, Debug, Serialize)]
pub struct BrowserUserSnapshot {
    pub agent_session_id: String,
    pub browser_id: String,
    pub run: BrowserRunSnapshot,
    pub runtime: Option<BrowserRuntimeSnapshot>,
}

pub fn managed_workspace_key(principal_id: &str, agent_session_id: &str) -> Result<BrowserResourceKey, WorkspaceError> {
    let key = BrowserResourceKey {
        principal_id: principal_id.to_owned(),
        agent_session_id: agent_session_id.to_owned(),
        resource_binding_id: "managed-browser".to_owned(),
    };
    key.validate_profile_identity()?;
    Ok(key)
}

pub struct BrowserWorkspace {
    key: BrowserResourceKey,
    agent_handles: Mutex<BTreeMap<String, std::sync::Weak<BrowserResource>>>,
    provider: Mutex<Option<crate::product::BrowserProviderDescriptor>>,
    slot: Arc<RuntimeSlot>,
    coordinator: Arc<BrowserRunCoordinator>,
    closing: AtomicBool,
}

impl BrowserWorkspace {
    async fn authorize_agent(self: &Arc<Self>, authority: BrowserSessionAuthority) -> Result<Arc<BrowserResource>, WorkspaceError> {
        if authority.principal_id() != self.key.principal_id || authority.agent_session_id() != self.key.agent_session_id {
            return Err(WorkspaceError::ActionDenied);
        }
        if authority.resource().provider().kind() != BrowserProviderKind::Managed { return Err(WorkspaceError::NativeUnavailable); }
        let mut provider = self.provider.lock().await;
        if provider.as_ref().is_some_and(|current| current != authority.resource().provider()) {
            return Err(WorkspaceError::ProviderChanged);
        }
        *provider = Some(authority.resource().provider().clone());
        drop(provider);
        // Weak handles are an implementation cache, never the active grant set.
        // Every caller supplies current canonical authority before entering it.
        let mut handles = self.agent_handles.lock().await;
        handles.retain(|_, handle| handle.strong_count() > 0);
        if let Some(resource) = handles.get(authority.resource().binding_id()).and_then(std::sync::Weak::upgrade) {
            ensure_same_authority(resource.authority(), &authority)?;
            return Ok(resource);
        }
        if self.closing.load(Ordering::Acquire) { return Err(WorkspaceError::WorkspaceClosed); }
        let resource = Arc::new(BrowserResource { key: authority.key(), authority, workspace: self.clone() });
        handles.insert(resource.key.resource_binding_id.clone(), Arc::downgrade(&resource));
        Ok(resource)
    }

    pub fn runtime_generation(&self)->u64 {self.slot.request.runtime_generation}
    pub async fn has_active_run(&self)->bool {self.coordinator.has_active_run().await}
    pub fn run_changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.coordinator.subscribe()
    }

    pub async fn runtime_changes(
        &self,
    ) -> Result<tokio::sync::watch::Receiver<u64>, WorkspaceError> {
        if self.closing.load(Ordering::Acquire) {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        self.slot
            .ensure()
            .await?
            .changes()
            .ok_or(WorkspaceError::NativeUnavailable)
    }
    pub fn key(&self) -> &BrowserResourceKey {
        &self.key
    }
    pub async fn snapshot(&self) -> Result<BrowserUserSnapshot, WorkspaceError> {
        let state = self.slot.inner.lock().await;
        let runtime = match &state.runtime {
            Some(runtime) => Some(runtime.snapshot().await?),
            None => None,
        };
        drop(state);
        Ok(BrowserUserSnapshot {
            agent_session_id: self.key.agent_session_id.clone(),
            browser_id: self.key.resource_binding_id.clone(),
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

    pub async fn settle_failed_start(self: &Arc<Self>) -> Result<(), WorkspaceError> {
        let state = self.slot.inner.lock().await;
        if state.closed && state.runtime.is_none() { return Ok(()); }
        drop(state);
        self.coordinator.settle_failed_start().await?;
        Ok(())
    }

    /// The trusted Turn owner calls this only when begin_run returned no guard.
    /// A failed native gate remains locked until this recovery proves cleanup.
    pub async fn recover_failed_start(self: &Arc<Self>) -> Result<(), WorkspaceError> {
        let state = self.slot.inner.lock().await;
        if state.closed && state.runtime.is_none() { return Ok(()); }
        drop(state);
        self.coordinator.recover_failed_start().await?;
        Ok(())
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
        if let BrowserTabCommand::SetZoom { percent, .. } = &command {
            if !(50..=200).contains(percent) {
                return Err(WorkspaceError::InvalidZoom);
            }
        }
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

/// Exact canonical Agent authorization over the same domain-owned browser.
/// Constructed only after the host validates the current Session binding.
pub struct BrowserResource {
    key: BrowserResourceKey,
    authority: BrowserSessionAuthority,
    workspace: Arc<BrowserWorkspace>,
}
impl std::ops::Deref for BrowserResource {
    type Target = BrowserWorkspace;
    fn deref(&self) -> &Self::Target { &self.workspace }
}
impl BrowserResource {
    pub fn workspace(&self) -> &Arc<BrowserWorkspace> { &self.workspace }
    pub async fn begin_run(self: &Arc<Self>) -> Result<BrowserRunGuard, WorkspaceError> { self.workspace.begin_run().await }
    pub async fn user_command(self: &Arc<Self>, command: BrowserTabCommand) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        self.workspace.user_command(command).await
    }
    pub async fn close(self: &Arc<Self>) -> Result<(), WorkspaceError> { self.workspace.close().await }

    pub async fn screenshot(self: &Arc<Self>, run: &BrowserRunGuard, tab_id: Option<String>) -> Result<crate::runtime::BrowserScreenshot, WorkspaceError> {
        let workspace = self.clone();
        self.coordinator.agent_operation(run, move |cancel| async move {
            Ok(async {
                workspace.authority.authorize(BrowserCapabilityAction::RenderContent)?;
                if workspace.closing.load(Ordering::Acquire) { return Err(WorkspaceError::WorkspaceClosed); }
                let runtime = workspace.slot.ensure().await?;
                runtime.automation().ok_or(WorkspaceError::UnsupportedAction)?.screenshot(tab_id, cancel).await
            }.await)
        }).await?
    }

    pub async fn agent_snapshot(
        self: &Arc<Self>,
        run: &BrowserRunGuard,
    ) -> Result<BrowserResourceSnapshot, WorkspaceError> {
        let workspace = self.clone();
        self.coordinator
            .agent_operation(run, move |_| async move {
                Ok(async {
                    workspace
                        .authority
                        .authorize(BrowserCapabilityAction::Observe)?;
                    workspace.snapshot().await
                }
                .await)
            })
            .await?
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
                    workspace.authority.authorize(BrowserCapabilityAction::Observe)?;
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
                    workspace.authority.authorize(BrowserCapabilityAction::Act)?;
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
    pub fn key(&self) -> &BrowserResourceKey {
        &self.key
    }
    pub fn authority(&self) -> &BrowserSessionAuthority {
        &self.authority
    }
    pub async fn evaluate(self: &Arc<Self>, run: &BrowserRunGuard, request: crate::runtime::BrowserEvaluation)
        -> Result<crate::runtime::BrowserEvaluationResult, WorkspaceError> {
        let workspace = self.clone();
        self.coordinator.agent_operation(run, move |cancel| async move {
            Ok(async {
                workspace.authority.authorize(BrowserCapabilityAction::Evaluate)?;
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
                workspace.authority.authorize(BrowserCapabilityAction::Act)?;
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
                workspace.authority.authorize(BrowserCapabilityAction::Upload)?;
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
                workspace.authority.authorize(BrowserCapabilityAction::Download)?;
                if workspace.closing.load(Ordering::Acquire) {return Err(WorkspaceError::WorkspaceClosed);}
                let runtime=workspace.slot.ensure().await?;
                let automation=runtime.automation().ok_or(WorkspaceError::UnsupportedAction)?;
                let file=scope.prepare()?;
                automation.download(element,file,cancel).await
            }.await)
        }).await?
    }

    pub async fn snapshot(&self) -> Result<BrowserResourceSnapshot, WorkspaceError> {
        let state = self.slot.inner.lock().await;
        let runtime = match &state.runtime {
            Some(runtime) => Some(runtime.snapshot().await?),
            None => None,
        };
        drop(state);
        Ok(BrowserResourceSnapshot {
            agent_session_id: self.key.agent_session_id.clone(),
            resource_binding_id: self.key.resource_binding_id.clone(),
            provider_id: self
                .authority
                .resource()
                .provider()
                .provider_id()
                .to_owned(),
            provider_kind: self.authority.resource().provider().kind(),
            allowed_actions: BrowserCapabilityAction::all()
                .into_iter()
                .filter(|action| self.authority.authorize(*action).is_ok())
                .map(|action| action.action_id().to_owned())
                .collect(),
            run: self.coordinator.snapshot().await,
            runtime,
        })
    }

    pub async fn agent_command(
        self: &Arc<Self>,
        run: &BrowserRunGuard,
        command: BrowserTabCommand,
    ) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        if matches!(command, BrowserTabCommand::Permission { .. } | BrowserTabCommand::Dialog { .. } | BrowserTabCommand::CancelDownload { .. } | BrowserTabCommand::OpenExternal { .. } | BrowserTabCommand::CloseAll { .. } | BrowserTabCommand::OpenDownloads { .. } | BrowserTabCommand::ClearSiteData { .. } | BrowserTabCommand::SetZoom { .. }) {
            return Err(WorkspaceError::UnsupportedAction);
        }
        self.authority.authorize(action_for_tab_command(&command))?;
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
}

pub struct BrowserResourceService {
    factory: Arc<dyn BrowserRuntimeFactory>,
    profile_store: Option<BrowserProfileStore>,
    resources: Mutex<BTreeMap<BrowserResourceKey, Arc<BrowserWorkspace>>>,
    lifecycle: Mutex<ServiceLifecycle>,
    retired_sessions: Mutex<BTreeSet<(String, String)>>,
    next_generation: AtomicU64,
    stopping: AtomicBool,
}

#[derive(Default)]
struct ServiceLifecycle {
    resources_closed: bool,
    native_shutdown: Option<tokio::task::JoinHandle<Result<(), WorkspaceError>>>,
    native_closed: bool,
    native_after_storage: bool,
}

impl BrowserResourceService {
    pub async fn get_for_agent_session(
        &self,
        principal_id: &str,
        agent_session_id: &str,
    ) -> Result<Option<Arc<BrowserWorkspace>>, WorkspaceError> {
        let resources = self.resources.lock().await;
        let mut matches = resources.iter().filter(|(key, _)| {
            key.principal_id == principal_id && key.agent_session_id == agent_session_id
        });
        let resource = matches.next().map(|(_, resource)| Arc::clone(resource));
        if matches.next().is_some() {
            return Err(WorkspaceError::ProviderChanged);
        }
        Ok(resource)
    }

    pub async fn get(
        &self,
        authority: &BrowserSessionAuthority,
    ) -> Result<Option<Arc<BrowserResource>>, WorkspaceError> {
        let key = managed_workspace_key(authority.principal_id(), authority.agent_session_id())?;
        let workspace = self.resources.lock().await.get(&key).cloned();
        match workspace {
            Some(workspace) => workspace.authorize_agent(authority.clone()).await.map(Some),
            None => Ok(None),
        }
    }
    pub fn new(factory: Arc<dyn BrowserRuntimeFactory>) -> Self {
        Self {
            factory,
            profile_store: None,
            resources: Mutex::new(BTreeMap::new()),
            lifecycle: Mutex::new(ServiceLifecycle::default()),
            retired_sessions: Mutex::new(BTreeSet::new()),
            next_generation: AtomicU64::new(1),
            stopping: AtomicBool::new(false),
        }
    }

    pub fn with_profile_store(mut self, profile_store: BrowserProfileStore) -> Self {
        self.profile_store = Some(profile_store);
        self
    }

    /// The caller proves ownership of the canonical Session. This creates no
    /// Agent binding and does not start the native engine or open a webpage.
    pub async fn ensure_user(
        &self,
        principal_id: &str,
        agent_session_id: &str,
        profile: BrowserProfile,
    ) -> Result<Arc<BrowserWorkspace>, WorkspaceError> {
        let _lifecycle = self.lifecycle.lock().await;
        let key = managed_workspace_key(principal_id, agent_session_id)?;
        if let Some(store) = &self.profile_store {
            let persistence = match &profile {
                BrowserProfile::Persistent(_) => BrowserProfilePersistence::Persistent,
                BrowserProfile::Ephemeral => BrowserProfilePersistence::Ephemeral,
            };
            if store.profile_for(&key, persistence)? != profile {
                return Err(WorkspaceError::ProfileCleanupInvalid);
            }
        }
        if self.retired_sessions.lock().await.contains(&(key.principal_id.clone(), key.agent_session_id.clone())) {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        let mut resources = self.resources.lock().await;
        if self.stopping.load(Ordering::Acquire) { return Err(WorkspaceError::WorkspaceClosed); }
        if let Some(workspace) = resources.get(&key) {
            if workspace.closing.load(Ordering::Acquire) { return Err(WorkspaceError::WorkspaceClosed); }
            if workspace.slot.request.profile != profile { return Err(WorkspaceError::ProfileCleanupInvalid); }
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
            inner: Mutex::new(SlotState { runtime: None, locked: false, closed: false }),
        });
        let workspace = Arc::new(BrowserWorkspace {
            key: key.clone(),
            agent_handles: Mutex::new(BTreeMap::new()),
            provider: Mutex::new(None),
            coordinator: BrowserRunCoordinator::new(slot.clone()),
            slot,
            closing: AtomicBool::new(false),
        });
        resources.insert(key, workspace.clone());
        Ok(workspace)
    }

    /// Bind exact Agent authority to the existing physical managed browser.
    /// A new grant definition never selects another page or profile.
    pub async fn ensure(
        &self,
        authority: BrowserSessionAuthority,
        profile: BrowserProfile,
    ) -> Result<Arc<BrowserResource>, WorkspaceError> {
        if authority.resource().provider().kind() != BrowserProviderKind::Managed {
            return Err(WorkspaceError::NativeUnavailable);
        }
        self.ensure_user(authority.principal_id(), authority.agent_session_id(), profile)
            .await?.authorize_agent(authority).await
    }

    pub async fn close(&self, key: &BrowserResourceKey) -> Result<(), WorkspaceError> {
        let key = managed_workspace_key(&key.principal_id, &key.agent_session_id)?;
        let key = &key;
        let resource = self.resources.lock().await.get(key).cloned();
        if let Some(resource) = resource {
            resource.close().await?;
            let mut resources = self.resources.lock().await;
            if resources
                .get(key)
                .is_some_and(|current| Arc::ptr_eq(current, &resource))
            {
                resources.remove(key);
            }
        }
        Ok(())
    }

    /// Canonical AgentSession deletion/resource-lease cleanup. Every Browser
    /// binding for the exact principal and Session is closed; another Session
    /// is never selected by resource id or profile path.
    pub async fn close_agent_session(
        &self,
        principal_id: &str,
        agent_session_id: &str,
    ) -> Result<(), WorkspaceError> {
        let _lifecycle = self.lifecycle.lock().await;
        self.close_agent_session_locked(principal_id, agent_session_id)
            .await
            .map(|_| ())
    }

    /// Canonical deletion includes the domain-owned user browser, even when
    /// this Session has never granted Browser actions to an Agent.
    pub async fn delete_agent_session(
        &self,
        principal_id: &str,
        agent_session_id: &str,
    ) -> Result<(), WorkspaceError> {
        let key = managed_workspace_key(principal_id, agent_session_id)?;
        let _lifecycle = self.lifecycle.lock().await;
        let live = self.close_agent_session_locked(principal_id, agent_session_id).await?;
        let persistent = live.iter().any(|(_, profile)| matches!(profile, BrowserProfile::Persistent(_)));
        let Some(store) = self.profile_store.clone() else {
            return if persistent { Err(WorkspaceError::ProfileCleanupUnavailable) } else { Ok(()) };
        };
        tokio::task::spawn_blocking(move || store.delete_persistent_profile(&key))
            .await.map_err(|_| WorkspaceError::ProfileCleanupFailed)?
    }

    async fn close_agent_session_locked(
        &self,
        principal_id: &str,
        agent_session_id: &str,
    ) -> Result<Vec<(BrowserResourceKey, BrowserProfile)>, WorkspaceError> {
        self.retired_sessions.lock().await.insert((
            principal_id.to_owned(),
            agent_session_id.to_owned(),
        ));
        let resources = self
            .resources
            .lock()
            .await
            .iter()
            .filter_map(|(key, resource)| {
                (key.principal_id == principal_id
                    && key.agent_session_id == agent_session_id)
                    .then(|| {
                        (
                            key.clone(),
                            resource.slot.request.profile.clone(),
                        )
                    })
            })
            .collect::<Vec<_>>();
        let mut failure = None;
        for (key, _) in &resources {
            if let Err(error) = self.close(key).await {
                failure = Some(error);
            }
        }
        failure.map_or(Ok(resources), Err)
    }

    /// Infrastructure for an explicitly confirmed human browser rebuild.
    /// A running native guard is rejected. Terminal-proven failed-release
    /// recovery uses the separate trusted close_after_terminal entry.
    pub async fn close_idle(self: &Arc<Self>,key:BrowserResourceKey,expected_generation:u64)->Result<(),WorkspaceError> {
        let service=self.clone();
        tokio::spawn(async move {
            let key = managed_workspace_key(&key.principal_id, &key.agent_session_id)?;
            let resource=service.resources.lock().await.get(&key).cloned();
            if let Some(resource)=resource {
                if resource.runtime_generation()!=expected_generation {return Err(WorkspaceError::StaleTarget);}
                resource.close_idle().await?;
                let mut resources=service.resources.lock().await;
                if resources.get(&key).is_some_and(|current|Arc::ptr_eq(current,&resource)) {resources.remove(&key);}
            }
            Ok(())
        }).await.map_err(|_|WorkspaceError::Admission(RunAdmissionError::WorkerFailed))?
    }

    /// The application has proved the exact retained run's durable terminal
    /// under its canonical Session operation fence. This may close a settled
    /// guard whose final unlock failed; it is never a running-Agent Stop API.
    pub async fn close_after_terminal(self: &Arc<Self>, key: BrowserResourceKey, expected_generation: u64) -> Result<(), WorkspaceError> {
        let service = self.clone();
        tokio::spawn(async move {
            let key = managed_workspace_key(&key.principal_id, &key.agent_session_id)?;
            let resource = service.resources.lock().await.get(&key).cloned();
            if let Some(resource) = resource {
                if resource.runtime_generation() != expected_generation { return Err(WorkspaceError::StaleTarget); }
                resource.close().await?;
                let mut resources = service.resources.lock().await;
                if resources.get(&key).is_some_and(|current| Arc::ptr_eq(current, &resource)) { resources.remove(&key); }
            }
            Ok(())
        }).await.map_err(|_| WorkspaceError::Admission(RunAdmissionError::WorkerFailed))?
    }

    /// Factory opt-in only; this is not proof that resources have closed.
    pub fn supports_storage_independent_shutdown(&self) -> bool {
        self.factory.supports_storage_independent_shutdown()
    }

    /// Permanently close admission and wait for every Resource's native view
    /// and owned operation cleanup. Process-wide native infrastructure remains
    /// alive so the host can complete its other shutdown prerequisites.
    /// Failure or cancellation keeps the barrier unproven and can be retried.
    pub async fn close_resources(&self) -> Result<(), WorkspaceError> {
        self.stopping.store(true, Ordering::Release);
        let mut lifecycle = self.lifecycle.lock().await;
        if lifecycle.resources_closed {
            return Ok(());
        }
        let keys: Vec<_> = self
            .resources
            .lock()
            .await
            .iter()
            .map(|(key, resource)| {
                // Close every cached Resource's ingress before awaiting any one
                // native view; a delayed first close cannot admit later runs.
                resource.closing.store(true, Ordering::Release);
                key.clone()
            })
            .collect();
        let mut failure = None;
        for key in keys {
            if let Err(error) = self.close(&key).await {
                failure = Some(error);
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        lifecycle.resources_closed = true;
        Ok(())
    }

    /// Close process-wide native infrastructure only after a successful
    /// `close_resources` barrier. The host owns any intervening prerequisites.
    /// The stored worker survives a dropped caller; retries join that same
    /// physical shutdown, and an acknowledged success is never re-entered.
    pub async fn close_native_runtime(&self) -> Result<(), WorkspaceError> {
        self.close_native_runtime_inner(false).await
    }

    /// Used only after the host has joined consumers and acknowledged storage
    /// closure. Ordinary resource callers cannot upgrade an existing flight.
    pub async fn close_native_runtime_after_storage_close(&self) -> Result<(), WorkspaceError> {
        if !self.supports_storage_independent_shutdown() {return Err(WorkspaceError::UnsupportedAction);}
        self.close_native_runtime_inner(true).await
    }

    async fn close_native_runtime_inner(&self, after_storage: bool) -> Result<(), WorkspaceError> {
        let mut lifecycle = self.lifecycle.lock().await;
        if !lifecycle.resources_closed {
            return Err(WorkspaceError::NativeCommandFailed);
        }
        if lifecycle.native_closed {
            return Ok(());
        }
        if lifecycle.native_shutdown.is_none() {
            let factory = self.factory.clone();
            lifecycle.native_after_storage=after_storage;
            lifecycle.native_shutdown =
                Some(tokio::spawn(async move {
                    if after_storage {factory.shutdown_after_storage_close().await} else {factory.shutdown().await}
                }));
        } else if after_storage && !lifecycle.native_after_storage {
            return Err(WorkspaceError::NativeCommandFailed);
        }
        let result = lifecycle
            .native_shutdown
            .as_mut()
            .expect("native shutdown worker was installed")
            .await
            .unwrap_or(Err(WorkspaceError::Admission(RunAdmissionError::WorkerFailed)));
        lifecycle.native_shutdown = None;
        if result.is_ok() {
            lifecycle.native_closed = true;
        }
        result
    }

    /// Compatibility entry point for hosts with no intervening prerequisites.
    pub async fn shutdown(&self) -> Result<(), WorkspaceError> {
        self.close_resources().await?;
        self.close_native_runtime().await
    }
}

fn ensure_same_authority(
    current: &BrowserSessionAuthority,
    requested: &BrowserSessionAuthority,
) -> Result<(), WorkspaceError> {
    if current.resource().provider() != requested.resource().provider() {
        return Err(WorkspaceError::ProviderChanged);
    }
    if current != requested {
        return Err(WorkspaceError::ActionDenied);
    }
    Ok(())
}

fn action_for_tab_command(command: &BrowserTabCommand) -> BrowserCapabilityAction {
    match command {
        BrowserTabCommand::Create { .. }
        | BrowserTabCommand::Activate { .. }
        | BrowserTabCommand::SetZoom { .. }
        | BrowserTabCommand::Navigate { .. }
        | BrowserTabCommand::Back { .. }
        | BrowserTabCommand::Forward { .. }
        | BrowserTabCommand::Reload { .. }
        | BrowserTabCommand::StopLoading { .. } => BrowserCapabilityAction::Navigate,
        BrowserTabCommand::Close { .. } => {
            BrowserCapabilityAction::Act
        }
        BrowserTabCommand::OpenDownloads { .. }
        | BrowserTabCommand::CancelDownload { .. } => BrowserCapabilityAction::Download,
        BrowserTabCommand::CloseAll { .. }
        | BrowserTabCommand::ClearSiteData { .. }
        | BrowserTabCommand::OpenExternal { .. }
        | BrowserTabCommand::Permission { .. }
        | BrowserTabCommand::Dialog { .. } => BrowserCapabilityAction::Act,
    }
}

#[cfg(test)]
mod tests;
