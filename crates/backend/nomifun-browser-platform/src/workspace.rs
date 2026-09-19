//! AgentSession Browser Resource authority. Agent turns and the visible panel
//! borrow the same provider-backed runtime.

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
        BrowserProfile, BrowserProfileBinding, BrowserProfilePersistence,
        BrowserProfileStore, BrowserResourceKey, BrowserRuntime, BrowserRuntimeFactory,
        BrowserRuntimeSnapshot, BrowserTabCommand, CreateBrowserRuntime, WorkspaceError,
        is_bounded_profile_identity,
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

pub struct BrowserResource {
    key: BrowserResourceKey,
    authority: BrowserSessionAuthority,
    slot: Arc<RuntimeSlot>,
    coordinator: Arc<BrowserRunCoordinator>,
    closing: AtomicBool,
}

impl BrowserResource {
    pub fn runtime_generation(&self)->u64 {self.slot.request.runtime_generation}
    pub async fn has_active_run(&self)->bool {self.coordinator.has_active_run().await}
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
        self.authority.authorize(action_for_tab_command(&command))?;
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

pub struct BrowserResourceService {
    factory: Arc<dyn BrowserRuntimeFactory>,
    profile_store: Option<BrowserProfileStore>,
    resources: Mutex<BTreeMap<BrowserResourceKey, Arc<BrowserResource>>>,
    lifecycle: Mutex<()>,
    retired_sessions: Mutex<BTreeSet<(String, String)>>,
    next_generation: AtomicU64,
    stopping: AtomicBool,
}

impl BrowserResourceService {
    pub async fn get_for_agent_session(
        &self,
        principal_id: &str,
        agent_session_id: &str,
    ) -> Result<Option<Arc<BrowserResource>>, WorkspaceError> {
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
        let resource = self.resources.lock().await.get(&authority.key()).cloned();
        if let Some(resource) = &resource {
            ensure_same_authority(&resource.authority, authority)?;
        }
        Ok(resource)
    }
    pub fn new(factory: Arc<dyn BrowserRuntimeFactory>) -> Self {
        Self {
            factory,
            profile_store: None,
            resources: Mutex::new(BTreeMap::new()),
            lifecycle: Mutex::new(()),
            retired_sessions: Mutex::new(BTreeSet::new()),
            next_generation: AtomicU64::new(1),
            stopping: AtomicBool::new(false),
        }
    }

    pub fn with_profile_store(mut self, profile_store: BrowserProfileStore) -> Self {
        self.profile_store = Some(profile_store);
        self
    }

    /// The authenticated host supplies an immutable AgentSession Action grant,
    /// exact Resource binding, profile and provider lock. `ensure` does not
    /// launch a browser; opening the first tab does.
    pub async fn ensure(
        &self,
        authority: BrowserSessionAuthority,
        profile: BrowserProfile,
    ) -> Result<Arc<BrowserResource>, WorkspaceError> {
        let _lifecycle = self.lifecycle.lock().await;
        let key = authority.key();
        if authority.resource().provider().kind() != BrowserProviderKind::Managed {
            return Err(WorkspaceError::NativeUnavailable);
        }
        if let Some(store) = &self.profile_store {
            let persistence = match &profile {
                BrowserProfile::Persistent(_) => BrowserProfilePersistence::Persistent,
                BrowserProfile::Ephemeral => BrowserProfilePersistence::Ephemeral,
            };
            if store.profile_for(&key, persistence)? != profile {
                return Err(WorkspaceError::ProfileCleanupInvalid);
            }
        }
        if self
            .retired_sessions
            .lock()
            .await
            .contains(&(key.principal_id.clone(), key.agent_session_id.clone()))
        {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        let mut resources = self.resources.lock().await;
        if self.stopping.load(Ordering::Acquire) {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        if let Some(resource) = resources.get(&key) {
            if resource.closing.load(Ordering::Acquire) {
                return Err(WorkspaceError::WorkspaceClosed);
            }
            ensure_same_authority(&resource.authority, &authority)?;
            return Ok(resource.clone());
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
        let resource = Arc::new(BrowserResource {
            key: key.clone(),
            authority,
            coordinator: BrowserRunCoordinator::new(slot.clone()),
            slot,
            closing: AtomicBool::new(false),
        });
        resources.insert(key, resource.clone());
        Ok(resource)
    }

    pub async fn close(&self, key: &BrowserResourceKey) -> Result<(), WorkspaceError> {
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

    /// Destructive canonical AgentSession owner operation. The caller supplies
    /// only authenticated identities and the frozen managed-Browser binding
    /// policies; profile paths are recomputed from the host-owned store.
    ///
    /// Native resources and run guards settle first. Any close, authority, or
    /// filesystem failure is returned so the Session tombstone cannot commit.
    /// Replays are exact and absorb already-removed profile directories.
    pub async fn delete_agent_session(
        &self,
        principal_id: &str,
        agent_session_id: &str,
        bindings: &[BrowserProfileBinding],
    ) -> Result<(), WorkspaceError> {
        if !is_bounded_profile_identity(principal_id)
            || !is_bounded_profile_identity(agent_session_id)
        {
            return Err(WorkspaceError::ProfileCleanupInvalid);
        }
        let mut frozen = BTreeMap::new();
        for binding in bindings {
            if frozen
                .insert(
                    binding.resource_binding_id().to_owned(),
                    binding.persistence(),
                )
                .is_some()
            {
                return Err(WorkspaceError::ProfileCleanupInvalid);
            }
        }

        let _lifecycle = self.lifecycle.lock().await;
        let live = self
            .close_agent_session_locked(principal_id, agent_session_id)
            .await?;
        for (key, profile) in live {
            let Some(persistence) = frozen.get(&key.resource_binding_id) else {
                return Err(WorkspaceError::ProfileCleanupInvalid);
            };
            if !matches!(
                (*persistence, &profile),
                (BrowserProfilePersistence::Persistent, BrowserProfile::Persistent(_))
                    | (BrowserProfilePersistence::Ephemeral, BrowserProfile::Ephemeral)
            ) {
                return Err(WorkspaceError::ProfileCleanupInvalid);
            }
        }

        let persistent = frozen
            .into_iter()
            .filter_map(|(resource_binding_id, persistence)| {
                (persistence == BrowserProfilePersistence::Persistent).then(|| {
                    BrowserResourceKey {
                        principal_id: principal_id.to_owned(),
                        agent_session_id: agent_session_id.to_owned(),
                        resource_binding_id,
                    }
                })
            })
            .collect::<Vec<_>>();
        if persistent.is_empty() {
            return Ok(());
        }
        let store = self
            .profile_store
            .clone()
            .ok_or(WorkspaceError::ProfileCleanupUnavailable)?;
        tokio::task::spawn_blocking(move || {
            for key in persistent {
                store.delete_persistent_profile(&key)?;
            }
            Ok(())
        })
        .await
        .map_err(|_| WorkspaceError::ProfileCleanupFailed)?
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
    /// The application must also retire its idle cached Agent before exposing
    /// a replacement Resource; this is not an Agent or public unlock API.
    pub async fn close_idle(self: &Arc<Self>,key:BrowserResourceKey,expected_generation:u64)->Result<(),WorkspaceError> {
        let service=self.clone();
        tokio::spawn(async move {
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

    pub async fn shutdown(&self) -> Result<(), WorkspaceError> {
        let _lifecycle = self.lifecycle.lock().await;
        self.stopping.store(true, Ordering::Release);
        let keys: Vec<_> = self.resources.lock().await.keys().cloned().collect();
        let mut failure = None;
        for key in keys {
            if let Err(error) = self.close(&key).await {
                failure = Some(error);
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        self.factory.shutdown().await
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
        | BrowserTabCommand::Navigate { .. }
        | BrowserTabCommand::Back { .. }
        | BrowserTabCommand::Forward { .. }
        | BrowserTabCommand::Reload { .. }
        | BrowserTabCommand::StopLoading { .. } => BrowserCapabilityAction::Navigate,
        BrowserTabCommand::Activate { .. } | BrowserTabCommand::Close { .. } => {
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
