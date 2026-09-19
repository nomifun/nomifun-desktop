//! Native lifecycle and deferred dialog ownership. No Cocoa modal UI is allowed
//! to bypass the conversation's existing dialog/Stop controls.
use super::*;
use nomifun_browser_platform::runtime::{
    BrowserDialogKind, BrowserPermissionRequest, BrowserTabLifecycle,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct NativeDialog {
    pub request_id: String,
    pub document_generation: u64,
    pub kind: BrowserDialogKind,
    pub message: String,
    pub default_text: String,
    pub origin: String,
    pub text_truncated: bool,
}

#[derive(Clone, Debug)]
pub struct PageSnapshot {
    pub document_generation: u64,
    pub url: String,
    pub title: String,
    pub lifecycle: BrowserTabLifecycle,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub dialog: Option<NativeDialog>,
    pub blocked_permissions: Vec<String>,
    pub permission_requests: Vec<BrowserPermissionRequest>,
}
impl Default for PageSnapshot {
    fn default() -> Self {
        Self { document_generation: 0, url: String::new(), title: String::new(), lifecycle: BrowserTabLifecycle::Loading,
            can_go_back: false, can_go_forward: false, dialog: None, blocked_permissions: Vec::new(), permission_requests: Vec::new() }
    }
}

pub(super) struct DeferredDialog {
    pub id: String,
    callback: JsdialogCallback,
}

pub(super) enum DeferredPermissionCallback {
    Media {
        callback: MediaAccessCallback,
        allowed: u32,
    },
    Prompt(PermissionPromptCallback),
}

pub(super) struct DeferredPermission {
    pub id: String,
    pub document_generation: u64,
    pub kind: String,
    callback: DeferredPermissionCallback,
}

impl DeferredPermission {
    fn finish(self, allow: bool) {
        match self.callback {
            DeferredPermissionCallback::Media { callback, allowed } => {
                if allow {
                    callback.cont(allowed);
                } else {
                    callback.cancel();
                }
            }
            DeferredPermissionCallback::Prompt(callback) => callback.cont(if allow {
                PermissionRequestResult::ACCEPT
            } else {
                PermissionRequestResult::DENY
            }),
        }
    }
}

fn bounded(value: Option<&CefString>, limit: usize) -> (String, bool) {
    let value = value.map(ToString::to_string).unwrap_or_default();
    let bounded: String = value.chars().take(limit).collect();
    let truncated = bounded.len() != value.len();
    (bounded, truncated)
}
fn origin(value: Option<&CefString>) -> String {
    value.and_then(|value| url::Url::parse(&value.to_string()).ok())
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .map(|url| url.origin().ascii_serialization()).unwrap_or_default()
}

impl Page {
    pub fn snapshot(&self) -> PageSnapshot { self.metadata.borrow().clone() }
    pub fn subscribe(&self) -> watch::Receiver<PageSnapshot> { self.metadata.subscribe() }

    /// A native observer must be nonblocking, and may only signal its owning
    /// runtime. Snapshots are updated before the signal or any dialog callback.
    pub fn set_change_listener(&self, listener: Arc<dyn Fn() + Send + Sync>) {
        *self.change_listener.lock().unwrap() = Some(listener);
    }
    pub(crate) fn changed(&self, update: impl FnOnce(&mut PageSnapshot)) {
        self.metadata.send_modify(update);
        let listener = self.change_listener.lock().unwrap().clone();
        if let Some(listener) = listener { listener(); }
    }
    pub(super) fn clear_dialog(&self, continue_request: bool) {
        let pending = self.dialog.lock().unwrap().take();
        self.changed(|state| state.dialog = None);
        self.protocol.set_modal_pending(false);
        if continue_request {
            if let Some(pending) = pending { pending.callback.cont(0, None); }
        }
    }

    fn record_permission_denial(&self, kind: &str) {
        self.changed(|state| {
            if !state
                .blocked_permissions
                .iter()
                .any(|value| value == kind)
            {
                state.blocked_permissions.push(kind.to_owned());
            }
        });
    }

    pub(super) fn clear_permissions(&self, record_denials: bool) {
        let pending = std::mem::take(&mut *self.permissions.lock().unwrap());
        if pending.is_empty() {
            return;
        }
        let denied = pending
            .values()
            .map(|permission| permission.kind.clone())
            .collect::<Vec<_>>();
        for permission in pending.into_values() {
            permission.finish(false);
        }
        self.changed(|state| {
            state.permission_requests.clear();
            if record_denials {
                for kind in denied {
                    if !state.blocked_permissions.contains(&kind) {
                        state.blocked_permissions.push(kind);
                    }
                }
            }
        });
    }

    fn offer_permission(
        self: &Arc<Self>,
        kind: String,
        origin: String,
        callback: DeferredPermissionCallback,
    ) -> bool {
        if kind.is_empty()
            || origin.is_empty()
            || self.input_locked.load(Ordering::Acquire)
            || !self.visible.load(Ordering::Acquire)
            || self.close_requested.load(Ordering::Acquire)
            || self.protocol.is_closed()
        {
            callback_to_denial(callback);
            self.record_permission_denial(if kind.is_empty() {
                "permission"
            } else {
                &kind
            });
            return false;
        }
        let generation = self.metadata.borrow().document_generation;
        let id = uuid::Uuid::now_v7().to_string();
        {
            let mut pending = self.permissions.lock().unwrap();
            if pending.len() >= 4 {
                drop(pending);
                callback_to_denial(callback);
                self.record_permission_denial(&kind);
                return false;
            }
            pending.insert(
                id.clone(),
                DeferredPermission {
                    id: id.clone(),
                    document_generation: generation,
                    kind: kind.clone(),
                    callback,
                },
            );
        }
        self.changed(|state| {
            state.permission_requests.push(BrowserPermissionRequest {
                request_id: id.clone(),
                kind,
                origin,
            });
        });
        let weak = Arc::downgrade(self);
        let timeout_id = id.clone();
        let scheduled = dispatch2::DispatchQueue::main().after(
            dispatch2::DispatchTime::try_from(std::time::Duration::from_secs(30))
                .expect("bounded permission timeout"),
            move || {
                let Some(page) = weak.upgrade() else {
                    return;
                };
                let permission = page.permissions.lock().unwrap().remove(&timeout_id);
                if let Some(permission) = permission {
                    let kind = permission.kind.clone();
                    permission.finish(false);
                    page.changed(|state| {
                        state
                            .permission_requests
                            .retain(|request| request.request_id != timeout_id);
                        if !state.blocked_permissions.contains(&kind) {
                            state.blocked_permissions.push(kind);
                        }
                    });
                }
            },
        );
        if scheduled.is_err() {
            if let Some(permission) = self.permissions.lock().unwrap().remove(&id) {
                permission.finish(false);
            }
            self.changed(|state| {
                state
                    .permission_requests
                    .retain(|request| request.request_id != id);
            });
            return false;
        }
        true
    }

    pub async fn reply_permission(
        self: &Arc<Self>,
        id: String,
        generation: u64,
        allow: bool,
    ) -> Result<(), String> {
        let page = self.clone();
        let (tx, rx) = oneshot::channel();
        self.engine.post(Box::new(move || {
            let result = (|| {
                if page.input_locked.load(Ordering::Acquire)
                    || !page.visible.load(Ordering::Acquire)
                    || page.close_requested.load(Ordering::Acquire)
                    || page.metadata.borrow().document_generation != generation
                {
                    return Err("CEF permission request is no longer actionable".to_owned());
                }
                let permission = page
                    .permissions
                    .lock()
                    .unwrap()
                    .remove(&id)
                    .ok_or_else(|| "CEF permission request is stale".to_owned())?;
                if permission.document_generation != generation || permission.id != id {
                    permission.finish(false);
                    return Err("CEF permission request belongs to an older document".to_owned());
                }
                let kind = permission.kind.clone();
                permission.finish(allow);
                page.changed(|state| {
                    state
                        .permission_requests
                        .retain(|request| request.request_id != id);
                    if !allow && !state.blocked_permissions.contains(&kind) {
                        state.blocked_permissions.push(kind);
                    }
                });
                Ok(())
            })();
            let _ = tx.send(result);
        }))?;
        rx.await
            .map_err(|_| "CEF permission reply acknowledgement was lost")?
    }
    fn offer_dialog(&self, kind: BrowserDialogKind, origin_url: Option<&CefString>, message: Option<&CefString>, default: Option<&CefString>, callback: &JsdialogCallback) -> bool {
        if self.dialog_draining.load(Ordering::Acquire) || self.close_requested.load(Ordering::Acquire)
            || self.protocol.is_closed() || self.dialog.lock().unwrap().is_some() { return false; }
        let (message, message_cut) = bounded(message, 4096);
        let (default_text, default_cut) = bounded(default, 4096);
        let id = uuid::Uuid::now_v7().to_string();
        *self.dialog.lock().unwrap() = Some(DeferredDialog { id: id.clone(), callback: callback.clone() });
        self.protocol.set_modal_pending(true);
        self.changed(|state| state.dialog = Some(NativeDialog { request_id: id, document_generation: state.document_generation,
            kind, message, default_text, origin: origin(origin_url), text_truncated: message_cut || default_cut }));
        true
    }
    /// Keep cancellation policy active until the owning operation has settled.
    /// Clearing a dialog never reissues the action which opened it.
    pub async fn set_dialog_draining(self: &Arc<Self>, draining: bool) -> Result<(), String> {
        let page = self.clone();
        let (tx, rx) = oneshot::channel();
        self.engine.post(Box::new(move || {
            page.dialog_draining.store(draining, Ordering::Release);
            if draining { page.clear_dialog(true); }
            let _ = tx.send(());
        }))?;
        rx.await.map_err(|_| "CEF dialog policy acknowledgement was lost".into())
    }
    pub async fn reply_dialog(self: &Arc<Self>, id: String, generation: u64, accept: bool, text: String, cancel: CancellationToken) -> Result<(), String> {
        if text.len() > 16384 { return Err("CEF dialog reply exceeds its limit".into()); }
        let page = self.clone();
        let (tx, rx) = oneshot::channel();
        self.engine.post(Box::new(move || {
            let result = (|| {
                if cancel.is_cancelled() || page.dialog_draining.load(Ordering::Acquire) || page.protocol.is_closed()
                    || page.close_requested.load(Ordering::Acquire) { return Err("CEF dialog reply was cancelled".to_owned()); }
                if page.metadata.borrow().document_generation != generation { return Err("CEF dialog belongs to an older document".into()); }
                let pending = {
                    let mut slot = page.dialog.lock().unwrap();
                    if slot.as_ref().is_none_or(|pending| pending.id != id) { return Err("CEF dialog reply is stale".into()); }
                    slot.take().unwrap()
                };
                // Remove first: Continue can synchronously expose another dialog.
                page.changed(|state| state.dialog = None);
                page.protocol.set_modal_pending(false);
                pending.callback.cont(i32::from(accept), Some(&CefString::from(text.as_str())));
                Ok(())
            })();
            let _ = tx.send(result);
        }))?;
        rx.await.map_err(|_| "CEF dialog reply acknowledgement was lost")?
    }
}

wrap_jsdialog_handler! { pub(super) struct Dialogs { page: Arc<Page>, } impl JsdialogHandler {
    fn on_jsdialog(&self, _browser: Option<&mut Browser>, origin_url: Option<&CefString>, dialog_type: JsdialogType, message_text: Option<&CefString>, default_prompt_text: Option<&CefString>, callback: Option<&mut JsdialogCallback>, suppress_message: Option<&mut i32>) -> i32 {
        let kind = if dialog_type == JsdialogType::ALERT { Some(BrowserDialogKind::Alert) }
            else if dialog_type == JsdialogType::CONFIRM { Some(BrowserDialogKind::Confirm) }
            else if dialog_type == JsdialogType::PROMPT { Some(BrowserDialogKind::Prompt) } else { None };
        if let (Some(kind), Some(callback)) = (kind, callback) {
            if self.page.offer_dialog(kind, origin_url, message_text, default_prompt_text, callback) { return 1; }
        }
        if let Some(suppress) = suppress_message { *suppress = 1; }
        0
    }
    fn on_before_unload_dialog(&self, browser: Option<&mut Browser>, message_text: Option<&CefString>, _is_reload: i32, callback: Option<&mut JsdialogCallback>) -> i32 {
        if let Some(callback) = callback {
            let url = browser.and_then(|browser| browser.main_frame()).map(|frame| CefString::from(&frame.url()));
            if !self.page.offer_dialog(BrowserDialogKind::BeforeUnload, url.as_ref(), message_text, None, callback) { callback.cont(0, None); }
        }
        1
    }
    fn on_reset_dialog_state(&self, _browser: Option<&mut Browser>) { self.page.clear_dialog(true); }
} }

wrap_load_handler! { pub(super) struct Loading { page: Arc<Page>, } impl LoadHandler {
    fn on_loading_state_change(&self, _browser: Option<&mut Browser>, is_loading: i32, can_go_back: i32, can_go_forward: i32) {
        self.page.changed(|state| {
            state.can_go_back = can_go_back != 0; state.can_go_forward = can_go_forward != 0;
            if is_loading != 0 { state.lifecycle = BrowserTabLifecycle::Loading; }
            else if state.lifecycle == BrowserTabLifecycle::Loading { state.lifecycle = BrowserTabLifecycle::Ready; }
        });
    }
    fn on_load_start(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, _transition_type: TransitionType) {
        if frame.is_some_and(|frame| frame.is_main() != 0) {
            self.page.clear_dialog(true);
            self.page.clear_permissions(false);
            self.page.cancel_surface_downloads();
            self.page.changed(|state| { state.document_generation = state.document_generation.saturating_add(1); state.lifecycle = BrowserTabLifecycle::Loading; state.blocked_permissions.clear(); state.permission_requests.clear(); });
        }
    }
    fn on_load_error(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, error_code: Errorcode, _error_text: Option<&CefString>, _failed_url: Option<&CefString>) {
        if error_code != Errorcode::ABORTED && frame.is_some_and(|frame| frame.is_main() != 0) {
            self.page.changed(|state| state.lifecycle = BrowserTabLifecycle::Failed);
        }
    }
} }
wrap_display_handler! { pub(super) struct Display { page: Arc<Page>, } impl DisplayHandler {
    fn on_address_change(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, url: Option<&CefString>) {
        if frame.is_some_and(|frame| frame.is_main() != 0) {
            self.page.changed(|state| state.url = bounded(url, 8192).0);
        }
    }
    fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) { self.page.changed(|state| state.title = bounded(title, 512).0); }
} }

// Until a runtime explicitly owns a permission deferral, deny rather than let
// CEF show an out-of-band prompt. The host records this as blocked, never granted.
wrap_permission_handler! { pub(super) struct Permissions { page: Arc<Page>, } impl PermissionHandler {
    fn on_request_media_access_permission(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, requesting_origin: Option<&CefString>, requested_permissions: u32, callback: Option<&mut MediaAccessCallback>) -> i32 {
        let camera = PermissionRequestTypes::CAMERA_STREAM.get_raw();
        let microphone = PermissionRequestTypes::MIC_STREAM.get_raw();
        let kind = match requested_permissions {
            value if value == camera => "camera",
            value if value == microphone => "microphone",
            value if value == camera | microphone => "camera_microphone",
            _ => "",
        };
        if let Some(callback) = callback {
            self.page.offer_permission(
                kind.to_owned(),
                origin(requesting_origin),
                DeferredPermissionCallback::Media { callback: callback.clone(), allowed: requested_permissions },
            );
        }
        1
    }
    fn on_show_permission_prompt(&self, _browser: Option<&mut Browser>, _prompt_id: u64, requesting_origin: Option<&CefString>, requested_permissions: u32, callback: Option<&mut PermissionPromptCallback>) -> i32 {
        let kind = if requested_permissions == PermissionRequestTypes::GEOLOCATION.get_raw() { "geolocation" }
            else if requested_permissions == PermissionRequestTypes::NOTIFICATIONS.get_raw() { "notifications" }
            else if requested_permissions == PermissionRequestTypes::CLIPBOARD.get_raw() { "clipboard" }
            else if requested_permissions == PermissionRequestTypes::MIDI_SYSEX.get_raw() { "midi" }
            else { "" };
        if let Some(callback) = callback {
            self.page.offer_permission(
                kind.to_owned(),
                origin(requesting_origin),
                DeferredPermissionCallback::Prompt(callback.clone()),
            );
        }
        1
    }
} }

fn callback_to_denial(callback: DeferredPermissionCallback) {
    match callback {
        DeferredPermissionCallback::Media { callback, .. } => callback.cancel(),
        DeferredPermissionCallback::Prompt(callback) => callback.cont(PermissionRequestResult::DENY),
    }
}

// These deferrals must be replaced by host-owned upload/download admission
// before production availability is enabled. Defaults would display native UI
// outside the RunGuard and cannot be allowed to run while an Agent owns input.
wrap_dialog_handler! { pub(super) struct FileDialogs; impl DialogHandler {
    fn on_file_dialog(&self, _browser: Option<&mut Browser>, _mode: FileDialogMode, _title: Option<&CefString>, _default_file_path: Option<&CefString>, _accept_filters: Option<&mut CefStringList>, _accept_extensions: Option<&mut CefStringList>, _accept_descriptions: Option<&mut CefStringList>, callback: Option<&mut FileDialogCallback>) -> i32 {
        if let Some(callback) = callback { callback.cancel(); }
        1
    }
} }
wrap_download_handler! { pub(super) struct Downloads { page: Arc<Page>, } impl DownloadHandler {
    fn can_download(&self, _browser: Option<&mut Browser>, url: Option<&CefString>, _request_method: Option<&CefString>) -> i32 { i32::from(self.page.can_download(url)) }
    fn on_before_download(&self, _browser: Option<&mut Browser>, download_item: Option<&mut DownloadItem>, suggested_name: Option<&CefString>, callback: Option<&mut BeforeDownloadCallback>) -> i32 {
        i32::from(self.page.begin_download(download_item, suggested_name, callback))
    }
    fn on_download_updated(&self, _browser: Option<&mut Browser>, download_item: Option<&mut DownloadItem>, callback: Option<&mut DownloadItemCallback>) {
        self.page.update_download(download_item, callback);
    }
} }
wrap_context_menu_handler! { pub(super) struct Menus { page: Arc<Page>, } impl ContextMenuHandler {
    fn on_before_context_menu(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _params: Option<&mut ContextMenuParams>, model: Option<&mut MenuModel>) {
        if let Some(model) = model {
            if self.page.input_locked() { model.clear(); }
            else { model.remove(cef::sys::cef_menu_id_t::MENU_ID_VIEW_SOURCE as i32); }
        }
    }
} }
