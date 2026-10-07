//! Desktop-owned BrowserRuntime. Native handles stay in the Tauri host.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex as StdMutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use nomifun_browser_platform::{
    run_guard::{NativeInputGate, RunAdmissionError},
    runtime::*,
};
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::windows;
use super::windows as native;
#[path = "screenshot.rs"]
mod screenshot;
#[path = "pending_work.rs"]
mod pending_work;
#[path = "evaluation.rs"]
mod evaluation;

pub struct DesktopBrowserHost {
    app: tauri::AppHandle,
    popups: bool,
}

impl DesktopBrowserHost {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app, popups: true }
    }

    /// Only the transport conformance runner manually consumes native deferrals.
    pub(crate) fn for_transport_conformance(app: tauri::AppHandle) -> Self {
        Self { app, popups: false }
    }
}

#[async_trait]
impl BrowserRuntimeFactory for DesktopBrowserHost {
    async fn create(
        &self,
        request: CreateBrowserRuntime,
    ) -> Result<Arc<dyn BrowserRuntime>, WorkspaceError> {
        let temporary = if matches!(request.profile, BrowserProfile::Ephemeral) {
            Some(
                tempfile::Builder::new()
                    .prefix("nomi-browser-v2-")
                    .tempdir()
                    .map_err(|_| WorkspaceError::NativeCommandFailed)?,
            )
        } else {
            None
        };
        let directory = match &request.profile {
            BrowserProfile::Persistent(path) => path.clone(),
            BrowserProfile::Ephemeral => temporary.as_ref().unwrap().path().to_path_buf(),
        };
        let input_enabled = request.user_input_enabled;
        let revision = Arc::new(nomifun_browser_platform::revision::BrowserRevision::default());
        let downloads = windows::user_downloads::DownloadHistory::new(revision.clone());
        Ok(Arc::new_cyclic(|weak| DesktopBrowserRuntime {
            weak: weak.clone(),
            popups: self.popups,
            input_locked: Arc::new(AtomicBool::new(!input_enabled)),
            app: self.app.clone(),
            request,
            directory,
            temporary: StdMutex::new(temporary),
            revision,
            downloads,
            creating: Mutex::new(()),
            site_data_view: Mutex::new(None),
            pending_work: Mutex::new(BTreeMap::new()),
            closing: CancellationToken::new(),
            state: Mutex::new(RuntimeState {
                tabs: BTreeMap::new(),
                active: None,
                bounds: None,
                visible: false,
                closed: false,
                input_enabled,
                presentation_requested: false,
                surface_epoch: 0,
                surface_cancel: CancellationToken::new(),
                uploads: Vec::new(),
                download_files: Vec::new(),
            }),
        }))
    }
}

struct NativeTab {
    close_gate: Mutex<()>,
    rendering_capture: Arc<AtomicBool>,
    view: tauri::Webview,
    metadata: Arc<StdMutex<BrowserTabSnapshot>>,
    automation: Arc<Mutex<super::automation::TabAutomation>>,
    operation: Arc<StdMutex<Option<CancellationToken>>>,
    popup_stop: CancellationToken,
    popup_work: Arc<Mutex<()>>,
}

struct RuntimeState {
    download_files: Vec<Arc<nomifun_browser_platform::downloads::PreparedBrowserDownload>>,
    uploads: Vec<Arc<nomifun_browser_platform::uploads::PreparedBrowserUpload>>,
    surface_cancel: CancellationToken,
    surface_epoch: u64,
    presentation_requested: bool,
    tabs: BTreeMap<String, Arc<NativeTab>>,
    active: Option<String>,
    bounds: Option<BrowserSurfaceBounds>,
    visible: bool,
    closed: bool,
    input_enabled: bool,
}

pub struct DesktopBrowserRuntime {
    weak: Weak<DesktopBrowserRuntime>,
    popups: bool,
    input_locked: Arc<AtomicBool>,
    app: tauri::AppHandle,
    request: CreateBrowserRuntime,
    directory: std::path::PathBuf,
    temporary: StdMutex<Option<tempfile::TempDir>>,
    revision: Arc<nomifun_browser_platform::revision::BrowserRevision>,
    downloads: Arc<windows::user_downloads::DownloadHistory>,
    creating: Mutex<()>,
    // Not a user tab. Retained on failed teardown, never silently abandoned.
    site_data_view: Mutex<Option<OwnedSiteDataView>>,
    pending_work: Mutex<BTreeMap<String, Arc<pending_work::PendingWork>>>,
    closing: CancellationToken,
    state: Mutex<RuntimeState>,
}

struct OwnedSiteDataView {
    label: String,
    view: Option<tauri::Webview>,
}

fn reconcile_chooser_after_load(view:&tauri::Webview,input_locked:Arc<AtomicBool>) {
    let view=view.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error)=windows::reconcile_file_chooser_policy(&view,input_locked).await {
            tracing::warn!(%error,"native file chooser policy restoration failed");
        }
    });
}

enum NativeOperation {
    Input(BrowserAction),
    Upload { element: BrowserElementRef, files: Arc<nomifun_browser_platform::uploads::PreparedBrowserUpload> },
    Download { element: BrowserElementRef, file: Arc<nomifun_browser_platform::downloads::PreparedBrowserDownload> },
}
impl NativeOperation {
    fn element(&self)->&BrowserElementRef {
        match self {Self::Input(action)=>action.element(),Self::Upload{element,..}|Self::Download{element,..}=>element}
    }
}

struct InputScope(Arc<StdMutex<Option<CancellationToken>>>);
impl Drop for InputScope {
    fn drop(&mut self) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
    }
}

fn valid_url(input: &str) -> Result<url::Url, WorkspaceError> {
    let url = url::Url::parse(input).map_err(|_| WorkspaceError::InvalidUrl)?;
    if input.len() > 8192
        || !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(WorkspaceError::InvalidUrl);
    }
    Ok(url)
}

fn native_error(error: impl std::fmt::Display) -> WorkspaceError {
    tracing::warn!(error = %error, "native browser host operation failed");
    WorkspaceError::NativeCommandFailed
}

/// Navigation cancellation stops the same native page and still awaits the
/// original callback. Never drop that callback or submit navigation after Stop.
async fn navigation_command(
    view: &tauri::Webview,
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
    let navigation = windows::protocol_call(view, method, params);
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
        let stop = windows::protocol_call(view, "Page.stopLoading", serde_json::json!({})).await;
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

impl DesktopBrowserRuntime {
    async fn create_site_data_view(&self) -> Result<tauri::Webview, WorkspaceError> {
        // Caller holds creating, and has already validated user/generation.
        if self.site_data_view.lock().await.is_some() { return Err(RunAdmissionError::WorkerFailed.into()); }
        let app = self.app.clone();
        if app.get_window("main").is_none() { return Err(WorkspaceError::NativeUnavailable); }
        let directory = self.directory.clone();
        let ephemeral = matches!(self.request.profile, BrowserProfile::Ephemeral);
        let label = format!("browser-site-data-{}",uuid::Uuid::now_v7());
        *self.site_data_view.lock().await = Some(OwnedSiteDataView { label:label.clone(), view:None });
        let created = tauri::async_runtime::spawn_blocking(move || {
            app.get_window("main").ok_or(WorkspaceError::NativeUnavailable)?.add_child(
                tauri::webview::WebviewBuilder::new(label,tauri::WebviewUrl::External("about:blank".parse().unwrap()))
                    .data_directory(directory).incognito(ephemeral).devtools(false).focused(false).disable_drag_drop_handler()
                    .on_navigation(|url|url.as_str()=="about:blank")
                    .on_new_window(|_,_|tauri::webview::NewWindowResponse::Deny),
                tauri::LogicalPosition::new(-32000.0,-32000.0),tauri::LogicalSize::new(1.0,1.0),
            ).map_err(native_error)
        }).await;
        let view = match created {
            Ok(Ok(view)) => view,
            // Keep the exact pending label on uncertain creation. Teardown may
            // recover its registered view, but absence is not a close proof.
            _ => return Err(RunAdmissionError::WorkerFailed.into()),
        };
        self.site_data_view.lock().await.as_mut().ok_or(WorkspaceError::WorkspaceClosed)?.view = Some(view.clone());
        let prepared = async {
            view.hide().map_err(native_error)?;
            windows::set_native_user_input_enabled(&view,false).await.map_err(native_error)
        }.await;
        if let Err(error) = prepared { self.close_site_data_view().await?; return Err(error); }
        Ok(view)
    }
    async fn close_site_data_view(&self) -> Result<(), WorkspaceError> {
        let mut slot = self.site_data_view.lock().await;
        if let Some(owned) = slot.as_mut() {
            if owned.view.is_none() { owned.view = self.app.get_webview(&owned.label); }
            let view=owned.view.as_ref().ok_or(WorkspaceError::Admission(RunAdmissionError::WorkerFailed))?;
            windows::site_data::settle(view).await?;
            windows::close_native_view(view).await.map_err(|_|WorkspaceError::Admission(RunAdmissionError::WorkerFailed))?;
        }
        *slot = None;
        Ok(())
    }
    fn install_popup<'a>(
        &'a self,
        tab: &'a Arc<NativeTab>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), WorkspaceError>> + Send + 'a>>
    {
        Box::pin(async move {
            if !self.popups {
                return Ok(());
            }
            if tab.popup_stop.is_cancelled() {
                return Err(WorkspaceError::WorkspaceClosed);
            }
            let metadata = tab.metadata.clone();
            let operation = tab.operation.clone();
            let locked = self.input_locked.clone();
            let closed = tab.popup_stop.clone();
            let admission = Arc::new(move || {
                if closed.is_cancelled() {
                    return None;
                }
                let operation = operation
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone();
                if locked.load(Ordering::Acquire) && operation.is_none() {
                    return None;
                }
                let opener = metadata
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .target
                        .clone();
                Some(windows::popup::PopupAdmission {
                    download: windows::user_downloads::capture_agent(&opener),
                    opener,
                    cancel: operation.unwrap_or_default(),
                    closed: closed.clone(),
                })
            });
            let mut subscription =
                windows::popup::PopupSubscription::listen_guarded(&tab.view, admission)
                    .await
                    .map_err(native_error)?;
            let weak = self.weak.clone();
            let stop = tab.popup_stop.clone();
            let work = tab.popup_work.clone();
            let page_closed = subscription.page_closed();
            let tab_id = tab.view.label().to_owned();
            tauri::async_runtime::spawn(async move {
                loop {
                    let request = tokio::select! {
                        biased;
                        _=stop.cancelled()=>break,
                        _=page_closed.cancelled()=>{
                            if let Some(runtime)=weak.upgrade() {
                                if let Err(error)=runtime.close_from_page(&tab_id).await {
                                    tracing::warn!(code=error.code(),"native page close did not settle");
                                }
                            }
                            break;
                        },
                        request=subscription.next()=>request,
                    };
                    let Some(request) = request else {
                        // Wry can release the popup event sender while destroying
                        // its container, just before the close notification runs.
                        tokio::select! {
                            biased;
                            _=stop.cancelled()=>{},
                            _=page_closed.cancelled()=>{
                                if let Some(runtime)=weak.upgrade() {
                                    if let Err(error)=runtime.close_from_page(&tab_id).await {
                                        tracing::warn!(code=error.code(),"native page close did not settle");
                                    }
                                }
                            },
                            _=tokio::time::sleep(std::time::Duration::from_secs(1))=>{
                                if let Some(runtime)=weak.upgrade() {
                                    let state=runtime.state.lock().await;
                                    if let Some(tab)=state.tabs.get(&tab_id) {
                                        tab.metadata.lock().unwrap_or_else(|error|error.into_inner()).lifecycle=BrowserTabLifecycle::Failed;
                                        runtime.revision.bump();
                                    }
                                }
                            },
                        }
                        break;
                    };
                    let _working = work.lock().await;
                    let Some(runtime) = weak.upgrade() else { break };
                    if let Err(error) = runtime.consume_popup(request).await {
                        tracing::debug!(code = error.code(), "native popup was not admitted");
                    }
                }
            });
            Ok(())
        })
    }

    async fn close_from_page(&self, id: &str) -> Result<(), WorkspaceError> {
        let tab=self.state.lock().await.tabs.get(id).cloned();
        match tab {Some(tab)=>{
            windows::user_downloads::wait_page_downloads(&tab.view).await?;
            self.retire_native_tab(&tab).await
        },None=>Ok(())}
    }

    /// A single native teardown path for toolbar close, page close, failed
    /// initialization and workspace shutdown. No driver lock before destruction.
    async fn retire_native_tab(&self, tab: &Arc<NativeTab>) -> Result<(), WorkspaceError> {
        let _closing=tab.close_gate.lock().await;
        let id=tab.view.label();
        if !self.state.lock().await.tabs.get(id).is_some_and(|current|Arc::ptr_eq(current,tab)) {return Ok(());}
        tab.popup_stop.cancel();
        let _=windows::script_dialogs::drain(&tab.view).await;
        if let Err(error)=windows::close_native_view(&tab.view).await {
            tab.metadata.lock().unwrap_or_else(|e|e.into_inner()).lifecycle=BrowserTabLifecycle::Failed;
            self.revision.bump();
            return Err(native_error(error));
        }
        let mut state=self.state.lock().await;
        state.tabs.remove(id);
        if state.active.as_deref()==Some(id) {state.active=state.tabs.keys().next().cloned();}
        if !state.closed {self.apply_surface(&state).await?;}
        self.revision.bump();
        Ok(())
    }

    async fn consume_popup(
        &self,
        request: windows::popup::PopupRequest,
    ) -> Result<(), WorkspaceError> {
        let context = request
            .admission
            .clone()
            .ok_or(WorkspaceError::StaleTarget)?;
        if context.opener.tab_id != request.opener_label {
            return Err(WorkspaceError::StaleTarget);
        }
        if request.url != "about:blank" {
            valid_url(&request.url)?;
        }
        let _creation = tokio::select! {
            biased;
            _=context.cancel.cancelled()=>return Err(RunAdmissionError::Cancelled.into()),
            _=context.closed.cancelled()=>return Err(WorkspaceError::WorkspaceClosed),
            guard=self.creating.lock()=>guard,
        };
        {
            let state = self.state.lock().await;
            if state.closed {
                return Err(WorkspaceError::WorkspaceClosed);
            }
            self.target(&state, &context.opener)?;
            if state.tabs.len() >= 8 {
                return Err(WorkspaceError::TabLimit);
            }
        }
        let metadata = Arc::new(StdMutex::new(BrowserTabSnapshot {
            target: BrowserTabTarget {
                tab_id: String::new(),
                runtime_generation: self.request.runtime_generation,
                document_generation: 0,
            },
            title: String::new(),
            url: request.url.clone(),
            lifecycle: BrowserTabLifecycle::Loading,
            can_go_back: false,
            can_go_forward: false,
            zoom_percent: 100,
            blocked_permissions: vec![],
            permission_requests: vec![],
            script_dialog: None,
            diagnostics: Default::default(),
        }));
        let title_metadata = metadata.clone();
        let load_metadata = metadata.clone();
        let title_revision = self.revision.clone();
        let load_revision = self.revision.clone();
        let load_input_locked=self.input_locked.clone();
        let view = request
            // Navigation/recovery must restore policy from live run ownership,
            // not from the state captured when this view was first created.
            .create_child(move |builder| {
                builder
                    // No user-visible DevTools product in v2. Keep F12/Inspect
                    // disabled; host-only protocol calls remain internal.
                    .devtools(false)
                    .on_document_title_changed(move |_, title| {
                        title_metadata
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .title = title.chars().take(512).collect();
                        title_revision.bump();
                    })
                    .on_page_load(move |view, payload| {
                        if matches!(payload.event(),tauri::webview::PageLoadEvent::Started) {
                            let _=windows::permissions::navigation_started(view.label());
                            windows::user_file_chooser::cancel(view.label());
                            windows::user_downloads::cancel_pending(view.label());
                        }
                        let mut data = load_metadata
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        data.url = payload.url().to_string();
                        match payload.event() {
                            tauri::webview::PageLoadEvent::Started => {
                                data.target.document_generation += 1;
                                data.blocked_permissions.clear();
                                data.diagnostics.clear_page();
                                data.lifecycle = BrowserTabLifecycle::Loading;
                            }
                            tauri::webview::PageLoadEvent::Finished => {
                                data.lifecycle = BrowserTabLifecycle::Ready
                            }
                        }
                        drop(data);
                        if matches!(payload.event(),tauri::webview::PageLoadEvent::Finished) {
                            windows::diagnostics::document_loaded(&view);
                            reconcile_chooser_after_load(&view,load_input_locked.clone());
                        }
                        load_revision.bump();
                    })
            })
            .await
            .map_err(native_error)?;
        let id = view.label().to_owned();
        metadata
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .target
            .tab_id = id.clone();
        windows::set_user_input_enabled(&view, false)
            .await
            .map_err(native_error)?;
        {
            let state = self.state.lock().await;
            if state.closed || context.closed.is_cancelled() {
                return Err(WorkspaceError::WorkspaceClosed);
            }
            if context.cancel.is_cancelled() {
                return Err(RunAdmissionError::Cancelled.into());
            }
            self.target(&state, &context.opener)?;
        }
        // Before binding, the native request owns its unclaimed child. Binding
        // transfers that exact view, never a URL clone, to the Runtime registry.
        windows::permissions::install(&view, metadata.clone(), self.revision.clone(), self.input_locked.clone())
            .await
            .map_err(native_error)?;
        windows::process_failure::install(&view, metadata.clone(), self.revision.clone()).await.map_err(native_error)?;
        windows::diagnostics::install(&view, metadata.clone()).await;
        windows::script_dialogs::install(&view, metadata.clone(), self.revision.clone()).await?;
        let tab = Arc::new(NativeTab {
            close_gate: Mutex::new(()),
            rendering_capture: Default::default(),
            view: view.clone(),
            metadata,
            automation: Default::default(),
            operation: Default::default(),
            popup_stop: self.closing.child_token(),
            popup_work: Default::default(),
        });
        let mut automation = tab.automation.lock().await;
        // From here the retained candidate (and then state.tabs) is the sole
        // close owner. Expiry/Stop may deny binding but cannot erase its view.
        // consume_popup is owned work and close waits on the creation guard.
        request.claim_child(&view).await.map_err(native_error)?;
        let previous = {
            let mut state = self.state.lock().await;
            state.tabs.insert(id.clone(), tab.clone());
            let previous = state.active.replace(id.clone());
            self.revision.bump();
            previous
        };
        let initialized = async {
            if context.cancel.is_cancelled() {
                return Err(RunAdmissionError::Cancelled.into());
            }
            if context.closed.is_cancelled() {
                return Err(WorkspaceError::WorkspaceClosed);
            }
            windows::user_file_chooser::install(&view,tab.automation.clone(),self.input_locked.clone(),self.app.path().home_dir().map_err(native_error)?)
                .await.map_err(native_error)?;
            windows::shortcuts::install(&view, self.request.key.agent_session_id.clone(), tab.metadata.clone(), self.input_locked.clone()).await.map_err(native_error)?;
            windows::user_downloads::install(&view, tab.metadata.clone(), self.input_locked.clone(), self.app.path().download_dir().map_err(native_error)?, self.downloads.clone())
                .await.map_err(native_error)?;
            automation.configure_file_choosers(&view, self.input_locked.load(Ordering::Acquire)).await?;
            self.install_popup(&tab).await?;
            windows::navigation_metadata::install(
                &view,
                tab.metadata.clone(),
                self.revision.clone(),
            )
            .await
            .map_err(native_error)?;
            let state = self.state.lock().await;
            if state.closed || context.closed.is_cancelled() {
                return Err(WorkspaceError::WorkspaceClosed);
            }
            if context.cancel.is_cancelled() {
                return Err(RunAdmissionError::Cancelled.into());
            }
            self.target(&state, &context.opener)?;
            windows::set_user_input_enabled(&view, false)
                .await
                .map_err(native_error)?;
            drop(state);
            windows::script_dialogs::resume(&view).await?;
            if let Some(download) = context.download.clone() {
                windows::user_downloads::inherit_agent(&view, context.opener.clone(), download).await?;
            }
            // All handlers and the candidate are owned before the popup's first
            // document executes. A dialog can now yield through the registry.
            request.complete(&view).await.map_err(native_error)?;
            let mut state = self.state.lock().await;
            if !state.tabs.contains_key(&id) { return Ok(()); }
            if state.closed || context.closed.is_cancelled() { return Err(WorkspaceError::WorkspaceClosed); }
            if context.cancel.is_cancelled() { return Err(RunAdmissionError::Cancelled.into()); }
            windows::set_native_user_input_enabled(&view,state.input_enabled).await.map_err(native_error)?;
            state.active = Some(id.clone());
            self.apply_surface(&state).await?;
            Ok(())
        }
        .await;
        if let Err(error) = initialized {
            {
                let mut state=self.state.lock().await;
                if state.active.as_ref()==Some(&id) {state.active=previous.filter(|id|state.tabs.contains_key(id));}
            }
            self.retire_native_tab(&tab).await?;
            return Err(error);
        }
        self.revision.bump();
        Ok(())
    }

    fn request_presentation(&self, state: &mut RuntimeState) {
        if !state.input_enabled && !state.presentation_requested && state.active.is_some() {
            if state.visible {
                state.presentation_requested = true;
                return;
            }
            match self.app.emit_to(
                "main",
                "browser-workspace-open",
                &self.request.key.agent_session_id,
            ) {
                Ok(()) => state.presentation_requested = true,
                Err(error) => tracing::warn!(%error,"could not announce native browser workspace"),
            }
        }
    }

    fn snapshot_locked(&self, state: &RuntimeState) -> BrowserRuntimeSnapshot {
        BrowserRuntimeSnapshot {
            runtime_generation: self.request.runtime_generation,
            revision: self.revision.current(),
            active_tab_id: state.active.clone(),
            downloads: self.downloads.snapshot(),
            tabs: state
                .tabs
                .values()
                .map(|tab| {
                    tab.metadata
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone()
                })
                .collect(),
        }
    }

    async fn create_tab(
        &self,
        url: &str,
        cancel: &CancellationToken,
        scope: &Arc<StdMutex<Option<String>>>,
    ) -> Result<(), WorkspaceError> {
        if cancel.is_cancelled() {
            return Err(RunAdmissionError::Cancelled.into());
        }
        let _creation = tokio::select! {
            biased;
            _=self.closing.cancelled()=>return Err(WorkspaceError::WorkspaceClosed),
            _=cancel.cancelled()=>return Err(RunAdmissionError::Cancelled.into()),
            guard=self.creating.lock()=>guard,
        };
        let url = valid_url(url)?;
        let state = self.state.lock().await;
        if state.closed {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        if state.tabs.len() >= 8 {
            return Err(WorkspaceError::TabLimit);
        }
        drop(state);
        let id = format!("browser-{}", uuid::Uuid::now_v7());
        *scope.lock().unwrap_or_else(|e|e.into_inner())=Some(id.clone());
        let metadata = Arc::new(StdMutex::new(BrowserTabSnapshot {
            target: BrowserTabTarget {
                tab_id: id.clone(),
                runtime_generation: self.request.runtime_generation,
                document_generation: 0,
            },
            title: String::new(),
            url: "about:blank".into(),
            lifecycle: BrowserTabLifecycle::Loading,
            can_go_back: false,
            can_go_forward: false,
            zoom_percent: 100,
            blocked_permissions: vec![],
            permission_requests: vec![],
            script_dialog: None,
            diagnostics: Default::default(),
        }));
        let title_metadata = metadata.clone();
        let load_metadata = metadata.clone();
        let title_revision = self.revision.clone();
        let load_revision = self.revision.clone();
        let load_input_locked=self.input_locked.clone();
        let directory = self.directory.clone();
        let ephemeral = matches!(self.request.profile, BrowserProfile::Ephemeral);
        let app = self.app.clone();
        let label = id.clone();
        let view = tauri::async_runtime::spawn_blocking(move || {
            let window = app
                .get_window("main")
                .ok_or(WorkspaceError::NativeUnavailable)?;
            window
                .add_child(
                    tauri::webview::WebviewBuilder::new(
                        label,
                        tauri::WebviewUrl::External("about:blank".parse().unwrap()),
                    )
                    .data_directory(directory)
                    .devtools(false)
                    .incognito(ephemeral)
                    .focused(false)
                    .disable_drag_drop_handler()
                    .on_navigation(|url| {
                        url.as_str() == "about:blank" || valid_url(url.as_str()).is_ok()
                    })
                    .on_document_title_changed(move |_, title| {
                        title_metadata
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .title = title.chars().take(512).collect();
                        title_revision.bump();
                    })
                    .on_page_load(move |view, payload| {
                        if matches!(payload.event(),tauri::webview::PageLoadEvent::Started) {
                            let _=windows::permissions::navigation_started(view.label());
                            windows::user_file_chooser::cancel(view.label());
                            windows::user_downloads::cancel_pending(view.label());
                        }
                        let mut data = load_metadata.lock().unwrap_or_else(|e| e.into_inner());
                        data.url = payload.url().to_string();
                        match payload.event() {
                            tauri::webview::PageLoadEvent::Started => {
                                data.target.document_generation += 1;
                                data.blocked_permissions.clear();
                                data.diagnostics.clear_page();
                                data.lifecycle = BrowserTabLifecycle::Loading;
                            }
                            tauri::webview::PageLoadEvent::Finished => {
                                data.lifecycle = BrowserTabLifecycle::Ready
                            }
                        }
                        drop(data);
                        if matches!(payload.event(),tauri::webview::PageLoadEvent::Finished) {
                            windows::diagnostics::document_loaded(&view);
                            reconcile_chooser_after_load(&view,load_input_locked.clone());
                        }
                        load_revision.bump();
                    }),
                    // Native view is offscreen until the input policy is applied and
                    // the authenticated renderer has supplied a real display slot.
                    tauri::LogicalPosition::new(-32_000.0, -32_000.0),
                    tauri::LogicalSize::new(1024.0, 768.0),
                )
                .map_err(native_error)
        })
        .await
        .map_err(native_error)??;
        let tab = Arc::new(NativeTab {
            close_gate: Mutex::new(()),
            rendering_capture: Default::default(),
            automation: Default::default(),
            view: view.clone(),
            metadata,
            operation: Default::default(),
            popup_stop: self.closing.child_token(),
            popup_work: Default::default(),
        });
        let mut automation = tab.automation.lock().await;
        // Publishing the candidate retains cleanup authority and quota while
        // its page initializes. Other pages and surface hides remain usable.
        let previous = {
            let mut state = self.state.lock().await;
            state.tabs.insert(id.clone(), tab.clone());
            let previous = state.active.replace(id.clone());
            self.revision.bump();
            previous
        };
        let initialized = async {
            if self.closing.is_cancelled() {
                return Err(WorkspaceError::WorkspaceClosed);
            }
            if cancel.is_cancelled() {
                return Err(RunAdmissionError::Cancelled.into());
            }
            view.hide().map_err(native_error)?;
            {
                let state = self.state.lock().await;
                windows::set_user_input_enabled(&view, state.input_enabled)
                    .await
                    .map_err(native_error)?;
            }
            windows::permissions::install(&view, tab.metadata.clone(), self.revision.clone(), self.input_locked.clone())
                .await
                .map_err(native_error)?;
            windows::process_failure::install(&view, tab.metadata.clone(), self.revision.clone()).await.map_err(native_error)?;
            windows::diagnostics::install(&view, tab.metadata.clone()).await;
            windows::script_dialogs::install(&view, tab.metadata.clone(), self.revision.clone()).await?;
            windows::user_file_chooser::install(&view,tab.automation.clone(),self.input_locked.clone(),self.app.path().home_dir().map_err(native_error)?)
                .await.map_err(native_error)?;
            windows::shortcuts::install(&view, self.request.key.agent_session_id.clone(), tab.metadata.clone(), self.input_locked.clone()).await.map_err(native_error)?;
            windows::user_downloads::install(&view, tab.metadata.clone(), self.input_locked.clone(), self.app.path().download_dir().map_err(native_error)?, self.downloads.clone())
                .await.map_err(native_error)?;
            windows::protocol_call(&view, "Page.enable", serde_json::json!({}))
                .await
                .map_err(native_error)?;
            automation.configure_file_choosers(&view, self.input_locked.load(Ordering::Acquire)).await?;
            self.install_popup(&tab).await?;
            windows::navigation_metadata::install(
                &view,
                tab.metadata.clone(),
                self.revision.clone(),
            )
            .await
            .map_err(native_error)?;
            navigation_command(
                &view,
                "Page.navigate",
                serde_json::json!({"url":url.as_str()}),
                cancel,
                &self.closing,
            )
            .await?;
            let mut state = self.state.lock().await;
            if state.closed || self.closing.is_cancelled() {
                return Err(WorkspaceError::WorkspaceClosed);
            }
            if cancel.is_cancelled() {
                return Err(RunAdmissionError::Cancelled.into());
            }
            state.active = Some(id.clone());
            self.apply_surface(&state).await?;
            if cancel.is_cancelled() {
                return Err(RunAdmissionError::Cancelled.into());
            }
            Ok(())
        }
        .await;
        if let Err(error) = initialized {
            {
                let mut state=self.state.lock().await;
                if state.active.as_ref()==Some(&id) {state.active=previous.filter(|id|state.tabs.contains_key(id));}
            }
            self.retire_native_tab(&tab).await?;
            return Err(error);
        }
        self.revision.bump();
        Ok(())
    }

    async fn apply_surface(&self, state: &RuntimeState) -> Result<(), WorkspaceError> {
        let views: Vec<_> = state
            .tabs
            .iter()
            .map(|(id, tab)| (id.clone(), tab.view.clone(), tab.rendering_capture.clone(), tab.popup_stop.clone()))
            .collect();
        let active = state.active.clone();
        let visible = state.visible;
        let bounds = state.bounds;
        let cancel = state.surface_cancel.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.app
            .run_on_main_thread(move || {
                let result = (|| {
                    // The owner may be superseded after dispatch but before the UI
                    // runs. Do not let an old request show OR hide a newer surface.
                    if cancel.is_cancelled() {
                        return Ok(());
                    }
                    for (id, view, rendering_capture, closing) in views {
                        // Native destruction retains its registry entry until
                        // acknowledged; queued layout must not touch that view.
                        if closing.is_cancelled() { continue; }
                        if visible && active.as_ref() == Some(&id) && bounds.is_some() {
                            let bounds = bounds.unwrap();
                            view.set_position(tauri::LogicalPosition::new(bounds.x, bounds.y))
                                .map_err(native_error)?;
                            view.set_size(tauri::LogicalSize::new(bounds.width, bounds.height))
                                .map_err(native_error)?;
                            view.show().map_err(native_error)?;
                            windows::permissions::set_visible(&id, true).map_err(native_error)?;
                            windows::user_file_chooser::set_visible(&id, true);
                            windows::user_downloads::set_visible(&id, true);
                        } else {
                            windows::user_file_chooser::set_visible(&id, false);
                            windows::user_downloads::set_visible(&id, false);
                            let permission_cleanup=windows::permissions::set_visible(&id, false);
                            if rendering_capture.load(Ordering::Acquire) {
                                windows::hide_native_window(&view).map_err(native_error)?;
                            } else {
                                view.hide().map_err(native_error)?;
                            }
                            // A failed permission callback must not leave the
                            // native page covering a modal or a different pane.
                            permission_cleanup.map_err(native_error)?;
                        }
                    }
                    Ok(())
                })();
                let _ = tx.send(result);
            })
            .map_err(native_error)?;
        rx.await.map_err(native_error)?
    }

    fn target<'a>(
        &self,
        state: &'a RuntimeState,
        target: &BrowserTabTarget,
    ) -> Result<&'a Arc<NativeTab>, WorkspaceError> {
        let tab = state
            .tabs
            .get(&target.tab_id)
            .ok_or(WorkspaceError::TabNotFound)?;
        if tab
            .metadata
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .target
            != *target
        {
            return Err(WorkspaceError::StaleTarget);
        }
        Ok(tab)
    }
}

#[cfg(test)]
mod url_tests {
    use super::*;

    #[test]
    fn embedded_navigation_accepts_native_web_urls_without_restricting_local_networks() {
        for url in [
            "https://example.com/path?q=1#section",
            "http://localhost:5173/",
            "http://127.0.0.1:3000/",
            "http://192.168.1.10:8080/",
            "https://[fd00::1]/",
        ] {
            assert!(valid_url(url).is_ok(), "{url}");
        }
    }

    #[test]
    fn embedded_navigation_rejects_credentials_and_privileged_schemes() {
        for url in [
            "https://name:secret@example.com/",
            "https://name@example.com/",
            "file:///private/file",
            "javascript:alert(1)",
            "data:text/html,hello",
            "about:blank",
            "ftp://example.com/file",
            "ws://localhost:5173/socket",
            "http://",
        ] {
            assert_eq!(valid_url(url), Err(WorkspaceError::InvalidUrl), "{url}");
        }
        assert_eq!(
            valid_url(&format!("https://example.com/{}", "a".repeat(8192))),
            Err(WorkspaceError::InvalidUrl)
        );
    }
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
impl NativeInputGate for DesktopBrowserRuntime {
    async fn lock_user_input(&self) -> Result<(), RunAdmissionError> {
        let mut state = self.state.lock().await;
        if state.input_enabled {
            state.presentation_requested = false;
        }
        state.input_enabled = false;
        self.input_locked.store(true, Ordering::Release);
        let tabs: Vec<_> = state.tabs.values().cloned().collect();
        drop(state);
        let mut error = None;
        for tab in &tabs {
            if windows::set_user_input_enabled(&tab.view, false)
                .await
                .is_err()
            {
                error = Some(RunAdmissionError::InputGateFailed);
            }
        }
        // Seal existing OOPIFs and the policy used by newly attached ones at
        // run admission, even if this turn never calls a Browser Tool.
        for tab in tabs {
            if tab.metadata.lock().unwrap_or_else(|error|error.into_inner()).lifecycle == BrowserTabLifecycle::Crashed { continue; }
            if tab.automation.lock().await.configure_file_choosers(&tab.view, true).await.is_err() {
                error = Some(RunAdmissionError::InputGateFailed);
            }
        }
        error.map_or(Ok(()), Err)
    }
    async fn release_pressed_input(&self) -> Result<(), RunAdmissionError> {
        self.settle_pending_work().await?;
        let sources: Vec<_> = self.state.lock().await.tabs.values().cloned().collect();
        for tab in sources {
            let _settled = tab.popup_work.lock().await;
        }
        // Do not hold the tab registry while waiting for an input callback.
        // Native popup handling needs that registry to settle the callback.
        let tabs: Vec<_> = self.state.lock().await.tabs.values().cloned().collect();
        let mut failure = None;
        for tab in tabs {
            if windows::popup::settle_view(&tab.view).await.is_err() {
                failure = Some(RunAdmissionError::InputGateFailed);
            }
            if windows::user_downloads::settle_agent(&tab.view).await.is_err() {
                failure = Some(RunAdmissionError::InputGateFailed);
            }
            let mut automation=tab.automation.lock().await;
            if tab.metadata.lock().unwrap_or_else(|error|error.into_inner()).lifecycle==BrowserTabLifecycle::Crashed {
                // ProcessFailed proved the old document and its pressed/focus
                // state are gone. New protocol cleanup cannot reach that process.
                *automation=Default::default();
                continue;
            }
            if automation.settle_agent(&tab.view).await.is_err() {
                failure = Some(RunAdmissionError::InputGateFailed);
            }
        }
        if failure.is_none() {
            let mut state = self.state.lock().await;
            for file in &state.download_files {
                if file.close().is_err() { failure = Some(RunAdmissionError::InputGateFailed); }
            }
            if failure.is_none() { state.download_files.clear(); }
        }
        failure.map_or(Ok(()), Err)
    }
    async fn unlock_user_input(&self) -> Result<(), RunAdmissionError> {
        let mut state = self.state.lock().await;
        if state.closed {
            return Ok(());
        }
        // Re-enable ordinary website dialogs before any native user input is
        // unlocked. A previous run's drain policy must not silently cancel them.
        for tab in state.tabs.values() {
            windows::script_dialogs::resume(&tab.view).await.map_err(|_|RunAdmissionError::InputGateFailed)?;
        }
        self.input_locked.store(false,Ordering::Release);
        for tab in state.tabs.values() {
            if windows::set_user_input_enabled(&tab.view, true)
                .await
                .is_err()
            {
                // Roll back already-unlocked tabs on a partial platform failure.
                self.input_locked.store(true,Ordering::Release);
                for tab in state.tabs.values() {
                    let _ = windows::set_user_input_enabled(&tab.view, false).await;
                }
                return Err(RunAdmissionError::InputGateFailed);
            }
        }
        state.input_enabled = true;
        self.input_locked.store(false, Ordering::Release);
        Ok(())
    }
}

#[async_trait]
impl BrowserRuntime for DesktopBrowserRuntime {
    fn changes(&self) -> Option<tokio::sync::watch::Receiver<u64>> {
        Some(self.revision.subscribe())
    }
    fn automation(&self) -> Option<&dyn BrowserAutomationPort> {
        Some(self)
    }
    fn surface(&self) -> Option<&dyn BrowserNativeSurfacePort> {
        Some(self)
    }
    async fn snapshot(&self) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        let state = self.state.lock().await;
        if state.closed {
            return Err(WorkspaceError::WorkspaceClosed);
        }
        Ok(self.snapshot_locked(&state))
    }
    async fn execute(&self, command: BrowserTabCommand, cancel: CancellationToken) -> Result<BrowserRuntimeSnapshot, WorkspaceError> {
        self.execute_owned(command, cancel).await
    }
    async fn close(&self) -> Result<(), WorkspaceError> {
        self.closing.cancel();
        self.cancel_pending_for_close().await;
        {
            let mut state = self.state.lock().await;
            state.closed = true;
            state.surface_cancel.cancel();
        }
        // Do not delete a profile while a native controller is still being
        // created, even if it has not reached the tab registry yet.
        let _creation = self.creating.lock().await;
        self.close_site_data_view().await?;
        let tabs: Vec<_> = {
            let mut state = self.state.lock().await;
            state.closed = true;
            state.surface_cancel.cancel();
            state
                .tabs
                .iter()
                .map(|(id, tab)| (id.clone(), tab.clone()))
                .collect()
        };
        let mut failure = None;
        for (_, tab) in tabs {
            match self.retire_native_tab(&tab).await {
                Ok(())=>{let _page=tab.automation.lock().await;}
                Err(error)=>failure=Some(error),
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        self.retire_pending_after_close().await;
        {
            let mut state=self.state.lock().await;
            state.active=None;
            for files in &state.uploads {files.close()?;}
            state.uploads.clear();
            for file in &state.download_files { file.close()?; }
            state.download_files.clear();
        }
        // Keep the temp directory until all COM controllers have closed.
        let directory=self.temporary.lock().unwrap_or_else(|e|e.into_inner()).as_ref().map(|directory|directory.path().to_path_buf());
        if let Some(directory)=directory {
            // Controller Close precedes final profile-handle release and the
            // last shutdown writes. Await transient sharing/nonempty-directory
            // races only for this owned temp profile after native destruction,
            // retaining its owner; permission failures are not retried.
            let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(2);
            loop {
                match std::fs::remove_dir_all(&directory) {
                    Ok(())=>break,
                    Err(error) if error.kind()==std::io::ErrorKind::NotFound=>break,
                    Err(error) if matches!(error.raw_os_error(),Some(32|33|145)) && tokio::time::Instant::now()<deadline=>{
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                    },
                    Err(error)=>return Err(native_error(error)),
                }
            }
        }
        self.temporary.lock().unwrap_or_else(|e|e.into_inner()).take();
        Ok(())
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
        let mut download = None;
        let fidelity=match operation {
            NativeOperation::Download { element, file } => {
                // Retain only failures until explicit native/runtime cleanup.
                self.state.lock().await.download_files.push(file.clone());
                let result = async {
                    let request = windows::user_downloads::arm_agent(&tab.view, expected.clone(), file.clone(), cancel.clone()).await?;
                    let action = automation.act(&tab.view, BrowserAction::Click { element, button: BrowserMouseButton::Left, click_count: 1 }, &cancel).await;
                    windows::user_downloads::finish_agent(&tab.view, request, action).await
                }.await;
                if matches!(result, Err(WorkspaceError::NativeCommandFailed)) { return Err(WorkspaceError::NativeCommandFailed); }
                let cleanup = file.close();
                if cleanup.is_ok() { self.state.lock().await.download_files.retain(|retained| !Arc::ptr_eq(retained, &file)); }
                cleanup?;
                download = Some(result?);
                InteractionFidelity::BrowserInput
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
            BrowserTabCommand::OpenDownloads { runtime_generation } => {
                if runtime_generation != self.request.runtime_generation { return Err(WorkspaceError::StaleTarget); }
                if !state.input_enabled { return Err(WorkspaceError::NotActionable); }
                windows::external_browser::open_downloads(&self.app, self.input_locked.clone(), cancel.clone(), self.closing.clone()).await?;
            }
            BrowserTabCommand::OpenExternal { target } => {
                if !state.input_enabled || state.active.as_ref()!=Some(&target.tab_id) { return Err(WorkspaceError::NotActionable); }
                let tab = self.target(&state, &target)?.clone();
                // The menu occludes the native surface, so visibility is not an
                // authority check. Keep the active-tab registry stable through dispatch.
                windows::external_browser::open(&tab.view, tab.metadata.clone(), target, self.input_locked.clone(), cancel.clone(), self.closing.clone()).await?;
            }
            BrowserTabCommand::CancelDownload { target, download_id } => {
                if !state.input_enabled { return Err(WorkspaceError::NotActionable); }
                let tab = self.target(&state, &target)?.clone();
                drop(state);
                windows::user_downloads::cancel_one(&tab.view, target, download_id).await.map_err(native_error)?;
                state = self.state.lock().await;
            }
            BrowserTabCommand::Permission { target, request_id, allow } => {
                if !state.input_enabled || !state.visible || state.active.as_ref()!=Some(&target.tab_id) {
                    return Err(WorkspaceError::NotActionable);
                }
                let tab=self.target(&state,&target)?.clone();
                drop(state);
                windows::permissions::respond(&tab.view,target,request_id,allow).await?;
                state=self.state.lock().await;
            }
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
                tab.view.set_zoom(f64::from(percent) / 100.0).map_err(native_error)?;
                tab.metadata.lock().unwrap_or_else(|error| error.into_inner()).zoom_percent = percent;
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
                windows::protocol_call(&tab.view, "Page.stopLoading", serde_json::json!({}))
                    .await
                    .map_err(native_error)?;
                state = self.state.lock().await;
            }
            BrowserTabCommand::Back { target } | BrowserTabCommand::Forward { target } => {
                let tab = self.target(&state, &target)?.clone();
                drop(state);
                let history = windows::protocol_call(
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
