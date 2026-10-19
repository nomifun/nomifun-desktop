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
    // The Workspace and its native input gate author one shared purpose fact.
    // Input handoff must pin possible human use before native enable begins.
    uses: Arc<Mutex<WorkspaceUses>>,
    use_changes: Arc<crate::revision::BrowserRevision>,
}

struct SlotState {
    runtime: Option<Arc<dyn BrowserRuntime>>,
    created_by: Option<RuntimeUseSource>,
    locked: bool,
    native_lock_acknowledged: bool,
    closed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RuntimeUseSource { Agent, User }

impl RuntimeSlot {
    async fn ensure(&self, source: RuntimeUseSource) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
        let mut state = self.inner.lock().await;
        if state.closed {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        if let Some(runtime) = &state.runtime {
            if source == RuntimeUseSource::Agent && state.locked && !state.native_lock_acknowledged {
                return Err(RunAdmissionError::InputGateFailed.into());
            }
            return Ok(runtime.clone());
        }
        let mut request = self.request.clone();
        request.user_input_enabled = !state.locked;
        let runtime = self.factory.create(request).await?;
        state.created_by = Some(source);
        state.runtime = Some(runtime.clone());
        if state.locked {
            // Keep the created owner even if its native gate fails. The
            // request's initial-input policy is not a native acknowledgement.
            state.native_lock_acknowledged = false;
            runtime.lock_user_input().await?;
            state.native_lock_acknowledged = true;
        }
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
        let runtime = {
            let mut state = self.inner.lock().await;
            state.locked = true;
            state.native_lock_acknowledged = false;
            state.runtime.clone()
        };
        if let Some(runtime) = runtime {
            runtime.lock_user_input().await?;
            let mut state = self.inner.lock().await;
            if !state.closed && state.runtime.as_ref().is_some_and(|current| Arc::ptr_eq(current, &runtime)) {
                state.native_lock_acknowledged = true;
            }
        }
        Ok(())
    }
    async fn release_pressed_input(&self) -> Result<(), RunAdmissionError> {
        let mut state = self.inner.lock().await;
        let acknowledged = state.native_lock_acknowledged;
        state.native_lock_acknowledged = false;
        if let Some(runtime) = &state.runtime {
            runtime.release_pressed_input().await?;
        }
        state.native_lock_acknowledged = acknowledged;
        Ok(())
    }
    async fn prepare_agent_operation(&self) -> Result<(), RunAdmissionError> {
        let mut state = self.inner.lock().await;
        let acknowledged = state.native_lock_acknowledged;
        state.native_lock_acknowledged = false;
        if let Some(runtime) = &state.runtime {
            runtime.prepare_agent_operation().await?;
        }
        state.native_lock_acknowledged = acknowledged;
        Ok(())
    }
    async fn try_release_for_user(&self) -> Result<bool, RunAdmissionError> {
        let mut state = self.inner.lock().await;
        let acknowledged = state.native_lock_acknowledged;
        state.native_lock_acknowledged = false;
        let released = match &state.runtime {
            Some(runtime) => runtime.try_release_for_user().await,
            None => Ok(true),
        }?;
        state.native_lock_acknowledged = acknowledged;
        Ok(released)
    }
    async fn unlock_user_input(&self) -> Result<(), RunAdmissionError> {
        let runtime = {
            let mut state = self.inner.lock().await;
            if state.closed { return Ok(()); }
            let mut uses = self.uses.lock().await;
            // Some(true) includes an unconfirmed/cancelled show. None means
            // this owner never dispatched a show; retain that existing hidden
            // task-only path. An attachment can later expose the same page.
            if uses.presentation == Some(true) || !uses.surface_attachments.is_empty() {
                uses.user_pages = true;
                self.use_changes.bump();
            }
            state.native_lock_acknowledged = false;
            let runtime = state.runtime.clone();
            if runtime.is_none() { state.locked = false; }
            runtime
        };
        if let Some(runtime) = runtime {
            // No slot/use lock across native enable: a hide can still make
            // progress. A concurrent show sees the unconfirmed gate and pins.
            runtime.unlock_user_input().await?;
            let mut state = self.inner.lock().await;
            if !state.closed && state.runtime.as_ref().is_some_and(|current| Arc::ptr_eq(current, &runtime)) {
                state.locked = false;
            }
        }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interaction_capabilities: Option<crate::runtime::BrowserInteractionCapabilities>,
    pub run: BrowserRunSnapshot,
    pub runtime: Option<BrowserRuntimeSnapshot>,
}

/// A user surface describes the physical managed browser, never Agent grants.
#[derive(Clone, Debug, Serialize)]
pub struct BrowserUserSnapshot {
    pub agent_session_id: String,
    pub browser_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interaction_capabilities: Option<crate::runtime::BrowserInteractionCapabilities>,
    pub run: BrowserRunSnapshot,
    pub runtime: Option<BrowserRuntimeSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserRetentionReason {
    NoRegisteredWorkspace,
    ActiveTask,
    CurrentUserUse,
    TaskCompletionUnknown,
    UserPageStateUnknown,
    NativeCleanup,
    EligibleTaskResources,
    ObservationBusy,
    Retired,
}

/// Process-local observation of this Browser owner. Absence of a registered
/// Workspace is not a process-wide resource release receipt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BrowserWorkspaceObservation {
    pub service_instance_id: Option<String>,
    pub registered: bool,
    pub runtime_generation: Option<u64>,
    /// Native handle registration, never an inference that an OS process lives.
    pub native_registered: Option<bool>,
    /// Only a positive exact native close can produce Some(true).
    pub close_confirmed: Option<bool>,
    pub closing: bool,
    pub cleanup_unconfirmed: bool,
    pub task_operation_id: Option<String>,
    pub retention: BrowserRetentionReason,
    pub operation_in_flight: Option<bool>,
}

impl BrowserWorkspaceObservation {
    pub fn unregistered() -> Self {
          Self { service_instance_id: None, registered: false, runtime_generation: None, native_registered: None,
            close_confirmed: None, closing: false, cleanup_unconfirmed: false, task_operation_id: None,
            retention: BrowserRetentionReason::NoRegisteredWorkspace, operation_in_flight: None }
    }
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
    // Only the currently admitted human navigation's native cancellation.
    // This is transient operation ownership, never a second history ledger.
    user_navigation: std::sync::Mutex<Option<Arc<UserNavigation>>>,
    uses: Arc<Mutex<WorkspaceUses>>,
    use_changes: Arc<crate::revision::BrowserRevision>,
    registry: std::sync::Weak<Mutex<BTreeMap<BrowserResourceKey, Arc<BrowserWorkspace>>>>,
    retirement_worker: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

#[derive(Clone, Default, PartialEq, Eq)]
struct WorkspaceUses {
    task: Option<String>,
    turn_ended: bool,
    task_completed: bool,
    // Actual human intent or possible input-enabled exposure retains nonempty
    // pages. Agent-only presentation never clears a preceding human purpose.
    // This marker never retains a carrier whose last page was closed.
    user_pages: bool,
    // None means no native presentation lifecycle has been acknowledged. An
    // API command alone cannot invent an active or detached physical surface.
    presentation: Option<bool>,
    presentation_epoch: u64,
    surface_attachments: BTreeSet<u64>,
    retirement_started: bool,
}

enum UserNavigationScope {
    Existing(crate::runtime::BrowserTabTarget),
    Creating { runtime_generation: u64, previous_tabs: BTreeSet<String> },
}

struct UserNavigation {
    scope: UserNavigationScope,
    cancel: CancellationToken,
}

struct UserNavigationLease {
    workspace: std::sync::Weak<BrowserWorkspace>,
    navigation: Arc<UserNavigation>,
}

impl Drop for UserNavigationLease {
    fn drop(&mut self) {
        if let Some(workspace) = self.workspace.upgrade() {
            let mut pending = workspace.user_navigation.lock().unwrap_or_else(|error| error.into_inner());
            if pending.as_ref().is_some_and(|current| Arc::ptr_eq(current, &self.navigation)) {
                *pending = None;
            }
        }
    }
}

impl BrowserWorkspace {
    /// Exact native UI attachment ownership, separate from temporary hiding
    /// for a menu/overlay. Only the trusted native surface owner authors this.
    pub async fn retain_surface_attachment(&self, attachment: u64) -> Result<(), WorkspaceError> {
        if attachment == 0 { return Err(WorkspaceError::StaleTarget); }
        let mut uses = self.uses.lock().await;
        if self.closing.load(Ordering::Acquire) { return Err(WorkspaceError::WorkspaceClosed); }
        uses.surface_attachments.insert(attachment);
        self.use_changes.bump();
        Ok(())
    }

    /// Native hide is already acknowledged, or this exact attachment failed
    /// before any show was dispatched. A stale release cannot end a successor.
    pub async fn release_surface_attachment(&self, attachment: u64) {
        self.uses.lock().await.surface_attachments.remove(&attachment);
        self.use_changes.bump();
        self.schedule_registered_retirement().await;
    }

    /// The Browser owner registers the exact canonical task before borrowing
    /// the carrier. A successor protects it even between browser interactions.
    pub async fn retain_task(&self, operation: &str) -> Result<(), WorkspaceError> {
        if operation.is_empty() { return Err(WorkspaceError::StaleTarget); }
        let mut uses = self.uses.lock().await;
        if self.closing.load(Ordering::Acquire) { return Err(WorkspaceError::WorkspaceClosed); }
        uses.task = Some(operation.to_owned());
        uses.turn_ended = false;
        uses.task_completed = false;
        self.use_changes.bump();
        Ok(())
    }

    /// Accept cleanup responsibility without holding the Session caller while
    /// native destruction waits. The owned worker remains the close authority.
    pub async fn request_retirement_after_turn(self: &Arc<Self>, operation: &str, mission_completed: bool)
        -> Result<bool, WorkspaceError> {
        {
            let mut uses = self.uses.lock().await;
            if uses.task.as_deref() != Some(operation) { return Ok(false); }
            uses.turn_ended = true;
            uses.task_completed |= mission_completed;
            self.use_changes.bump();
        }
        self.schedule_retirement().await;
        Ok(true)
    }

    /// The trusted task owner already proved the formal result and settled
    /// effects/native work. This is a retirement request, not a close receipt.
    pub async fn try_retire_after_task(self: &Arc<Self>, operation: &str) -> Result<bool, WorkspaceError> {
        {
            let mut uses = self.uses.lock().await;
            if uses.task.as_deref() != Some(operation) { return Ok(false); }
            uses.turn_ended = true;
            uses.task_completed = true;
            self.use_changes.bump();
        }
        // Subscribe before inspecting the remaining responsibility. Download
        // completion at this boundary must not be lost between read and wait.
        self.schedule_retirement().await;
        self.try_retire_once().await
    }

    /// A closed conversational Turn alone permits only an already-empty
    /// carrier to retire. Nonempty page/background responsibility is unknown.
    pub async fn try_retire_empty_after_turn(self: &Arc<Self>, operation: &str) -> Result<bool, WorkspaceError> {
        {
            let mut uses = self.uses.lock().await;
            if uses.task.as_deref() != Some(operation) { return Ok(false); }
            uses.turn_ended = true;
            self.use_changes.bump();
        }
        self.schedule_retirement().await;
        self.try_retire_once().await
    }

    async fn try_retire_once(self: &Arc<Self>) -> Result<bool, WorkspaceError> {
        let workspace = self.clone();
        let retired = self.coordinator.idle_retirement(move || async move {
            let observed = {
                let uses = workspace.uses.lock().await;
                if uses.task.is_some() && !uses.turn_ended { return Ok(false); }
                if workspace.closing.load(Ordering::Acquire) && !uses.retirement_started { return Ok(false); }
                uses.clone()
            };
            if !observed.retirement_started {
                let state = workspace.slot.inner.lock().await;
                if let Some(runtime) = &state.runtime {
                    let snapshot = runtime.snapshot().await?;
                    if snapshot.downloads.iter().any(|download| matches!(download.state,
                        crate::runtime::BrowserDownloadState::Choosing | crate::runtime::BrowserDownloadState::InProgress)) {
                        return Ok(false);
                    }
                    // Cancelling/failed transfers still need native proof;
                    // close joins that original cleanup rather than pinning a
                    // completed task forever on its failed release projection.
                    let task_only = observed.task_completed && state.created_by == Some(RuntimeUseSource::Agent) && !observed.user_pages;
                    let detached_empty = snapshot.tabs.is_empty() && observed.presentation == Some(false);
                    if !observed.surface_attachments.is_empty() || observed.presentation == Some(true)
                        || (!task_only && !detached_empty) { return Ok(false); }
                } else if !observed.turn_ended && observed.presentation != Some(false) { return Ok(false); }
                drop(state);
                let mut uses = workspace.uses.lock().await;
                // A successor or human use may have arrived during the native
                // read. The short use/admission fence decides which one wins.
                if *uses != observed || workspace.closing.load(Ordering::Acquire) { return Ok(false); }
                // No user page can be lost: either no native page remains, or
                // this Agent-created carrier was never exposed to human input.
                // Visible/user downloads cannot be newly admitted in that
                // subset; native close still joins exact producers and handles.
                uses.retirement_started = true;
                workspace.closing.store(true, Ordering::Release);
                workspace.use_changes.bump();
            }
            // No use lock crosses the native close. New task admission can
            // observe this exact closing generation without waiting on it.
            workspace.slot.close().await?;
            Ok::<bool, WorkspaceError>(true)
        }).await?;
        let retired = retired.transpose()?.unwrap_or(false);
        if retired {
            if let Some(registry) = self.registry.upgrade() {
                let mut registry = registry.lock().await;
                if registry.get(&self.key).is_some_and(|current| Arc::ptr_eq(current, self)) {
                    registry.remove(&self.key);
                }
            }
        }
        Ok(retired)
    }

    async fn schedule_retirement(self: &Arc<Self>) {
        if self.closing.load(Ordering::Acquire) && !self.uses.lock().await.retirement_started { return; }
        let mut worker = self.retirement_worker.lock().await;
        if worker.as_ref().is_some_and(|worker| !worker.is_finished()) { return; }
        let weak = Arc::downgrade(self);
        *worker = Some(tokio::spawn(async move {
            let mut delay = std::time::Duration::from_secs(1);
            loop {
                let Some(workspace) = weak.upgrade() else { return; };
                let mut use_changes = workspace.use_changes.subscribe();
                let mut changes = {
                    let state = workspace.slot.inner.lock().await;
                    state.runtime.as_ref().and_then(|runtime| runtime.changes())
                };
                let result = workspace.try_retire_once().await;
                if matches!(result, Ok(true)) { return; }
                {
                    let uses = workspace.uses.lock().await;
                    if workspace.closing.load(Ordering::Acquire) && !uses.retirement_started { return; }
                }
                drop(workspace);
                match result {
                    Ok(false) => {
                        tokio::select! {
                            changed = use_changes.changed() => if changed.is_err() { return; },
                            changed = async { match changes.as_mut() {
                                Some(changes) => changes.changed().await,
                                None => std::future::pending().await,
                            }} => if changed.is_err() { return; },
                        }
                    },
                    Err(error) => {
                        tracing::warn!(code=error.code(), "Browser automatic retirement remains owned for retry");
                        tokio::select! {
                            _ = tokio::time::sleep(delay) => {},
                            changed = use_changes.changed() => if changed.is_err() { return; },
                        }
                        delay = (delay * 2).min(std::time::Duration::from_secs(30));
                    },
                    Ok(true) => return,
                }
            }
        }));
    }

    async fn schedule_registered_retirement(&self) {
        let Some(registry) = self.registry.upgrade() else { return; };
        let workspace = registry.lock().await.get(&self.key)
            .filter(|current| std::ptr::eq(current.as_ref(), self)).cloned();
        if let Some(workspace) = workspace { workspace.schedule_retirement().await; }
    }

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
            .ensure(RuntimeUseSource::User)
            .await?
            .changes()
            .ok_or(WorkspaceError::NativeUnavailable)
    }
    pub fn key(&self) -> &BrowserResourceKey {
        &self.key
    }
    /// Read the existing user's exact tab; diagnostics never initialize a
    /// runtime, grant Agent authority, or disturb the native input owner.
    pub async fn navigation_diagnostics(
        &self,
        target: crate::runtime::BrowserTabTarget,
    ) -> Result<crate::navigation::BrowserNavigationReport, WorkspaceError> {
        if self.closing.load(Ordering::Acquire) {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        if target.runtime_generation != self.runtime_generation() {
            return Err(WorkspaceError::StaleTarget);
        }
        let runtime = self.slot.inner.lock().await.runtime.clone()
            .ok_or(WorkspaceError::TabNotFound)?;
        let report = runtime.navigation_diagnostics(target.clone()).await?;
        if self.closing.load(Ordering::Acquire) || report.target != target {
            return Err(WorkspaceError::StaleTarget);
        }
        Ok(report.project_metadata())
    }
    pub async fn snapshot(&self) -> Result<BrowserUserSnapshot, WorkspaceError> {
        let state = self.slot.inner.lock().await;
        let interaction_capabilities = state.runtime.as_ref().and_then(|runtime| runtime.interaction_capabilities());
        let runtime = match &state.runtime {
            Some(runtime) => Some(runtime.snapshot().await?),
            None => None,
        };
        drop(state);
        Ok(BrowserUserSnapshot {
            agent_session_id: self.key.agent_session_id.clone(),
            browser_id: self.key.resource_binding_id.clone(),
            interaction_capabilities,
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

    /// Borrow the browser for one interaction, preserving valid observations
    /// while the Agent continues other work between browser calls.
    pub async fn begin_interaction(self: &Arc<Self>) -> Result<BrowserRunGuard, WorkspaceError> {
        if self.closing.load(Ordering::Acquire) {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        let run = self.coordinator.begin_interaction().await?;
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

    pub async fn finish_interaction(&self, run: &BrowserRunGuard) -> Result<bool, WorkspaceError> {
        Ok(self.coordinator.finish_interaction(run).await?)
    }

    pub async fn settle_run(&self, run: &BrowserRunGuard) -> Result<(), WorkspaceError> {
        self.coordinator.settle(run).await?;
        Ok(())
    }

    /// An explicit human opening/retention intent, including opening a pane
    /// under an Agent input lock. It grants no Agent or native input authority.
    pub async fn retain_user_page_use(&self) -> Result<(), WorkspaceError> {
        let mut uses = self.uses.lock().await;
        if self.closing.load(Ordering::Acquire) { return Err(WorkspaceError::WorkspaceClosed); }
        uses.user_pages = true;
        self.use_changes.bump();
        Ok(())
    }

    /// Host-authenticated human/unknown desktop layout. Possible human page
    /// exposure remains retained before native show, including failed shows.
    pub async fn set_surface(
        &self,
        bounds: crate::runtime::BrowserSurfaceBounds,
        visible: bool,
        layout_cancel: CancellationToken,
    ) -> Result<(), WorkspaceError> {
        self.set_surface_for_source(bounds, visible, layout_cancel, false).await
    }

    /// Trusted native Agent-auto presentation only; the caller's source flag
    /// is not authority. Only a proven original native input lock can avoid
    /// pinning human page use. Native show/hide still uses the normal owner.
    pub async fn set_agent_surface(
        &self,
        bounds: crate::runtime::BrowserSurfaceBounds,
        visible: bool,
        layout_cancel: CancellationToken,
    ) -> Result<(), WorkspaceError> {
        self.set_surface_for_source(bounds, visible, layout_cancel, true).await
    }

    async fn set_surface_for_source(
        &self,
        bounds: crate::runtime::BrowserSurfaceBounds,
        visible: bool,
        layout_cancel: CancellationToken,
        agent_auto: bool,
    ) -> Result<(), WorkspaceError> {
        if layout_cancel.is_cancelled() {
            return Ok(());
        }
        // Never await coordinator state while holding the Slot: input
        // transitions call the Slot from that original coordinator. A later
        // handoff still pins the latest presentation before native enable.
        let run = if visible && agent_auto { Some(self.coordinator.snapshot().await) } else { None };
        let epoch = if visible {
            let state = self.slot.inner.lock().await;
            let agent_locked = agent_auto && state.created_by == Some(RuntimeUseSource::Agent)
                && state.locked && state.native_lock_acknowledged && !state.closed
                && run.is_some_and(|run| run.input_state == crate::run_guard::BrowserInputState::AgentRunning && !run.input_gate_failed);
            let mut uses = self.uses.lock().await;
            uses.presentation_epoch = uses.presentation_epoch.checked_add(1).ok_or(WorkspaceError::NativeCommandFailed)?;
            if !agent_locked { uses.user_pages = true; }
            // Dispatch may show before its waiter returns. Even Agent-only
            // unknown/cancelled shows retain presentation until exact hide.
            uses.presentation = Some(true);
            self.use_changes.bump();
            uses.presentation_epoch
        } else {
            let mut uses = self.uses.lock().await;
            uses.presentation_epoch = uses.presentation_epoch.checked_add(1).ok_or(WorkspaceError::NativeCommandFailed)?;
            self.use_changes.bump();
            uses.presentation_epoch
        };
        if !visible {
            // Detaching a closed/never-opened pane must not create a runtime.
            // If cleanup is in flight, retain its handle and hide what remains.
            let runtime = self.slot.inner.lock().await.runtime.clone();
            let result = match runtime {
                Some(runtime) => {
                    runtime
                        .surface()
                        .ok_or(WorkspaceError::NativeUnavailable)?
                        .set_surface(bounds, false, layout_cancel.clone())
                        .await
                }
                None => Ok(()),
            };
            if result.is_ok() && !layout_cancel.is_cancelled() {
                let mut uses = self.uses.lock().await;
                if uses.presentation_epoch == epoch {
                    // Only the current original native acknowledgement ends
                    // presentation use; a late hide cannot overtake a show.
                    uses.presentation = Some(false);
                    self.use_changes.bump();
                }
                drop(uses);
                self.schedule_registered_retirement().await;
            }
            return result;
        }
        if self.closing.load(Ordering::Acquire) {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        let runtime = self.slot.ensure(RuntimeUseSource::User).await?;
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
        if matches!(&command, BrowserTabCommand::StopLoading { .. } | BrowserTabCommand::PauseNavigation { .. }) {
            let workspace = self.clone();
            let interrupt = command.clone();
            let cancelled = self.coordinator.user_interrupt(move || async move {
                workspace.interrupt_user_navigation(interrupt).await
            }).await??;
            if cancelled {
                // The original owned task retains the operation gate through
                // native callback settlement. Join that gate before returning;
                // never replay Stop/Pause against a successor document.
                let workspace = self.clone();
                return self.coordinator.user_operation(move || async move {
                    Ok(workspace.existing_runtime_snapshot().await)
                }).await?;
            }
        }
        let workspace = self.clone();
        let result = self.coordinator
            .user_operation(move || async move {
                workspace.uses.lock().await.user_pages = true;
                Ok(workspace.execute_user_command(command).await)
            })
            .await?;
        if result.is_ok() { self.schedule_retirement().await; }
        result
    }

    async fn existing_runtime_snapshot(&self) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        if self.closing.load(Ordering::Acquire) { return Err(WorkspaceError::WorkspaceClosed); }
        let runtime = self.slot.inner.lock().await.runtime.clone().ok_or(WorkspaceError::TabNotFound)?;
        runtime.snapshot().await
    }

    async fn interrupt_user_navigation(&self, command: BrowserTabCommand) -> Result<bool, WorkspaceError> {
        let (target, expected_sequence) = match &command {
            BrowserTabCommand::StopLoading { target, expected_navigation_sequence }
            | BrowserTabCommand::PauseNavigation { target, expected_navigation_sequence } => (target, *expected_navigation_sequence),
            _ => return Err(WorkspaceError::ActionDenied),
        };
        if target.runtime_generation != self.runtime_generation() { return Err(WorkspaceError::StaleTarget); }
        let pending = self.user_navigation.lock().unwrap_or_else(|error| error.into_inner()).clone();
        let Some(pending) = pending else { return Ok(false); };
        let matches_scope = match &pending.scope {
            UserNavigationScope::Existing(original) => original.tab_id == target.tab_id
                && original.runtime_generation == target.runtime_generation,
            UserNavigationScope::Creating { runtime_generation, previous_tabs } =>
                *runtime_generation == target.runtime_generation && !previous_tabs.contains(&target.tab_id),
        };
        if !matches_scope { return Ok(false); }
        let runtime = self.slot.inner.lock().await.runtime.clone().ok_or(WorkspaceError::TabNotFound)?;
        let snapshot = runtime.snapshot().await?;
        if !snapshot.tabs.iter().any(|tab| tab.target == *target
            && tab.load.as_ref().is_some_and(|load| load.navigation_sequence == expected_sequence)) {
            return Err(WorkspaceError::StaleTarget);
        }
        // This is the user's explicit current-target control, rather than an
        // old command's guessed cancellation receipt. Native dispatch rechecks
        // the target/attempt atomically; the original operation still owns the
        // coordinator gate until its callback and cleanup settle.
        runtime.execute(command, CancellationToken::new()).await?;
        let registered = self.user_navigation.lock().unwrap_or_else(|error| error.into_inner());
        if registered.as_ref().is_some_and(|current| Arc::ptr_eq(current, &pending)) {
            pending.cancel.cancel();
        }
        Ok(true)
    }

    async fn execute_user_command(self: &Arc<Self>, command: BrowserTabCommand) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        let scope = match &command {
            BrowserTabCommand::Navigate { target, .. } | BrowserTabCommand::Reload { target, .. }
            | BrowserTabCommand::Back { target } | BrowserTabCommand::Forward { target } =>
                Some(UserNavigationScope::Existing(target.clone())),
            BrowserTabCommand::Create { .. } => {
                let snapshot = self.slot.ensure(RuntimeUseSource::User).await?.snapshot().await?;
                Some(UserNavigationScope::Creating { runtime_generation: snapshot.runtime_generation,
                    previous_tabs: snapshot.tabs.into_iter().map(|tab| tab.target.tab_id).collect() })
            }
            _ => None,
        };
        let cancel = CancellationToken::new();
        let _lease = scope.map(|scope| {
            let navigation = Arc::new(UserNavigation { scope, cancel: cancel.clone() });
            *self.user_navigation.lock().unwrap_or_else(|error| error.into_inner()) = Some(navigation.clone());
            UserNavigationLease { workspace: Arc::downgrade(self), navigation }
        });
        self.execute(command, cancel, RuntimeUseSource::User).await
    }

    async fn execute(
        &self,
        command: BrowserTabCommand,
        cancel: CancellationToken,
        source: RuntimeUseSource,
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
        } else { self.slot.ensure(source).await? };
        if cancel.is_cancelled() {
            return Err(RunAdmissionError::Cancelled.into());
        }
        runtime.execute(command, cancel).await
    }

    /// Ingress closes immediately; native destruction is serialized behind any
    /// in-flight run operation. Failure retains authority for another close.
    pub async fn close(self: &Arc<Self>) -> Result<(), WorkspaceError> {
        self.closing.store(true, Ordering::Release);
        self.use_changes.bump();
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
    pub async fn begin_interaction(self: &Arc<Self>) -> Result<BrowserRunGuard, WorkspaceError> { self.workspace.begin_interaction().await }
    pub async fn user_command(self: &Arc<Self>, command: BrowserTabCommand) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        self.workspace.user_command(command).await
    }
    pub async fn close(self: &Arc<Self>) -> Result<(), WorkspaceError> { self.workspace.close().await }

    pub async fn screenshot(self: &Arc<Self>, run: &BrowserRunGuard, tab_id: Option<String>) -> Result<crate::runtime::BrowserScreenshot, WorkspaceError> {
        let workspace = self.clone();
        self.coordinator.agent_operation(run, move |cancel| async move {
            Ok(async {
                workspace.authority.authorize(BrowserCapabilityAction::Observe)?;
                if workspace.closing.load(Ordering::Acquire) { return Err(WorkspaceError::WorkspaceClosed); }
                let runtime = workspace.slot.ensure(RuntimeUseSource::Agent).await?;
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
                    let runtime = workspace.slot.ensure(RuntimeUseSource::Agent).await?;
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
                    let runtime = workspace.slot.ensure(RuntimeUseSource::Agent).await?;
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
                let runtime = workspace.slot.ensure(RuntimeUseSource::Agent).await?;
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
                let runtime = workspace.slot.ensure(RuntimeUseSource::Agent).await?;
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
                let runtime=workspace.slot.ensure(RuntimeUseSource::Agent).await?;
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
                let runtime=workspace.slot.ensure(RuntimeUseSource::Agent).await?;
                let automation=runtime.automation().ok_or(WorkspaceError::UnsupportedAction)?;
                let file=scope.prepare()?;
                automation.download(element,file,cancel).await
            }.await)
        }).await?
    }

    pub async fn snapshot(&self) -> Result<BrowserResourceSnapshot, WorkspaceError> {
        let state = self.slot.inner.lock().await;
        let interaction_capabilities = state.runtime.as_ref().and_then(|runtime| runtime.interaction_capabilities());
        let runtime = match &state.runtime {
            Some(runtime) => Some(runtime.snapshot().await?),
            None => None,
        };
        drop(state);
        Ok(BrowserResourceSnapshot {
            interaction_capabilities,
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
        if matches!(command, BrowserTabCommand::Permission { .. } | BrowserTabCommand::Dialog { .. } | BrowserTabCommand::CancelDownload { .. } | BrowserTabCommand::OpenExternal { .. } | BrowserTabCommand::CloseAll { .. } | BrowserTabCommand::OpenDownloads { .. } | BrowserTabCommand::ClearSiteData { .. } | BrowserTabCommand::SetZoom { .. } | BrowserTabCommand::PauseNavigation { .. } | BrowserTabCommand::ResumeNavigation { .. }) {
            return Err(WorkspaceError::UnsupportedAction);
        }
        self.authority.authorize(action_for_tab_command(&command))?;
        if self.closing.load(Ordering::Acquire) {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        let workspace = self.clone();
        self.coordinator
            .agent_operation(run, move |cancel| async move {
                Ok(workspace.execute(command, cancel, RuntimeUseSource::Agent).await)
            })
            .await?
    }
}

pub struct BrowserResourceService {
    factory: Arc<dyn BrowserRuntimeFactory>,
    profile_store: Option<BrowserProfileStore>,
    resources: Arc<Mutex<BTreeMap<BrowserResourceKey, Arc<BrowserWorkspace>>>>,
    lifecycle: Mutex<ServiceLifecycle>,
    retired_sessions: Mutex<BTreeMap<(String, String), Arc<SessionRetirement>>>,
    next_generation: AtomicU64,
    service_instance_id: String,
    stopping: AtomicBool,
}

#[derive(Default)]
struct ServiceLifecycle {
    resources_closed: bool,
    native_shutdown: Option<tokio::task::JoinHandle<Result<(), WorkspaceError>>>,
    native_closed: bool,
}

/// The original exact-Session cleanup owner survives both a dropped waiter and
/// removal of its native Workspace from the live registry. In particular, a
/// failed Profile deletion must not lose the Profile that still needs cleanup.
struct SessionRetirement {
    key: BrowserResourceKey,
    runtime_generation: Option<u64>,
    resources: Mutex<Vec<Arc<BrowserWorkspace>>>,
    profiles: Vec<BrowserProfile>,
    registry: std::sync::Weak<Mutex<BTreeMap<BrowserResourceKey, Arc<BrowserWorkspace>>>>,
    factory: Arc<dyn BrowserRuntimeFactory>,
    profile_store: Option<BrowserProfileStore>,
    delete_requested: AtomicBool,
    native_closed: AtomicBool,
    profile_deleted: AtomicBool,
    worker: Mutex<Option<tokio::task::JoinHandle<Result<(), WorkspaceError>>>>,
}

impl SessionRetirement {
    fn complete(&self) -> bool {
        self.native_closed.load(Ordering::Acquire)
            && (!self.delete_requested.load(Ordering::Acquire) || self.profile_deleted.load(Ordering::Acquire))
    }

    /// Called under the short service admission fence, before returning the
    /// owner to a waiter. The task, rather than the waiter, owns native cleanup.
    fn start(self: &Arc<Self>) {
        if let Ok(mut worker) = self.worker.try_lock() {
            if worker.is_none() && !self.complete() {
                let owner = self.clone();
                *worker = Some(tokio::spawn(async move { owner.cleanup().await }));
            }
        }
    }

    async fn settle(self: &Arc<Self>) -> Result<(), WorkspaceError> {
        loop {
            let mut worker = self.worker.lock().await;
            if worker.is_none() && !self.complete() {
                let owner = self.clone();
                *worker = Some(tokio::spawn(async move { owner.cleanup().await }));
            }
            let Some(task) = worker.as_mut() else { return Ok(()); };
            let result = task.await.unwrap_or(Err(WorkspaceError::Admission(RunAdmissionError::WorkerFailed)));
            *worker = None;
            result?;
            // A close-only flight may have finished just before another
            // caller requested deletion. Consume that upgrade on this owner.
            if self.complete() { return Ok(()); }
        }
    }

    async fn cleanup(&self) -> Result<(), WorkspaceError> {
        if !self.native_closed.load(Ordering::Acquire) {
            let mut resources = self.resources.lock().await;
            let mut failure = None;
            for resource in resources.iter() {
                if let Err(error) = resource.close().await {
                    failure = Some(error);
                    continue;
                }
                if let Some(registry) = self.registry.upgrade() {
                    let mut registry = registry.lock().await;
                    if registry.get(&resource.key).is_some_and(|current| Arc::ptr_eq(current, resource)) {
                        registry.remove(&resource.key);
                    }
                }
            }
            if let Some(error) = failure { return Err(error); }
            resources.clear();
            self.native_closed.store(true, Ordering::Release);
        }
        if !self.delete_requested.load(Ordering::Acquire) || self.profile_deleted.load(Ordering::Acquire) {
            return Ok(());
        }
        let persistent = self.profiles.iter().any(|profile| profile.persistence() == BrowserProfilePersistence::Persistent);
        let Some(store) = self.profile_store.clone() else {
            if persistent { return Err(WorkspaceError::ProfileCleanupUnavailable); }
            self.profile_deleted.store(true, Ordering::Release);
            return Ok(());
        };
        let profile = store.profile_for(&self.key, BrowserProfilePersistence::Persistent)?;
        if self.profiles.iter().any(|existing| existing.persistence() == BrowserProfilePersistence::Persistent && existing != &profile) {
            return Err(WorkspaceError::ProfileCleanupInvalid);
        }
        self.factory.delete_persistent_profile(&profile).await?;
        match profile {
            BrowserProfile::WebKitPersistent { .. } => (),
            BrowserProfile::Persistent(_) => {
                let key = self.key.clone();
                tokio::task::spawn_blocking(move || store.delete_persistent_profile(&key))
                    .await.map_err(|_| WorkspaceError::ProfileCleanupFailed)??;
            }
            BrowserProfile::Ephemeral => return Err(WorkspaceError::ProfileCleanupInvalid),
        }
        self.profile_deleted.store(true, Ordering::Release);
        Ok(())
    }
}

impl BrowserResourceService {
    pub fn service_instance_id(&self) -> &str { &self.service_instance_id }
    /// Original owner inventory only. Historical Session presence and native
    /// liveness are not inferred; no native method or lazy creation is called.
    pub async fn registered_workspace_sessions(&self, principal_id: &str) -> Result<Vec<String>, WorkspaceError> {
        let mut sessions = self.resources.lock().await.keys()
            .filter(|key| key.principal_id == principal_id)
            .map(|key| key.agent_session_id.clone()).collect::<BTreeSet<_>>();
        for ((owner, session), cleanup) in self.retired_sessions.lock().await.iter() {
            if owner == principal_id && !cleanup.complete() { sessions.insert(session.clone()); }
        }
        Ok(sessions.into_iter().collect())
    }

    /// Peek only: no runtime creation, native request, Session claim or replay.
    /// Native close can hold its mutex indefinitely; activity observations
    /// report uncertainty instead of waiting behind that original owner.
    pub async fn observe_workspace(&self, principal_id: &str, agent_session_id: &str)
        -> Result<BrowserWorkspaceObservation, WorkspaceError> {
        let Some(workspace) = self.get_for_agent_session(principal_id, agent_session_id).await? else {
            if let Some(owner) = self.retired_sessions.lock().await.get(&(principal_id.to_owned(), agent_session_id.to_owned())) {
                if !owner.complete() {
                    let closed = owner.native_closed.load(Ordering::Acquire);
                    return Ok(BrowserWorkspaceObservation { service_instance_id: Some(self.service_instance_id.clone()), registered: true, runtime_generation: owner.runtime_generation,
                        native_registered: closed.then_some(false), close_confirmed: closed.then_some(true),
                        closing: true, cleanup_unconfirmed: true, task_operation_id: None,
                        retention: BrowserRetentionReason::NativeCleanup, operation_in_flight: None });
                }
            }
            return Ok(BrowserWorkspaceObservation::unregistered());
        };
        let closing = workspace.closing.load(Ordering::Acquire);
        let (native_registered, close_confirmed) = match workspace.slot.inner.try_lock() {
            Ok(state) => (Some(state.runtime.is_some()), Some(state.closed && state.runtime.is_none())),
            Err(_) => (None, None),
        };
        let (task_operation_id, retention) = match workspace.uses.try_lock() {
            Ok(uses) => {
                let reason = if close_confirmed == Some(true) { BrowserRetentionReason::Retired }
                    else if closing { BrowserRetentionReason::NativeCleanup }
                    else if uses.task.is_some() && !uses.turn_ended { BrowserRetentionReason::ActiveTask }
                    else if !uses.surface_attachments.is_empty() || uses.presentation == Some(true) { BrowserRetentionReason::CurrentUserUse }
                    else if uses.user_pages { BrowserRetentionReason::UserPageStateUnknown }
                    else if uses.task.is_some() && !uses.task_completed { BrowserRetentionReason::TaskCompletionUnknown }
                    else { BrowserRetentionReason::EligibleTaskResources };
                (uses.task.clone(), reason)
            },
            Err(_) => (None, BrowserRetentionReason::ObservationBusy),
        };
        Ok(BrowserWorkspaceObservation { service_instance_id: Some(self.service_instance_id.clone()), registered: true, runtime_generation: Some(workspace.runtime_generation()),
            native_registered, close_confirmed, closing, cleanup_unconfirmed: closing && close_confirmed != Some(true),
            task_operation_id, retention, operation_in_flight: Some(workspace.coordinator.operation_in_flight()) })
    }

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
            resources: Arc::new(Mutex::new(BTreeMap::new())),
            lifecycle: Mutex::new(ServiceLifecycle::default()),
            retired_sessions: Mutex::new(BTreeMap::new()),
            next_generation: AtomicU64::new(1),
            service_instance_id: uuid::Uuid::now_v7().to_string(),
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
            let persistence = profile.persistence();
            if store.profile_for(&key, persistence)? != profile {
                return Err(WorkspaceError::ProfileCleanupInvalid);
            }
        }
        if self.retired_sessions.lock().await.contains_key(&(key.principal_id.clone(), key.agent_session_id.clone())) {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        let mut resources = self.resources.lock().await;
        if self.stopping.load(Ordering::Acquire) { return Err(WorkspaceError::WorkspaceClosed); }
        if let Some(workspace) = resources.get(&key) {
            if workspace.closing.load(Ordering::Acquire) { return Err(WorkspaceError::WorkspaceClosed); }
            if workspace.slot.request.profile != profile { return Err(WorkspaceError::ProfileCleanupInvalid); }
            return Ok(workspace.clone());
        }
        let uses = Arc::new(Mutex::new(WorkspaceUses::default()));
        let use_changes = Arc::new(crate::revision::BrowserRevision::default());
        let slot = Arc::new(RuntimeSlot {
            factory: self.factory.clone(),
            request: CreateBrowserRuntime {
                key: key.clone(),
                runtime_generation: self.next_generation.fetch_add(1, Ordering::Relaxed),
                profile,
                user_input_enabled: true,
            },
            inner: Mutex::new(SlotState { runtime: None, created_by: None, locked: false, native_lock_acknowledged: false, closed: false }),
            uses: uses.clone(),
            use_changes: use_changes.clone(),
        });
        let workspace = Arc::new(BrowserWorkspace {
            key: key.clone(),
            agent_handles: Mutex::new(BTreeMap::new()),
            provider: Mutex::new(None),
            coordinator: BrowserRunCoordinator::new(slot.clone()),
            slot,
            closing: AtomicBool::new(false),
            user_navigation: std::sync::Mutex::new(None),
            uses,
            use_changes,
            registry: Arc::downgrade(&self.resources),
            retirement_worker: Mutex::new(None),
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
        self.retire_agent_session(principal_id, agent_session_id, false).await?.settle().await
    }

    /// Canonical deletion includes the domain-owned user browser, even when
    /// this Session has never granted Browser actions to an Agent.
    pub async fn delete_agent_session(
        &self,
        principal_id: &str,
        agent_session_id: &str,
    ) -> Result<(), WorkspaceError> {
        self.retire_agent_session(principal_id, agent_session_id, true).await?.settle().await
    }

    /// Bootstrap dataset coordinator only. Retired canonical keys and the
    /// archived storage generation identify system-owned data after filesystem
    /// quarantine; no directory or replacement Session ledger is introduced.
    /// A live runtime prevents this offline cleanup path from running.
    pub async fn delete_retired_webkit_profile(
        &self,
        principal_id: &str,
        agent_session_id: &str,
        storage_generation: &str,
    ) -> Result<(), WorkspaceError> {
        let _lifecycle = self.lifecycle.lock().await;
        if !self.resources.lock().await.is_empty() {
            return Err(WorkspaceError::ProfileCleanupFailed);
        }
        for owner in self.retired_sessions.lock().await.values() {
            owner.settle().await?;
        }
        let store = self.profile_store.as_ref().ok_or(WorkspaceError::ProfileCleanupUnavailable)?;
        let key = managed_workspace_key(principal_id, agent_session_id)?;
        let profile = store.webkit_profile_for_generation(&key, storage_generation)?;
        self.factory.delete_persistent_profile(&profile).await
    }

    async fn retire_agent_session(
        &self,
        principal_id: &str,
        agent_session_id: &str,
        delete: bool,
    ) -> Result<Arc<SessionRetirement>, WorkspaceError> {
        let key = managed_workspace_key(principal_id, agent_session_id)?;
        let lifecycle = self.lifecycle.lock().await;
        let mut retired = self.retired_sessions.lock().await;
        let identity = (principal_id.to_owned(), agent_session_id.to_owned());
        // No new native/Profile work may begin after the global resource
        // barrier has been acknowledged and shared native shutdown may start.
        if lifecycle.resources_closed || lifecycle.native_closed {
            let owner = retired.get(&identity).filter(|owner| owner.complete()
                && (!delete || owner.profile_deleted.load(Ordering::Acquire)));
            return owner.cloned().ok_or(WorkspaceError::WorkspaceClosed);
        }
        if !retired.contains_key(&identity) {
            // Capture exact resource/Profile authority and close ingress under
            // the same short admission fence, once for this Session lifetime.
            let resources = self.resources.lock().await;
            let mut owned = Vec::new();
            let mut profiles = Vec::new();
            for (resource_key, resource) in resources.iter() {
                if resource_key.principal_id == principal_id && resource_key.agent_session_id == agent_session_id {
                    resource.closing.store(true, Ordering::Release);
                    resource.use_changes.bump();
                    profiles.push(resource.slot.request.profile.clone());
                    owned.push(resource.clone());
                }
            }
            retired.insert(identity.clone(), Arc::new(SessionRetirement {
                key, runtime_generation: owned.first().map(|resource| resource.runtime_generation()),
                resources: Mutex::new(owned), profiles,
                registry: Arc::downgrade(&self.resources), factory: self.factory.clone(),
                profile_store: self.profile_store.clone(), delete_requested: AtomicBool::new(false),
                native_closed: AtomicBool::new(false), profile_deleted: AtomicBool::new(false),
                worker: Mutex::new(None),
            }));
        }
        let owner = retired.get(&identity).expect("retired Session owner was registered");
        if delete { owner.delete_requested.store(true, Ordering::Release); }
        owner.start();
        Ok(owner.clone())
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
            } else {
                service.retry_retired_session_cleanup(&key,expected_generation).await?;
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
            } else {
                service.retry_retired_session_cleanup(&key,expected_generation).await?;
            }
            Ok(())
        }).await.map_err(|_| WorkspaceError::Admission(RunAdmissionError::WorkerFailed))?
    }

    async fn retry_retired_session_cleanup(&self, key: &BrowserResourceKey, expected_generation: u64)
        -> Result<(), WorkspaceError> {
        let owner = self.retired_sessions.lock().await
            .get(&(key.principal_id.clone(),key.agent_session_id.clone())).cloned();
        if let Some(owner) = owner.filter(|owner| !owner.complete()) {
            if owner.runtime_generation != Some(expected_generation) { return Err(WorkspaceError::StaleTarget); }
            // This is a retry of the already-authorized original deletion,
            // never permission to delete an unrelated Profile or new runtime.
            owner.settle().await?;
        }
        Ok(())
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
        // Session deletion may already have removed its native registry entry
        // while Profile cleanup is still running. Join that original owner
        // before permitting process-wide native shutdown or dataset cleanup.
        for owner in self.retired_sessions.lock().await.values() {
            if let Err(error) = owner.settle().await { failure = Some(error); }
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
        let mut lifecycle = self.lifecycle.lock().await;
        if !lifecycle.resources_closed {
            return Err(WorkspaceError::NativeCommandFailed);
        }
        if lifecycle.native_closed {
            return Ok(());
        }
        if lifecycle.native_shutdown.is_none() {
            let factory = self.factory.clone();
            lifecycle.native_shutdown =
                Some(tokio::spawn(async move { factory.shutdown().await }));
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
        | BrowserTabCommand::PauseNavigation { .. }
        | BrowserTabCommand::ResumeNavigation { .. }
        | BrowserTabCommand::Permission { .. }
        | BrowserTabCommand::Dialog { .. } => BrowserCapabilityAction::Act,
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod user_navigation_tests;
