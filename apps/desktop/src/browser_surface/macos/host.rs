//! WKWebView pages in the existing desktop NSWindow. Native callbacks own
//! lifecycle completion; Agent input uses the isolated semantic DOM world.
use std::{collections::BTreeMap, sync::{Arc, Weak, Mutex as StdMutex, atomic::{AtomicBool, Ordering}}};
use async_trait::async_trait;
use nomifun_browser_macos::engine::{Context, Engine, NavigationCommand as Navigation, Page, ParentView, PopupCandidate};
use nomifun_browser_platform::{run_guard::{NativeInputGate, RunAdmissionError}, runtime::*};
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use super::native::{self, View};
use super::semantic::TabAutomation;
use base64::Engine as _;
mod pending_work;

pub struct DesktopBrowserHost { app: tauri::AppHandle, engine: Arc<super::lifecycle::DeferredEngine> }
impl DesktopBrowserHost {
    pub fn new(app: tauri::AppHandle) -> Self { Self { app, engine: Arc::new(Default::default()) } }
}
#[async_trait]
impl BrowserRuntimeFactory for DesktopBrowserHost {
    async fn create(&self, request: CreateBrowserRuntime) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
        let engine = self.engine.get(&self.app).await.map_err(|_| WorkspaceError::NativeInitializationFailed)?;
        let identifier = match &request.profile {
            BrowserProfile::Ephemeral => None,
            BrowserProfile::WebKitPersistent { identifier } => Some(uuid::Uuid::from_bytes(*identifier)),
            BrowserProfile::Persistent(_) => return Err(WorkspaceError::ProfileCleanupInvalid),
        };
        let context = engine.create_context(identifier).await.map_err(native_error)?;
        let app = self.app.clone();
        let parent: Arc<ParentView> = Arc::new(move || {
            let window = app.get_window("main").ok_or("Browser parent window is gone")?;
            let raw = window.ns_window().map_err(|_| "Browser parent window is unavailable")?;
            let window = unsafe { raw.cast::<objc2_app_kit::NSWindow>().as_ref() }.ok_or("Browser parent window is null")?;
            window.contentView().ok_or_else(|| "Browser parent view is unavailable".into())
        });
        let input_enabled = request.user_input_enabled;
        Ok(Arc::new_cyclic(|weak| DesktopBrowserRuntime {
            weak: weak.clone(), app: self.app.clone(), engine, context, parent, request,
            input_locked: Arc::new(AtomicBool::new(!input_enabled)), revision: Default::default(),
            creating: Mutex::new(()), pending_work: Mutex::new(BTreeMap::new()), closing: CancellationToken::new(),
            state: Mutex::new(RuntimeState { tabs: BTreeMap::new(), active: None, bounds: None, visible: false, closed: false,
                input_enabled, presentation_requested: false, surface_cancel: CancellationToken::new(), surface_epoch: 0 }),
        }))
    }
    async fn delete_persistent_profile(&self, profile: &BrowserProfile) -> Result<(), WorkspaceError> {
        match profile {
            BrowserProfile::WebKitPersistent { identifier } => Engine::remove_data_store(uuid::Uuid::from_bytes(*identifier)).await.map_err(|_| WorkspaceError::ProfileCleanupFailed),
            _ => Err(WorkspaceError::ProfileCleanupInvalid),
        }
    }
    async fn shutdown(&self) -> Result<(), WorkspaceError> { self.engine.shutdown().await.map_err(native_error) }
}
struct NativeTab {
    close_gate: Mutex<()>, view: View, metadata: Arc<StdMutex<BrowserTabSnapshot>>,
    automation: Arc<Mutex<TabAutomation>>, operation: Arc<StdMutex<Option<CancellationToken>>>,
    popup_stop: CancellationToken, popup_work: Arc<Mutex<()>>,
    popup_cleanup: Arc<Mutex<Vec<Arc<Page>>>>,
}
struct RuntimeState {
    tabs: BTreeMap<String, Arc<NativeTab>>, active: Option<String>, bounds: Option<BrowserSurfaceBounds>,
    visible: bool, closed: bool, input_enabled: bool, presentation_requested: bool,
    surface_cancel: CancellationToken, surface_epoch: u64,
}
pub struct DesktopBrowserRuntime {
    weak: Weak<Self>, app: tauri::AppHandle, engine: Arc<Engine>, context: Arc<Context>, parent: Arc<ParentView>, request: CreateBrowserRuntime,
    input_locked: Arc<AtomicBool>, revision: Arc<nomifun_browser_platform::revision::BrowserRevision>,
    creating: Mutex<()>, pending_work: Mutex<BTreeMap<String, Arc<pending_work::PendingWork>>>,
    closing: CancellationToken, state: Mutex<RuntimeState>,
}
enum NativeOperation { Input(BrowserAction) }
impl NativeOperation {
    fn element(&self) -> &BrowserElementRef { match self { Self::Input(action) => action.element() } }
}
struct InputScope(Arc<StdMutex<Option<CancellationToken>>>);
impl Drop for InputScope { fn drop(&mut self) { self.0.lock().unwrap_or_else(|error| error.into_inner()).take(); } }
fn native_error(error: impl std::fmt::Display) -> WorkspaceError {
    // Native error messages may contain page data. Do not log them.
    match error.to_string().as_str() {
        "BROWSER_CANCELLED" => RunAdmissionError::Cancelled.into(),
        "BROWSER_ACTION_INTERRUPTED" => WorkspaceError::ActionInterrupted,
        "BROWSER_EXECUTION_UNCONFIRMED" => RunAdmissionError::WorkerFailed.into(),
        "BROWSER_STALE_OBSERVATION" => WorkspaceError::StaleObservation,
        "BROWSER_UNSUPPORTED_ACTION" => WorkspaceError::UnsupportedAction,
        "BROWSER_ELEMENT_NOT_ACTIONABLE" => WorkspaceError::NotActionable,
        _ => WorkspaceError::NativeCommandFailed,
    }
}
fn valid_url(input: &str) -> Result<url::Url, WorkspaceError> {
    let url = url::Url::parse(input).map_err(|_| WorkspaceError::InvalidUrl)?;
    if input.len() > 8192 || !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() { return Err(WorkspaceError::InvalidUrl); }
    Ok(url)
}

impl DesktopBrowserRuntime {
    fn build_native_tab(&self, page: Arc<Page>) -> Arc<NativeTab> {
        let id = format!("browser-{}", page.id());
        let metadata = Arc::new(StdMutex::new(BrowserTabSnapshot {
            target: BrowserTabTarget {
                tab_id: id,
                runtime_generation: self.request.runtime_generation,
                document_generation: 0,
            },
            title: String::new(),
            url: String::new(),
            lifecycle: BrowserTabLifecycle::Loading,
            can_go_back: false,
            can_go_forward: false,
            zoom_percent: 100,
            blocked_permissions: vec![],
            permission_requests: vec![],
            script_dialog: None,
            diagnostics: BrowserDiagnostics { unavailable: true, ..Default::default() },
        }));
        let target_metadata = metadata.clone();
        let weak_page = Arc::downgrade(&page);
        let revision = self.revision.clone();
        let changed: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let Some(page) = weak_page.upgrade() else { return; };
            let snapshot = page.snapshot();
            let mut metadata = target_metadata.lock().unwrap();
            if metadata.target.document_generation != snapshot.document_generation {
                metadata.diagnostics.clear_page();
            }
            metadata.target.document_generation = snapshot.document_generation;
            metadata.url = snapshot.url;
            metadata.title = snapshot.title;
            metadata.lifecycle = snapshot.lifecycle;
            metadata.can_go_back = snapshot.can_go_back;
            metadata.can_go_forward = snapshot.can_go_forward;
            metadata.blocked_permissions = snapshot.blocked_permissions;
            metadata.permission_requests = snapshot.permission_requests;
            metadata.script_dialog = snapshot.dialog.map(|dialog| BrowserDialog {
                request_id: dialog.request_id,
                target: metadata.target.clone(),
                kind: dialog.kind,
                message: dialog.message,
                default_text: dialog.default_text,
                origin: dialog.origin,
                text_truncated: dialog.text_truncated,
            });
            drop(metadata);
            revision.bump();
        });
        page.set_change_listener(changed.clone());
        changed();
        Arc::new(NativeTab {
            close_gate: Mutex::new(()),
            view: View::new(page),
            metadata,
            automation: Arc::new(Mutex::new(Default::default())),
            operation: Arc::new(StdMutex::new(None)),
            popup_stop: self.closing.child_token(),
            popup_work: Arc::new(Mutex::new(())),
            popup_cleanup: Arc::new(Mutex::new(Vec::new())),
        })
    }

    async fn initialize_native_tab(&self, tab: &Arc<NativeTab>) -> Result<(), WorkspaceError> {
        let input_enabled = self.state.lock().await.input_enabled;
        native::set_native_user_input_enabled(&tab.view, input_enabled).await.map_err(native_error)?;
        tab.view.page.set_dialog_draining(false).await.map_err(native_error)?;
        tab.view.page.configure_user_downloads(self.app.path().download_dir().map_err(native_error)?).await.map_err(native_error)?;
        self.install_popup_worker(tab)?;

        let mut closed = tab.view.page.closed();
        let weak = self.weak.clone();
        let weak_tab = Arc::downgrade(tab);
        tokio::spawn(async move {
            while !*closed.borrow_and_update() {
                if closed.changed().await.is_err() {
                    return;
                }
            }
            if let (Some(runtime), Some(tab)) = (weak.upgrade(), weak_tab.upgrade()) {
                let _ = runtime.retire_native_tab(&tab).await;
            }
        });
        Ok(())
    }

    fn install_popup_worker(&self, tab: &Arc<NativeTab>) -> Result<(), WorkspaceError> {
        let mut popups = tab.view.page.listen_popups().map_err(native_error)?;
        let weak = self.weak.clone();
        let stop = tab.popup_stop.clone();
        let work = tab.popup_work.clone();
        let cleanup = tab.popup_cleanup.clone();
        tokio::spawn(async move {
            let _work = work.lock().await;
            loop {
                let candidate = tokio::select! {
                    biased;
                    _ = stop.cancelled() => break,
                    candidate = popups.recv() => candidate,
                };
                let Some(candidate) = candidate else { break; };
                let page = candidate.page.clone();
                if let Some(runtime) = weak.upgrade() {
                    if let Err(error) = runtime.consume_popup(candidate, &stop).await {
                        tracing::debug!(code = error.code(), "macOS WK popup was not admitted");
                        if page.force_close().await.is_err() { cleanup.lock().await.push(page); }
                    }
                } else {
                    let _ = candidate.ready.await;
                    if page.force_close().await.is_err() { cleanup.lock().await.push(page); }
                }
            }
            // Queued popups already own real WK views. Closing the receiver and
            // relying on Drop is not an acknowledged Profile-cleanup barrier.
            while let Ok(candidate) = popups.try_recv() {
                let page = candidate.page.clone();
                let _ = candidate.ready.await;
                if page.force_close().await.is_err() { cleanup.lock().await.push(page); }
            }
        });
        Ok(())
    }

    async fn consume_popup(self: &Arc<Self>, candidate: PopupCandidate, opener_stop: &CancellationToken) -> Result<(), WorkspaceError> {
        let page = candidate.page.clone();
        if candidate.ready.await.map_err(native_error)?.is_err() {
            return Err(WorkspaceError::NativeCommandFailed);
        }
        let _creation = tokio::select! { biased;
            _ = opener_stop.cancelled() => {
                page.force_close().await.map_err(native_error)?;
                return Err(WorkspaceError::WorkspaceClosed);
            }
            guard = self.creating.lock() => guard,
        };
        let tab = self.build_native_tab(page.clone());
        let id = tab.metadata.lock().unwrap().target.tab_id.clone();
        {
            let mut state = self.state.lock().await;
            if state.closed || self.closing.is_cancelled() || opener_stop.is_cancelled() || state.tabs.len() >= 8 {
                drop(state);
                page.force_close().await.map_err(native_error)?;
                return Err(if self.closing.is_cancelled() {
                    WorkspaceError::WorkspaceClosed
                } else {
                    WorkspaceError::TabLimit
                });
            }
            state.tabs.insert(id.clone(), tab.clone());
            state.active = Some(id);
        }
        if let Err(error) = self.initialize_native_tab(&tab).await {
            self.retire_native_tab(&tab).await?;
            return Err(error);
        }
        {
            let mut state = self.state.lock().await;
            self.apply_surface(&state).await?;
            self.request_presentation(&mut state);
        }
        self.revision.bump();
        Ok(())
    }

    fn request_presentation(&self, state: &mut RuntimeState) {
        if !state.input_enabled && !state.presentation_requested && state.active.is_some() {
            if state.visible || self.app.emit_to("main", "browser-workspace-open", &self.request.key.agent_session_id).is_ok() { state.presentation_requested = true; }
        }
    }

    /// Wait for the renderer's acknowledged native layout before evaluating an
    /// Agent operation. Emitting the open event alone does not prove that WK is
    /// visible, and a pending layout must never wait behind the operation mutex.
    async fn present_for_agent(&self, tab: &Arc<NativeTab>, cancel: &CancellationToken) -> Result<(), WorkspaceError> {
        if cancel.is_cancelled() { return Err(RunAdmissionError::Cancelled.into()); }
        let mut changed = self.revision.subscribe();
        let id = tab.metadata.lock().unwrap().target.tab_id.clone();
        {
            let mut state = self.state.lock().await;
            if cancel.is_cancelled() { return Err(RunAdmissionError::Cancelled.into()); }
            if state.closed { return Err(WorkspaceError::WorkspaceClosed); }
            if state.input_enabled { return Err(RunAdmissionError::StaleRun.into()); }
            if !state.tabs.get(&id).is_some_and(|current| Arc::ptr_eq(current, tab)) { return Err(WorkspaceError::TabNotFound); }
            state.active = Some(id.clone());
            // A prior shown panel may have been collapsed during this run.
            if !state.visible { state.presentation_requested = false; }
            self.request_presentation(&mut state);
            self.apply_surface(&state).await?;
        }
        let visible = async {
            loop {
                {
                    let state = self.state.lock().await;
                    if state.closed { return Err(WorkspaceError::WorkspaceClosed); }
                    if state.input_enabled { return Err(RunAdmissionError::StaleRun.into()); }
                    if !state.tabs.get(&id).is_some_and(|current| Arc::ptr_eq(current, tab)) { return Err(WorkspaceError::TabNotFound); }
                    if state.visible && state.active.as_ref() == Some(&id) && state.bounds.is_some() && !state.surface_cancel.is_cancelled() { return Ok(()); }
                }
                tokio::select! { biased;
                    _ = cancel.cancelled() => return Err(RunAdmissionError::Cancelled.into()),
                    _ = self.closing.cancelled() => return Err(WorkspaceError::WorkspaceClosed),
                    result = changed.changed() => { result.map_err(|_| WorkspaceError::NativeCommandFailed)?; }
                }
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), visible).await.map_err(|_| WorkspaceError::NotActionable)?
    }
    fn snapshot_locked(&self, state: &RuntimeState) -> BrowserRuntimeSnapshot {
        BrowserRuntimeSnapshot { runtime_generation: self.request.runtime_generation, revision: self.revision.current(), active_tab_id: state.active.clone(),
            tabs: state.tabs.values().map(|tab| tab.metadata.lock().unwrap().clone()).collect(),
            downloads: state.tabs.values().flat_map(|tab|tab.view.page.user_download_snapshot()).collect() }
    }
    fn target<'a>(&self, state: &'a RuntimeState, target: &BrowserTabTarget) -> Result<&'a Arc<NativeTab>, WorkspaceError> {
        let tab = state.tabs.get(&target.tab_id).ok_or(WorkspaceError::TabNotFound)?;
        if tab.metadata.lock().unwrap().target != *target { return Err(WorkspaceError::StaleTarget); }
        Ok(tab)
    }
    async fn apply_surface(&self, state: &RuntimeState) -> Result<(), WorkspaceError> {
        // Teardown owns destruction, not layout. Its cancelled surface lease
        // must not abort after the first tab while others still need closing.
        if state.closed || self.closing.is_cancelled() { return Ok(()); }
        for (id, tab) in &state.tabs {
            if tab.popup_stop.is_cancelled() { continue; }
            if let Some(bounds) = state.bounds {
                let visible = state.visible && state.active.as_ref() == Some(id);
                tab.view.page.set_surface(bounds, visible, state.surface_cancel.clone()).await.map_err(native_error)?;
            }
        }
        Ok(())
    }
    async fn retire_native_tab(&self, tab: &Arc<NativeTab>) -> Result<(), WorkspaceError> {
        let _close = tab.close_gate.lock().await;
        tab.popup_stop.cancel();
        // A website close or cancelled evaluation may already have released
        // WK. force_close is the idempotent owner barrier and also drains its
        // downloads; no preliminary page mutation may block retirement.
        tab.view
            .page
            .force_close()
            .await
            .map_err(|error| native_error(format!("WK page close failed: {error}")))?;
        let _ = tab.popup_work.lock().await;
        let mut cleanup = tab.popup_cleanup.lock().await;
        while let Some(page) = cleanup.last() {
            page.force_close().await.map_err(native_error)?;
            cleanup.pop();
        }
        let id = tab.metadata.lock().unwrap().target.tab_id.clone();
        let mut state = self.state.lock().await;
        state.tabs.remove(&id);
        if state.active.as_ref() == Some(&id) { state.active = state.tabs.keys().next().cloned(); }
        self.apply_surface(&state).await?;
        self.revision.bump(); Ok(())
    }
    async fn create_tab(&self, url: &str, cancel: &CancellationToken, scope: &Arc<StdMutex<Option<String>>>) -> Result<(), WorkspaceError> {
        let url = valid_url(url)?;
        let _creation = tokio::select! { biased;
            _ = self.closing.cancelled() => return Err(WorkspaceError::WorkspaceClosed),
            _ = cancel.cancelled() => return Err(RunAdmissionError::Cancelled.into()),
            guard = self.creating.lock() => guard,
        };
        { let state = self.state.lock().await;
            if state.closed { return Err(WorkspaceError::WorkspaceClosed); }
            if state.tabs.len() >= 8 { return Err(WorkspaceError::TabLimit); }
        }
        let page = self.engine.create_page(self.parent.clone(), self.context.clone()).await.map_err(native_error)?;
        let id = format!("browser-{}", page.id());
        *scope.lock().unwrap() = Some(id.clone());
        let tab = self.build_native_tab(page.clone());
        {
            let mut state = self.state.lock().await;
            if state.closed || self.closing.is_cancelled() || cancel.is_cancelled() {
                drop(state); page.force_close().await.map_err(native_error)?; return Err(RunAdmissionError::Cancelled.into());
            }
            // Register ownership before initialization commands can fail or open
            // a dialog. A failed native tab remains available for explicit close.
            state.tabs.insert(id.clone(), tab.clone()); state.active = Some(id);
            self.apply_surface(&state).await?; self.request_presentation(&mut state);
        }
        if let Err(error) = self.initialize_native_tab(&tab).await {
            self.retire_native_tab(&tab).await?;
            return Err(error);
        }
        navigation_command(&tab.view, Navigation::Navigate(url.to_string()), tab.view.page.snapshot().document_generation, cancel, &self.closing).await?;
        self.revision.bump(); Ok(())
    }
}

#[async_trait]
impl BrowserRuntime for DesktopBrowserRuntime {
    fn changes(&self) -> Option<tokio::sync::watch::Receiver<u64>> { Some(self.revision.subscribe()) }
    fn automation(&self) -> Option<&dyn BrowserAutomationPort> { Some(self) }
    fn interaction_capabilities(&self) -> Option<BrowserInteractionCapabilities> { Some(BrowserInteractionCapabilities::wk_webview()) }
    fn surface(&self) -> Option<&dyn BrowserNativeSurfacePort> { Some(self) }
    async fn snapshot(&self) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        let state = self.state.lock().await; if state.closed { return Err(WorkspaceError::WorkspaceClosed); } Ok(self.snapshot_locked(&state))
    }
    async fn execute(&self, command: BrowserTabCommand, cancel: CancellationToken) -> Result<BrowserRuntimeSnapshot, WorkspaceError> { self.execute_owned(command, cancel).await }
    async fn close(&self) -> Result<(), WorkspaceError> {
        self.closing.cancel(); self.cancel_pending_for_close().await;
        { let mut state = self.state.lock().await; state.closed = true; state.surface_cancel.cancel(); }
        let _creation = self.creating.lock().await;
        let tabs: Vec<_> = self.state.lock().await.tabs.values().cloned().collect();
        for tab in tabs { self.retire_native_tab(&tab).await?; }
        self.retire_pending_after_close().await?;
        self.context.close().await.map_err(native_error)?;
        Ok(())
    }
}

#[async_trait]
impl NativeInputGate for DesktopBrowserRuntime {
    async fn lock_user_input(&self) -> Result<(), RunAdmissionError> {
        let tabs = {
            let mut state = self.state.lock().await;
            if state.input_enabled { state.presentation_requested = false; }
            state.input_enabled = false; self.input_locked.store(true, Ordering::Release);
            state.tabs.values().cloned().collect::<Vec<_>>()
        };
        let mut failed = false;
        for tab in &tabs {
            failed |= native::set_native_user_input_enabled(&tab.view, false).await.is_err();
            failed |= native::script_dialogs::drain(&tab.view).await.is_err();
        }
        if failed { Err(RunAdmissionError::InputGateFailed) } else { Ok(()) }
    }
    async fn release_pressed_input(&self) -> Result<(), RunAdmissionError> {
        self.settle_pending_work().await?;
        let tabs: Vec<_> = self.state.lock().await.tabs.values().cloned().collect();
        for tab in tabs { tab.automation.lock().await.invalidate_observation(); }
        Ok(())
    }
    async fn unlock_user_input(&self) -> Result<(), RunAdmissionError> {
        let mut state = self.state.lock().await;
        if state.closed { return Ok(()); }
        let tabs: Vec<_> = state.tabs.values().cloned().collect();
        for tab in &tabs {
            if native::script_dialogs::resume(&tab.view).await.is_err() || native::set_native_user_input_enabled(&tab.view, true).await.is_err() {
                for tab in &tabs { let _ = tab.view.page.set_input_locked(true).await; }
                return Err(RunAdmissionError::InputGateFailed);
            }
        }
        state.input_enabled = true;
        self.input_locked.store(false, Ordering::Release);
        Ok(())
    }
}

async fn navigation_command(view: &View, command: Navigation, generation: u64, cancel: &CancellationToken, closing: &CancellationToken) -> Result<(), WorkspaceError> {
    if closing.is_cancelled() { return Err(WorkspaceError::WorkspaceClosed); }
    if cancel.is_cancelled() { return Err(RunAdmissionError::Cancelled.into()); }
    let dispatched = Arc::new(AtomicBool::new(false));
    let marker = dispatched.clone();
    let dispatch_cancel = cancel.clone();
    let dispatch_closing = closing.clone();
    let page = view.page.clone();
    let guard = Arc::new(move || {
        if dispatch_cancel.is_cancelled() || dispatch_closing.is_cancelled() || page.snapshot().document_generation != generation { return false; }
        marker.store(true, Ordering::Release);
        true
    });
    // Await the dispatch acknowledgement even after cancellation. The native
    // guard rejects queued work before it changes the page, and sent navigation
    // is never replayed. stopLoading does not settle script/dialog callbacks.
    let result = view.page.navigate_guarded(command, guard).await;
    if closing.is_cancelled() || cancel.is_cancelled() {
        if dispatched.load(Ordering::Acquire) { view.page.stop_loading().await.map_err(native_error)?; }
        return Err(if closing.is_cancelled() { WorkspaceError::WorkspaceClosed }
            else if dispatched.load(Ordering::Acquire) { WorkspaceError::ActionInterrupted }
            else { RunAdmissionError::Cancelled.into() });
    }
    if !dispatched.load(Ordering::Acquire) && view.page.snapshot().document_generation != generation { return Err(WorkspaceError::StaleTarget); }
    result.map_err(native_error)
}

#[async_trait]
impl BrowserNativeSurfacePort for DesktopBrowserRuntime {
    async fn set_surface(
        &self,
        bounds: BrowserSurfaceBounds,
        visible: bool,
        layout_cancel: CancellationToken,
    ) -> Result<(), WorkspaceError> {
        if layout_cancel.is_cancelled() {
            return Ok(());
        }
        if !bounds.is_valid() {
            return Err(WorkspaceError::NativeCommandFailed);
        }
        let epoch = {
            let mut state = self.state.lock().await;
            if layout_cancel.is_cancelled() {
                return Ok(());
            }
            if state.closed && visible {
                return Err(WorkspaceError::WorkspaceClosed);
            }
            state.surface_epoch = state
                .surface_epoch
                .checked_add(1)
                .ok_or(WorkspaceError::NativeCommandFailed)?;
            state.surface_epoch
        };
        if !visible {
            // A modal, conversation switch or closed pane must hide immediately.
            // Hidden bounds are recorded without resizing the active document.
            let mut state = self.state.lock().await;
            if layout_cancel.is_cancelled() || state.surface_epoch != epoch {
                return Ok(());
            }
            state.bounds = Some(bounds);
            state.visible = false;
            state.surface_cancel = layout_cancel;
            self.apply_surface(&state).await?;
            self.revision.bump();
            return Ok(());
        }
        loop {
            let selected = {
                let state = self.state.lock().await;
                if state.closed {
                    return Err(WorkspaceError::WorkspaceClosed);
                }
                state
                    .active
                    .as_ref()
                    .and_then(|id| state.tabs.get(id))
                    .cloned()
            };
            let _page = match &selected {
                Some(tab) => Some(tokio::select! {
                    biased;
                    _=layout_cancel.cancelled()=>return Ok(()),
                    page=tab.automation.lock()=>page,
                }),
                None => None,
            };
            let mut state = self.state.lock().await;
            // A newer hide/resize supersedes a request waiting on page input.
            // Never resurrect an old native surface after a modal or detach.
            if layout_cancel.is_cancelled() || state.surface_epoch != epoch {
                return Ok(());
            }
            if state.closed {
                return Err(WorkspaceError::WorkspaceClosed);
            }
            let current = state.active.as_ref().and_then(|id| state.tabs.get(id));
            let same = match (selected.as_ref(), current) {
                (None, None) => true,
                (Some(selected), Some(current)) => Arc::ptr_eq(selected, current),
                _ => false,
            };
            if !same {
                continue;
            }
            state.bounds = Some(bounds);
            state.visible = true;
            state.surface_cancel = layout_cancel;
            self.apply_surface(&state).await?;
            self.revision.bump();
            return Ok(());
        }
    }
}


#[async_trait]
impl BrowserAutomationPort for DesktopBrowserRuntime {
    async fn evaluate(&self, request: BrowserEvaluation, cancel: CancellationToken) -> Result<BrowserEvaluationResult, WorkspaceError> {
        self.evaluate_owned(request, cancel).await
    }
    async fn respond_dialog(&self, reply: BrowserDialogReply, cancel: CancellationToken) -> Result<BrowserActionResult, WorkspaceError> {
        self.reply_to_dialog(reply, cancel).await
    }
    async fn screenshot(&self, tab_id: Option<String>, cancel: CancellationToken) -> Result<BrowserScreenshot, WorkspaceError> {
        self.capture_owned(tab_id, cancel).await
    }
    async fn observe(&self, tab_id: Option<String>, cancel: CancellationToken) -> Result<BrowserObservation, WorkspaceError> {
        self.observe_owned(tab_id, cancel).await
    }
    async fn act(
        &self,
        action: BrowserAction,
        cancel: CancellationToken,
    ) -> Result<BrowserActionResult, WorkspaceError> {
        self.perform_action(NativeOperation::Input(action),cancel).await
    }

}

impl DesktopBrowserRuntime {
    async fn evaluate_inner(&self, _request: BrowserEvaluation, _cancel: CancellationToken) -> Result<BrowserEvaluationResult, WorkspaceError> {
        Err(WorkspaceError::UnsupportedAction)
    }

    async fn perform_action_inner(&self,operation:NativeOperation,cancel:CancellationToken)->Result<BrowserActionResult,WorkspaceError> {
        let expected = operation.element().target.clone();
        if let NativeOperation::Input(BrowserAction::Drag { to, .. }) = &operation {
            if to.target != expected {
                return Err(WorkspaceError::StaleObservation);
            }
        }
        let tab = {
            let state = self.state.lock().await;
            if state.closed {
                return Err(WorkspaceError::WorkspaceClosed);
            }
            if state.input_enabled {
                return Err(RunAdmissionError::StaleRun.into());
            }
            self.target(&state, &expected)?.clone()
        };
        self.present_for_agent(&tab, &cancel).await?;
        let mut automation = tab.automation.lock().await;
        let _scope = InputScope(tab.operation.clone());
        {
            let mut state = self.state.lock().await;
            if state.closed {
                return Err(WorkspaceError::WorkspaceClosed);
            }
            if state.input_enabled {
                return Err(RunAdmissionError::StaleRun.into());
            }
            if !Arc::ptr_eq(self.target(&state, &expected)?, &tab) {
                return Err(WorkspaceError::StaleTarget);
            }
            if state.visible && state.surface_cancel.is_cancelled() {
                return Err(WorkspaceError::NotActionable);
            }
            state.active = Some(expected.tab_id.clone());
            *tab.operation
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(cancel.clone());
            self.apply_surface(&state).await?;
        }
        // Keep the exact native page retained until its callback acknowledges
        // settlement. DOM actions never claim trusted hardware input.
        let NativeOperation::Input(action) = operation;
        automation.act(tab.view.page.clone(), action, &cancel, self.input_locked.clone()).await?;
        let target = tab
            .metadata
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .target
            .clone();
        let mut state = self.state.lock().await;
        if state.closed {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        if !cancel.is_cancelled() {
            self.request_presentation(&mut state);
        }
        self.revision.bump();
        Ok(BrowserActionResult {
            download: None,
            target,
            interaction_fidelity: InteractionFidelity::SemanticDom,
            outcome: BrowserActionOutcome::Completed,
        })
    }
}

impl DesktopBrowserRuntime {
    async fn screenshot_inner(&self, tab_id: Option<String>, cancel: CancellationToken) -> Result<BrowserScreenshot, WorkspaceError> {
        let tab = {
            let state=self.state.lock().await;
            if state.closed { return Err(WorkspaceError::WorkspaceClosed); }
            if state.input_enabled { return Err(RunAdmissionError::StaleRun.into()); }
            let id=tab_id.or_else(||state.active.clone()).ok_or(WorkspaceError::TabNotFound)?;
            state.tabs.get(&id).cloned().ok_or(WorkspaceError::TabNotFound)?
        };
        self.present_for_agent(&tab, &cancel).await?;
        tab.view.page.wait_bootstrap_ready(&cancel, &self.closing).await.map_err(native_error)?;
        let mut automation=tab.automation.lock().await;
        let target=tab.metadata.lock().unwrap_or_else(|error|error.into_inner()).target.clone();
        {
            let state=self.state.lock().await;
            if state.closed { return Err(WorkspaceError::WorkspaceClosed); }
            if state.input_enabled { return Err(RunAdmissionError::StaleRun.into()); }
            if !Arc::ptr_eq(self.target(&state,&target)?,&tab) { return Err(WorkspaceError::StaleTarget); }
        }
        automation.invalidate_observation();
        {
            let mut state = self.state.lock().await;
            state.active = Some(target.tab_id.clone());
            self.apply_surface(&state).await?;
            self.request_presentation(&mut state);
        }
        let png = tab.view.page.screenshot(target.document_generation, cancel.clone()).await.map_err(|error| match error.as_str() {
            "BROWSER_ACTION_INTERRUPTED" => WorkspaceError::ActionInterrupted,
            "BROWSER_STALE_OBSERVATION" => WorkspaceError::StaleTarget,
            "BROWSER_CANCELLED" => RunAdmissionError::Cancelled.into(),
            _ => WorkspaceError::NativeCommandFailed,
        })?;
        // WK snapshots are native viewport captures, never the visible UI surface.
        if png.len() > 1536 * 1024 { return Err(WorkspaceError::ObservationLimit); }
        if png.len() < 33 || &png[..8] != b"\x89PNG\r\n\x1a\n" || &png[8..16] != b"\0\0\0\x0dIHDR" { return Err(WorkspaceError::NativeCommandFailed); }
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        if width == 0 || height == 0 || width > 1600 || height > 1600 { return Err(WorkspaceError::ObservationLimit); }
        let bounds = self.state.lock().await.bounds.ok_or(WorkspaceError::NotActionable)?;
        let zoom = f64::from(tab.metadata.lock().unwrap().zoom_percent) / 100.0;
        let result = BrowserScreenshot { target: target.clone(), width, height, viewport_width: bounds.width / zoom, viewport_height: bounds.height / zoom,
            png_base64: base64::engine::general_purpose::STANDARD.encode(png) };
        if tab.metadata.lock().unwrap_or_else(|error|error.into_inner()).target!=target { return Err(WorkspaceError::StaleTarget); }
        let mut state=self.state.lock().await;
        if state.closed { return Err(WorkspaceError::WorkspaceClosed); }
        if cancel.is_cancelled() { return Err(RunAdmissionError::Cancelled.into()); }
        self.request_presentation(&mut state);
        Ok(result)
    }

    async fn observe_inner(
        &self,
        tab_id: Option<String>,
        cancel: CancellationToken,
    ) -> Result<BrowserObservation, WorkspaceError> {
        let tab = {
            let state = self.state.lock().await;
            if state.closed {
                return Err(WorkspaceError::WorkspaceClosed);
            }
            if state.input_enabled {
                return Err(RunAdmissionError::StaleRun.into());
            }
            let id = tab_id
                .or_else(|| state.active.clone())
                .ok_or(WorkspaceError::TabNotFound)?;
            state
                .tabs
                .get(&id)
                .cloned()
                .ok_or(WorkspaceError::TabNotFound)?
        };
        self.present_for_agent(&tab, &cancel).await?;
        tab.view.page.wait_bootstrap_ready(&cancel, &self.closing).await.map_err(native_error)?;
        let mut automation = tab.automation.lock().await;
        let target = {
            let state = self.state.lock().await;
            if state.closed {
                return Err(WorkspaceError::WorkspaceClosed);
            }
            if state.input_enabled {
                return Err(RunAdmissionError::StaleRun.into());
            }
            let target = tab
                .metadata
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .target
                .clone();
            if !Arc::ptr_eq(self.target(&state, &target)?, &tab) {
                return Err(WorkspaceError::StaleTarget);
            }
            target
        };
        let result = automation
            .observe(tab.view.page.clone(), target.clone(), &cancel, self.input_locked.clone())
            .await?;
        if tab
            .metadata
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .target
            != target
        {
            return Err(WorkspaceError::StaleTarget);
        }
        let mut state = self.state.lock().await;
        if state.closed {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        if !cancel.is_cancelled() {
            self.request_presentation(&mut state);
        }
        Ok(result)
    }

    async fn execute_inner(
        &self,
        command: BrowserTabCommand,
        cancel: CancellationToken,
        scope: Arc<StdMutex<Option<String>>>,
    ) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        // Existing-page commands serialize with that page's input, but must
        // never wait on its operation lock while holding the tab registry.
        let selected = if let Some(target) = command.target() {
            let state = self.state.lock().await;
            Some(self.target(&state, target)?.clone())
        } else {
            None
        };
        let _page = match &selected {
            Some(tab) => Some(tab.automation.lock().await),
            None => None,
        };
        let mut state = self.state.lock().await;
        if state.closed {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        if cancel.is_cancelled() {
            return Err(RunAdmissionError::Cancelled.into());
        }
        if let Some(target) = command.target() {
            self.target(&state, target)?;
        }
        let backwards = matches!(&command, BrowserTabCommand::Back { .. });
        match command {
            BrowserTabCommand::Dialog { .. } => unreachable!("dialog command handled before page lock"),
            BrowserTabCommand::OpenDownloads { runtime_generation } => {
                if runtime_generation != self.request.runtime_generation {
                    return Err(WorkspaceError::StaleTarget);
                }
                if !state.input_enabled {
                    return Err(WorkspaceError::NotActionable);
                }
                let path = self.app.path().download_dir().map_err(native_error)?;
                drop(state);
                native::external_browser::open_downloads(path, cancel.clone()).await?;
                state = self.state.lock().await;
            }
            BrowserTabCommand::OpenExternal { target } => {
                if !state.input_enabled || state.active.as_ref() != Some(&target.tab_id) {
                    return Err(WorkspaceError::NotActionable);
                }
                let tab = self.target(&state, &target)?.clone();
                let url = tab.metadata.lock().unwrap_or_else(|error| error.into_inner()).url.clone();
                drop(state);
                native::external_browser::open_url(url, cancel.clone()).await?;
                state = self.state.lock().await;
            }
            BrowserTabCommand::CancelDownload { target, download_id } => {
                if !state.input_enabled {
                    return Err(WorkspaceError::NotActionable);
                }
                let tab = self.target(&state, &target)?.clone();
                tab.view.page.cancel_user_download(download_id).await.map_err(native_error)?;
            }
            BrowserTabCommand::Permission { .. } => return Err(WorkspaceError::UnsupportedAction),
            BrowserTabCommand::Create { url } => {
                drop(state);
                self.create_tab(&url, &cancel, &scope).await?;
                state = self.state.lock().await;
            }
            BrowserTabCommand::Activate { target } => {
                state.active = Some(target.tab_id);
                self.apply_surface(&state).await?;
            }
            BrowserTabCommand::SetZoom { target, percent } => {
                if !(50..=200).contains(&percent) { return Err(WorkspaceError::InvalidZoom); }
                if !state.input_enabled || state.active.as_ref() != Some(&target.tab_id) {
                    return Err(WorkspaceError::NotActionable);
                }
                let tab = self.target(&state, &target)?;
                tab.view.page.set_zoom_factor(f64::from(percent) / 100.0).await.map_err(native_error)?;
                tab.metadata.lock().unwrap().zoom_percent = percent;
            }
            BrowserTabCommand::Close {..} | BrowserTabCommand::CloseAll {..} | BrowserTabCommand::ClearSiteData {..} => unreachable!("close is handled before native input locks"),
            BrowserTabCommand::Navigate { target, url } => {
                let url = valid_url(&url)?;
                let tab = self.target(&state, &target)?.clone();
                drop(state);
                navigation_command(&tab.view, Navigation::Navigate(url.to_string()), target.document_generation, &cancel, &self.closing).await?;
                state = self.state.lock().await;
            }
            BrowserTabCommand::Reload { target } => {
                let tab = self.target(&state, &target)?.clone();
                drop(state);
                navigation_command(&tab.view, Navigation::Reload, target.document_generation, &cancel, &self.closing).await?;
                state = self.state.lock().await;
            }
            BrowserTabCommand::StopLoading { target } => {
                let tab = self.target(&state, &target)?.clone();
                drop(state);
                tab.view.page.stop_loading().await.map_err(native_error)?;
                state = self.state.lock().await;
            }
            BrowserTabCommand::Back { target } | BrowserTabCommand::Forward { target } => {
                let tab = self.target(&state, &target)?.clone();
                drop(state);
                navigation_command(&tab.view, if backwards { Navigation::Back } else { Navigation::Forward }, target.document_generation, &cancel, &self.closing).await?;
                state = self.state.lock().await;
            }
        }
        self.revision.bump();
        if state.closed {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        drop(state);
        let mut state = self.state.lock().await;
        if state.closed {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        if !cancel.is_cancelled() {
            self.request_presentation(&mut state);
        }
        Ok(self.snapshot_locked(&state))
    }
}
