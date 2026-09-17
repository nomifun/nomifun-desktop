//! macOS runtime: CEF request contexts and child NSViews. Windows retains its
//! independent WebView2 host. Only semantic algorithms and operation ownership
//! are shared through native ports.
use std::{collections::BTreeMap, sync::{Arc, Weak, Mutex as StdMutex, atomic::{AtomicBool, Ordering}}};
use async_trait::async_trait;
use nomifun_browser_macos::engine::{Context, Engine, ParentView};
use nomifun_browser_platform::{run_guard::{NativeInputGate, RunAdmissionError}, runtime::*};
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use super::native::{self, View};
use super::super::automation::TabAutomation;
#[path = "../pending_work.rs"]
mod pending_work;
#[path = "../evaluation.rs"]
mod evaluation;
#[path = "../screenshot.rs"]
mod screenshot;

pub struct DesktopBrowserHost { app: tauri::AppHandle, engine: Arc<Engine> }
impl DesktopBrowserHost {
    pub fn new(app: tauri::AppHandle, engine: Arc<Engine>) -> Self { Self { app, engine } }
}
#[async_trait]
impl BrowserRuntimeFactory for DesktopBrowserHost {
    async fn create(&self, request: CreateBrowserRuntime) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
        let context = self.engine.create_context(match &request.profile { BrowserProfile::Ephemeral => None, BrowserProfile::Persistent(path) => Some(path.clone()) }).await.map_err(native_error)?;
        let app = self.app.clone();
        let parent: Arc<ParentView> = Arc::new(move || {
            let window = app.get_window("main").ok_or("Browser parent window is gone")?;
            let raw = window.ns_window().map_err(|_| "Browser parent window is unavailable")?;
            let window = unsafe { raw.cast::<objc2_app_kit::NSWindow>().as_ref() }.ok_or("Browser parent window is null")?;
            window.contentView().ok_or_else(|| "Browser parent view is unavailable".into())
        });
        let input_enabled = request.user_input_enabled;
        Ok(Arc::new_cyclic(|weak| DesktopBrowserRuntime {
            weak: weak.clone(), app: self.app.clone(), engine: self.engine.clone(), context, parent, request,
            input_locked: Arc::new(AtomicBool::new(!input_enabled)), revision: Default::default(),
            creating: Mutex::new(()), site_data_view: Mutex::new(None), pending_work: Mutex::new(BTreeMap::new()), closing: CancellationToken::new(),
            state: Mutex::new(RuntimeState { tabs: BTreeMap::new(), active: None, bounds: None, visible: false, closed: false,
                input_enabled, presentation_requested: false, surface_cancel: CancellationToken::new(), surface_epoch: 0, uploads: vec![], download_files: vec![] }),
        }))
    }
}
struct NativeTab {
    close_gate: Mutex<()>, view: View, metadata: Arc<StdMutex<BrowserTabSnapshot>>,
    automation: Mutex<TabAutomation>, operation: Arc<StdMutex<Option<CancellationToken>>>,
    popup_stop: CancellationToken, rendering_capture: Arc<AtomicBool>,
}
struct RuntimeState {
    tabs: BTreeMap<String, Arc<NativeTab>>, active: Option<String>, bounds: Option<BrowserSurfaceBounds>,
    visible: bool, closed: bool, input_enabled: bool, presentation_requested: bool,
    surface_cancel: CancellationToken, surface_epoch: u64,
    uploads: Vec<Arc<nomifun_browser_platform::uploads::PreparedBrowserUpload>>,
    download_files: Vec<Arc<nomifun_browser_platform::downloads::PreparedBrowserDownload>>,
}
pub struct DesktopBrowserRuntime {
    weak: Weak<Self>, app: tauri::AppHandle, engine: Arc<Engine>, context: Arc<Context>, parent: Arc<ParentView>, request: CreateBrowserRuntime,
    input_locked: Arc<AtomicBool>, revision: Arc<nomifun_browser_platform::revision::BrowserRevision>,
    creating: Mutex<()>, site_data_view: Mutex<Option<View>>, pending_work: Mutex<BTreeMap<String, Arc<pending_work::PendingWork>>>,
    closing: CancellationToken, state: Mutex<RuntimeState>,
}
enum NativeOperation {
    Input(BrowserAction),
    Upload { element: BrowserElementRef, files: Arc<nomifun_browser_platform::uploads::PreparedBrowserUpload> },
    Download { element: BrowserElementRef, file: Arc<nomifun_browser_platform::downloads::PreparedBrowserDownload> },
}
impl NativeOperation {
    fn element(&self) -> &BrowserElementRef { match self { Self::Input(action) => action.element(), Self::Upload { element, .. } | Self::Download { element, .. } => element } }
}
struct InputScope(Arc<StdMutex<Option<CancellationToken>>>);
impl Drop for InputScope { fn drop(&mut self) { self.0.lock().unwrap_or_else(|error| error.into_inner()).take(); } }
fn native_error(error: impl std::fmt::Display) -> WorkspaceError {
    tracing::warn!(%error, "CEF browser host operation failed"); WorkspaceError::NativeCommandFailed
}
fn valid_url(input: &str) -> Result<url::Url, WorkspaceError> {
    let url = url::Url::parse(input).map_err(|_| WorkspaceError::InvalidUrl)?;
    if input.len() > 8192 || !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() { return Err(WorkspaceError::InvalidUrl); }
    Ok(url)
}

impl DesktopBrowserRuntime {
    fn request_presentation(&self, state: &mut RuntimeState) {
        if !state.input_enabled && !state.presentation_requested && state.active.is_some() {
            if state.visible || self.app.emit_to("main", "browser-workspace-open", &self.request.key.agent_session_id).is_ok() { state.presentation_requested = true; }
        }
    }
    fn snapshot_locked(&self, state: &RuntimeState) -> BrowserRuntimeSnapshot {
        BrowserRuntimeSnapshot { runtime_generation: self.request.runtime_generation, revision: self.revision.current(), active_tab_id: state.active.clone(),
            tabs: state.tabs.values().map(|tab| tab.metadata.lock().unwrap().clone()).collect(), downloads: vec![] }
    }
    fn target<'a>(&self, state: &'a RuntimeState, target: &BrowserTabTarget) -> Result<&'a Arc<NativeTab>, WorkspaceError> {
        let tab = state.tabs.get(&target.tab_id).ok_or(WorkspaceError::TabNotFound)?;
        if tab.metadata.lock().unwrap().target != *target { return Err(WorkspaceError::StaleTarget); }
        Ok(tab)
    }
    async fn apply_surface(&self, state: &RuntimeState) -> Result<(), WorkspaceError> {
        for (id, tab) in &state.tabs {
            if tab.popup_stop.is_cancelled() { continue; }
            if let Some(bounds) = state.bounds {
                tab.view.page.set_surface(bounds, state.visible && state.active.as_ref() == Some(id), state.surface_cancel.clone()).await.map_err(native_error)?;
            }
        }
        Ok(())
    }
    async fn create_site_data_view(&self) -> Result<View, WorkspaceError> {
        let mut retained = self.site_data_view.lock().await;
        if let Some(view) = retained.as_ref() { return Ok(view.clone()); }
        let view = View::new(self.engine.create_page(self.parent.clone(), self.context.clone()).await.map_err(native_error)?);
        *retained = Some(view.clone());
        native::protocol_call(&view, "Page.enable", serde_json::json!({})).await.map_err(native_error)?;
        Ok(view)
    }
    async fn close_site_data_view(&self) -> Result<(), WorkspaceError> {
        let mut retained = self.site_data_view.lock().await;
        if let Some(view) = retained.as_ref() { view.page.force_close().await.map_err(native_error)?; }
        retained.take(); Ok(())
    }
    async fn retire_native_tab(&self, tab: &Arc<NativeTab>) -> Result<(), WorkspaceError> {
        let _close = tab.close_gate.lock().await;
        tab.popup_stop.cancel();
        tab.view.page.force_close().await.map_err(native_error)?;
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
        let metadata = Arc::new(StdMutex::new(BrowserTabSnapshot { target: BrowserTabTarget { tab_id: id.clone(), runtime_generation: self.request.runtime_generation, document_generation: 0 },
            title: String::new(), url: String::new(), lifecycle: BrowserTabLifecycle::Loading, can_go_back: false, can_go_forward: false,
            blocked_permissions: vec![], permission_requests: vec![], script_dialog: None, diagnostics: BrowserDiagnostics { unavailable: true, ..Default::default() } }));
        let target_metadata = metadata.clone(); let weak_page = Arc::downgrade(&page); let revision = self.revision.clone();
        let changed: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let Some(page) = weak_page.upgrade() else { return; };
            let snapshot = page.snapshot(); let mut metadata = target_metadata.lock().unwrap();
            if metadata.target.document_generation != snapshot.document_generation { metadata.diagnostics.clear_page(); }
            metadata.target.document_generation = snapshot.document_generation;
            metadata.url = snapshot.url; metadata.title = snapshot.title; metadata.lifecycle = snapshot.lifecycle;
            metadata.can_go_back = snapshot.can_go_back; metadata.can_go_forward = snapshot.can_go_forward;
            metadata.blocked_permissions = snapshot.blocked_permissions;
            metadata.script_dialog = snapshot.dialog.map(|dialog| BrowserDialog { request_id: dialog.request_id, target: metadata.target.clone(), kind: dialog.kind,
                message: dialog.message, default_text: dialog.default_text, origin: dialog.origin, text_truncated: dialog.text_truncated });
            drop(metadata); revision.bump();
        });
        page.set_change_listener(changed.clone()); changed();
        let tab = Arc::new(NativeTab { close_gate: Mutex::new(()), view: View::new(page.clone()), metadata, automation: Mutex::new(Default::default()),
            operation: Arc::new(StdMutex::new(None)), popup_stop: CancellationToken::new(), rendering_capture: Arc::new(AtomicBool::new(false)) });
        {
            let mut state = self.state.lock().await;
            if state.closed || self.closing.is_cancelled() || cancel.is_cancelled() {
                drop(state); page.force_close().await.map_err(native_error)?; return Err(RunAdmissionError::Cancelled.into());
            }
            // Register ownership before initialization commands can fail or open
            // a dialog. A failed native tab remains available for explicit close.
            state.tabs.insert(id.clone(), tab.clone()); state.active = Some(id);
            native::set_user_input_enabled(&tab.view, state.input_enabled).await.map_err(native_error)?;
            page.set_dialog_draining(false).await.map_err(native_error)?;
            self.apply_surface(&state).await?; self.request_presentation(&mut state);
        }
        // window.close is a native lifecycle event, not a renderer command.
        // Retire it only after CEF's destruction acknowledgement, just like an
        // explicit host close. Weak owners avoid keeping dead runtimes alive.
        let mut closed = page.closed();
        let weak = self.weak.clone(); let weak_tab = Arc::downgrade(&tab);
        tokio::spawn(async move {
            while !*closed.borrow_and_update() { if closed.changed().await.is_err() { return; } }
            if let (Some(runtime), Some(tab)) = (weak.upgrade(), weak_tab.upgrade()) {
                let _ = runtime.retire_native_tab(&tab).await;
            }
        });
        native::protocol_call(&tab.view, "Page.enable", serde_json::json!({})).await.map_err(native_error)?;
        navigation_command(&tab.view, "Page.navigate", serde_json::json!({"url":url.as_str()}), cancel, &self.closing).await?;
        self.revision.bump(); Ok(())
    }
}

#[async_trait]
impl BrowserRuntime for DesktopBrowserRuntime {
    fn changes(&self) -> Option<tokio::sync::watch::Receiver<u64>> { Some(self.revision.subscribe()) }
    fn automation(&self) -> Option<&dyn BrowserAutomationPort> { Some(self) }
    fn surface(&self) -> Option<&dyn BrowserNativeSurfacePort> { Some(self) }
    async fn snapshot(&self) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        let state = self.state.lock().await; if state.closed { return Err(WorkspaceError::WorkspaceClosed); } Ok(self.snapshot_locked(&state))
    }
    async fn execute(&self, command: BrowserTabCommand, cancel: CancellationToken) -> Result<BrowserRuntimeSnapshot, WorkspaceError> { self.execute_owned(command, cancel).await }
    async fn close(&self) -> Result<(), WorkspaceError> {
        self.closing.cancel(); self.cancel_pending_for_close().await;
        { let mut state = self.state.lock().await; state.closed = true; state.surface_cancel.cancel(); }
        let _creation = self.creating.lock().await;
        self.close_site_data_view().await?;
        let tabs: Vec<_> = self.state.lock().await.tabs.values().cloned().collect();
        for tab in tabs { self.retire_native_tab(&tab).await?; }
        self.retire_pending_after_close().await;
        let mut state = self.state.lock().await;
        for files in &state.uploads { files.close()?; } state.uploads.clear();
        for file in &state.download_files { file.close()?; } state.download_files.clear();
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
        for tab in &tabs { failed |= native::set_user_input_enabled(&tab.view, false).await.is_err(); }
        for tab in tabs {
            if tab.metadata.lock().unwrap().lifecycle != BrowserTabLifecycle::Crashed {
                failed |= tab.automation.lock().await.configure_file_choosers(&tab.view, true).await.is_err();
            }
        }
        if failed { Err(RunAdmissionError::InputGateFailed) } else { Ok(()) }
    }
    async fn release_pressed_input(&self) -> Result<(), RunAdmissionError> {
        self.settle_pending_work().await?;
        let tabs: Vec<_> = self.state.lock().await.tabs.values().cloned().collect();
        for tab in tabs {
            let mut driver = tab.automation.lock().await;
            if tab.metadata.lock().unwrap().lifecycle == BrowserTabLifecycle::Crashed { *driver = Default::default(); }
            else { driver.settle_agent(&tab.view).await.map_err(|_| RunAdmissionError::InputGateFailed)?; }
        }
        Ok(())
    }
    async fn unlock_user_input(&self) -> Result<(), RunAdmissionError> {
        let mut state = self.state.lock().await;
        if state.closed { return Ok(()); }
        for tab in state.tabs.values() { native::script_dialogs::resume(&tab.view).await.map_err(|_| RunAdmissionError::InputGateFailed)?; }
        for tab in state.tabs.values() {
            if native::set_user_input_enabled(&tab.view, true).await.is_err() {
                for tab in state.tabs.values() { let _ = tab.view.page.set_input_locked(true).await; }
                return Err(RunAdmissionError::InputGateFailed);
            }
        }
        state.input_enabled = true; self.input_locked.store(false, Ordering::Release); Ok(())
    }
}

async fn navigation_command(
    view: &View,
    method: &str,
    params: serde_json::Value,
    cancel: &CancellationToken,
    closing: &CancellationToken,
) -> Result<(), WorkspaceError> {
    if closing.is_cancelled() {
        return Err(WorkspaceError::WorkspaceClosed);
    }
    if cancel.is_cancelled() {
        return Err(RunAdmissionError::Cancelled.into());
    }
    let navigation = native::protocol_call(view, method, params);
    tokio::pin!(navigation);
    // Poll navigation first: if cancellation wins, the command has already
    // been submitted and stopLoading is ordered after it on the UI thread.
    let result = tokio::select! {
        biased;
        result=&mut navigation=>Some(result),
        _=cancel.cancelled()=>None,
        _=closing.cancelled()=>None,
    };
    if cancel.is_cancelled() || closing.is_cancelled() || result.is_none() {
        let stop = native::protocol_call(view, "Page.stopLoading", serde_json::json!({})).await;
        if result.is_none() {
            let _ = navigation.await;
        }
        stop.map_err(native_error)?;
        if closing.is_cancelled() {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        return Err(RunAdmissionError::Cancelled.into());
    }
    result
        .ok_or(WorkspaceError::NativeCommandFailed)?
        .map_err(native_error)?;
    Ok(())
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
            return self.apply_surface(&state).await;
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
            return self.apply_surface(&state).await;
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
    async fn upload(&self,element:BrowserElementRef,files:Arc<nomifun_browser_platform::uploads::PreparedBrowserUpload>,cancel:CancellationToken)->Result<BrowserActionResult,WorkspaceError> {
        self.perform_action(NativeOperation::Upload{element,files},cancel).await
    }
    async fn download(&self,element:BrowserElementRef,file:Arc<nomifun_browser_platform::downloads::PreparedBrowserDownload>,cancel:CancellationToken)->Result<BrowserActionResult,WorkspaceError> {
        self.perform_action(NativeOperation::Download{element,file},cancel).await
    }
}

impl DesktopBrowserRuntime {
    async fn evaluate_inner(&self, request: BrowserEvaluation, cancel: CancellationToken) -> Result<BrowserEvaluationResult, WorkspaceError> {
        let tab = {
            let state = self.state.lock().await;
            if state.closed { return Err(WorkspaceError::WorkspaceClosed); }
            if state.input_enabled { return Err(RunAdmissionError::StaleRun.into()); }
            self.target(&state, &request.target)?.clone()
        };
        let mut automation = tab.automation.lock().await;
        let _scope = InputScope(tab.operation.clone());
        {
            let mut state = self.state.lock().await;
            if state.closed { return Err(WorkspaceError::WorkspaceClosed); }
            if state.input_enabled { return Err(RunAdmissionError::StaleRun.into()); }
            if cancel.is_cancelled() { return Err(RunAdmissionError::Cancelled.into()); }
            if !Arc::ptr_eq(self.target(&state, &request.target)?, &tab) { return Err(WorkspaceError::StaleTarget); }
            state.active = Some(request.target.tab_id.clone());
            *tab.operation.lock().unwrap_or_else(|e|e.into_inner()) = Some(cancel.clone());
            self.apply_surface(&state).await?;
            self.request_presentation(&mut state);
        }
        automation.activate_for_agent(&tab.view, &cancel).await?;
        automation.invalidate_observation();
        evaluation::run(&tab.view, tab.metadata.clone(), request, &cancel).await
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
        // The operation keeps the exact native tab alive, without blocking
        // registry reads or a popup's new-tab admission on its native callback.
        automation.activate_for_agent(&tab.view, &cancel).await?;
        let download = None;
        let fidelity=match operation {
            NativeOperation::Download { element, file } => {
                let _ = (element, file);
                return Err(WorkspaceError::UnsupportedAction);
            }
            NativeOperation::Input(action)=>{automation.act(&tab.view,action,&cancel).await?;InteractionFidelity::BrowserInput}
            NativeOperation::Upload{element,files}=>{
                if files.file_count()>0 {
                    use nomifun_browser_platform::uploads::{MAX_RETAINED_UPLOAD_BYTES,MAX_RETAINED_UPLOAD_FILES};
                    let mut state=self.state.lock().await;
                    let bytes=state.uploads.iter().map(|files|files.total_bytes()).sum::<u64>();
                    let count=state.uploads.iter().map(|files|files.file_count()).sum::<usize>();
                    if bytes.saturating_add(files.total_bytes())>MAX_RETAINED_UPLOAD_BYTES || count.saturating_add(files.file_count())>MAX_RETAINED_UPLOAD_FILES || state.uploads.len()>=128 {return Err(WorkspaceError::UploadLimit);}
                    // A File can be read by the page well after this Tool call.
                    // Retain even on uncertain protocol failure, until native close.
                    state.uploads.push(files.clone());
                }
                automation.upload(&tab.view,element,&files,&cancel).await?;
                InteractionFidelity::BrowserProtocol
            }
        };
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
            download,
            target,
            interaction_fidelity: fidelity,
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
        let mut automation=tab.automation.lock().await;
        let target=tab.metadata.lock().unwrap_or_else(|error|error.into_inner()).target.clone();
        {
            let state=self.state.lock().await;
            if state.closed { return Err(WorkspaceError::WorkspaceClosed); }
            if state.input_enabled { return Err(RunAdmissionError::StaleRun.into()); }
            if !Arc::ptr_eq(self.target(&state,&target)?,&tab) { return Err(WorkspaceError::StaleTarget); }
        }
        automation.activate_for_agent(&tab.view,&cancel).await?;
        let captured=screenshot::capture(&tab.view,target.clone(),&cancel,tab.rendering_capture.clone()).await;
        // Capture rendering never owns surface layout. Apply the latest layout
        // after its lease ends, including hides/activations that arrived in flight.
        { let state=self.state.lock().await; self.apply_surface(&state).await?; }
        let result=captured?;
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
        automation.activate_for_agent(&tab.view, &cancel).await?;
        let result = automation
            .observe(&tab.view, target.clone(), &cancel)
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
            BrowserTabCommand::OpenDownloads { .. } | BrowserTabCommand::OpenExternal { .. }
                | BrowserTabCommand::CancelDownload { .. } | BrowserTabCommand::Permission { .. } => return Err(WorkspaceError::UnsupportedAction),
            BrowserTabCommand::Create { url } => {
                drop(state);
                self.create_tab(&url, &cancel, &scope).await?;
                state = self.state.lock().await;
            }
            BrowserTabCommand::Activate { target } => {
                state.active = Some(target.tab_id);
                self.apply_surface(&state).await?;
            }
            BrowserTabCommand::Close {..} | BrowserTabCommand::CloseAll {..} | BrowserTabCommand::ClearSiteData {..} => unreachable!("close is handled before native input locks"),
            BrowserTabCommand::Navigate { target, url } => {
                let url = valid_url(&url)?;
                let tab = self.target(&state, &target)?.clone();
                drop(state);
                navigation_command(
                    &tab.view,
                    "Page.navigate",
                    serde_json::json!({"url":url.as_str()}),
                    &cancel,
                    &self.closing,
                )
                .await?;
                state = self.state.lock().await;
            }
            BrowserTabCommand::Reload { target } => {
                let tab = self.target(&state, &target)?.clone();
                drop(state);
                navigation_command(
                    &tab.view,
                    "Page.reload",
                    serde_json::json!({}),
                    &cancel,
                    &self.closing,
                )
                .await?;
                state = self.state.lock().await;
            }
            BrowserTabCommand::StopLoading { target } => {
                let tab = self.target(&state, &target)?.clone();
                drop(state);
                native::protocol_call(&tab.view, "Page.stopLoading", serde_json::json!({}))
                    .await
                    .map_err(native_error)?;
                state = self.state.lock().await;
            }
            BrowserTabCommand::Back { target } | BrowserTabCommand::Forward { target } => {
                let tab = self.target(&state, &target)?.clone();
                drop(state);
                let history = native::protocol_call(
                    &tab.view,
                    "Page.getNavigationHistory",
                    serde_json::json!({}),
                )
                .await
                .map_err(native_error)?;
                let current = history["currentIndex"]
                    .as_i64()
                    .ok_or(WorkspaceError::NativeCommandFailed)?;
                let entries = history["entries"]
                    .as_array()
                    .ok_or(WorkspaceError::NativeCommandFailed)?;
                let next = if backwards { current - 1 } else { current + 1 };
                if let Some(entry) = usize::try_from(next)
                    .ok()
                    .and_then(|next| entries.get(next))
                {
                    navigation_command(
                        &tab.view,
                        "Page.navigateToHistoryEntry",
                        serde_json::json!({"entryId":entry["id"]}),
                        &cancel,
                        &self.closing,
                    )
                    .await?;
                }
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
