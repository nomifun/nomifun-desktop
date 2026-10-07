//! Native website permissions. Requests belong to the visible human page only.
use nomifun_browser_platform::{revision::BrowserRevision, runtime::*};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio_util::sync::CancellationToken;
use webview2_com::{Microsoft::Web::WebView2::Win32::*, PermissionRequestedEventHandler};
use windows::core::Interface;

const MAX_PENDING: usize = 4;
const REQUEST_LIFETIME: std::time::Duration = std::time::Duration::from_secs(30);

struct Pending {
    id: String,
    kind: String,
    origin: String,
    target: BrowserTabTarget,
    args: ICoreWebView2PermissionRequestedEventArgs,
    deferral: Option<ICoreWebView2Deferral>,
    metadata: Arc<Mutex<BrowserTabSnapshot>>,
    revision: Arc<BrowserRevision>,
    timer: CancellationToken,
}
impl Pending {
    fn finish(&mut self, allow: bool) -> Result<(), String> {
        unsafe {
            self.args
                .SetState(if allow {
                    COREWEBVIEW2_PERMISSION_STATE_ALLOW
                } else {
                    COREWEBVIEW2_PERMISSION_STATE_DENY
                })
                .map_err(|_| "Permission decision could not be applied.")?;
            if let Some(deferral) = &self.deferral {
                deferral
                    .Complete()
                    .map_err(|_| "Permission request could not be completed.")?;
            }
        }
        self.deferral = None;
        Ok(())
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.timer.cancel();
        if self.deferral.is_some() {
            let _ = self.finish(false);
        }
        self.metadata
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .permission_requests
            .retain(|request| request.request_id != self.id);
        self.revision.bump();
    }
}
struct State {
    visible: Cell<bool>,
    locked: Arc<AtomicBool>,
    pending: RefCell<HashMap<String, Pending>>,
    denied_generation: Cell<u64>,
    cancelled: RefCell<HashSet<(String, String)>>,
}
impl State {
    fn document(&self, generation: u64) {
        if self.denied_generation.replace(generation) != generation {
            self.cancelled.borrow_mut().clear();
        }
    }
    fn cancel_request(&self, pending: &Pending) {
        self.document(pending.target.document_generation);
        let mut cancelled = self.cancelled.borrow_mut();
        if cancelled.len() < 64 {
            cancelled.insert((pending.origin.clone(), pending.kind.clone()));
        }
        record_denial(&pending.metadata, &pending.revision, &pending.kind);
    }
    fn cancelled(&self, generation: u64, origin: &str, kind: &str) -> bool {
        self.document(generation);
        let cancelled = self.cancelled.borrow();
        cancelled.contains(&(origin.to_owned(), kind.to_owned())) || cancelled.len() >= 64
    }
}
struct Registration {
    core: ICoreWebView2,
    token: i64,
    state: Rc<State>,
}
impl Drop for Registration {
    fn drop(&mut self) {
        let _ = unsafe { self.core.remove_PermissionRequested(self.token) };
        let pending = std::mem::take(&mut *self.state.pending.borrow_mut());
        drop(pending);
    }
}
thread_local! {static REGISTRATIONS:RefCell<HashMap<String,Registration>>=RefCell::default();}
fn lookup_state(label: &str) -> Option<Rc<State>> {
    REGISTRATIONS.with(|entries| entries.borrow().get(label).map(|entry| entry.state.clone()))
}
pub(crate) fn deny_pending(label: &str) -> Result<(), String> {
    let mut failure = None;
    if let Some(state) = lookup_state(label) {
        let pending = std::mem::take(&mut *state.pending.borrow_mut());
        for mut request in pending.into_values() {
            state.cancel_request(&request);
            if let Err(error) = request.finish(false) {
                failure = Some(error);
            }
        }
    }
    failure.map_or(Ok(()), Err)
}
pub(crate) fn set_visible(label: &str, visible: bool) -> Result<(), String> {
    if let Some(state) = lookup_state(label) {
        state.visible.set(visible);
    }
    if !visible {
        deny_pending(label)?;
    }
    Ok(())
}
pub(crate) fn navigation_started(label: &str) -> Result<(),String> {
    let result=deny_pending(label);
    if let Some(state)=lookup_state(label) {state.cancelled.borrow_mut().clear();}
    result
}
pub(super) fn close_view(label: &str) {
    let entry = REGISTRATIONS.with(|entries| entries.borrow_mut().remove(label));
    drop(entry);
}
fn permission_name(kind: COREWEBVIEW2_PERMISSION_KIND) -> &'static str {
    match kind {
        COREWEBVIEW2_PERMISSION_KIND_CAMERA => "camera",
        COREWEBVIEW2_PERMISSION_KIND_MICROPHONE => "microphone",
        COREWEBVIEW2_PERMISSION_KIND_GEOLOCATION => "geolocation",
        COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS => "notifications",
        COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ => "clipboard-read",
        COREWEBVIEW2_PERMISSION_KIND_MIDI_SYSTEM_EXCLUSIVE_MESSAGES => "midi",
        _ => "other",
    }
}
fn origin(uri: &str) -> Option<String> {
    let url = url::Url::parse(uri).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() || uri.len() > 8192 {
        return None;
    }
    Some(url.origin().ascii_serialization())
}
fn record_denial(metadata: &Mutex<BrowserTabSnapshot>, revision: &BrowserRevision, kind: &str) {
    let mut data = metadata.lock().unwrap_or_else(|e| e.into_inner());
    if !data.blocked_permissions.iter().any(|entry| entry == kind) {
        data.blocked_permissions.push(kind.into());
        drop(data);
        revision.bump();
    }
}
pub(crate) async fn respond(
    view: &tauri::Webview,
    target: BrowserTabTarget,
    request_id: String,
    allow: bool,
) -> Result<(), WorkspaceError> {
    let label = view.label().to_owned();
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |_| {
        let result = (|| {
            let state = lookup_state(&label).ok_or(WorkspaceError::StaleTarget)?;
            if state.locked.load(Ordering::Acquire) || !state.visible.get() {
                return Err(WorkspaceError::NotActionable);
            }
            let pending = state.pending.borrow_mut().remove(&request_id);
            let mut pending = pending.ok_or(WorkspaceError::StaleTarget)?;
            if pending.target != target
                || pending
                    .metadata
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .target
                    != target
            {
                return Err(WorkspaceError::StaleTarget);
            }
            pending
                .finish(allow)
                .map_err(|_| WorkspaceError::NativeCommandFailed)?;
            Ok(())
        })();
        let _ = tx.send(result);
    })
    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    rx.await.map_err(|_| WorkspaceError::NativeCommandFailed)?
}
pub(crate) async fn install(
    view: &tauri::Webview,
    metadata: Arc<Mutex<BrowserTabSnapshot>>,
    revision: Arc<BrowserRevision>,
    locked: Arc<AtomicBool>,
) -> Result<(), String> {
    let label = view.label().to_owned();
    let timer_view = view.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result = (|| -> windows::core::Result<()> {
            if lookup_state(&label).is_some() {
                return Ok(());
            }
            let core = unsafe { platform.controller().CoreWebView2()? };
            let state = Rc::new(State {
                visible: Cell::new(false),
                locked,
                pending: RefCell::default(),
                denied_generation: Cell::new(0),
                cancelled: RefCell::default(),
            });
            let event_state = state.clone();
            let event_label = label.clone();
            let handler = PermissionRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                // Deny before any allocation or parsing. Never fall through to
                // native OS/browser prompts outside our run input gate.
                unsafe {
                    args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                }
                let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
                unsafe {
                    args.PermissionKind(&mut kind)?;
                }
                let name = permission_name(kind);
                let saves = args.cast::<ICoreWebView2PermissionRequestedEventArgs3>();
                let Ok(saves) = saves else {
                    record_denial(&metadata, &revision, name);
                    return Ok(());
                };
                unsafe {
                    saves.SetSavesInProfile(false)?;
                }
                if event_state.locked.load(Ordering::Acquire)
                    || !event_state.visible.get()
                    || name == "other"
                    || event_state.pending.borrow().len() >= MAX_PENDING
                {
                    record_denial(&metadata, &revision, name);
                    return Ok(());
                }
                let mut raw = windows::core::PWSTR::null();
                let status = unsafe { args.Uri(&mut raw) };
                let uri = super::event_string(raw, 8192);
                status?;
                let Some(origin) = uri.as_deref().and_then(origin) else {
                    record_denial(&metadata, &revision, name);
                    return Ok(());
                };
                let target = metadata
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .target
                    .clone();
                // WebView2 may reissue a cancelled request on visibility restore,
                // retaining IsUserInitiated=true. That bit cannot prove a fresh
                // user retry. Keep automatic cancellation denied for this document;
                // ordinary navigation/refresh starts a clean decision scope.
                if event_state.cancelled(target.document_generation, &origin, name) {
                    record_denial(&metadata, &revision, name);
                    return Ok(());
                }
                let id = uuid::Uuid::now_v7().to_string();
                let timer = CancellationToken::new();
                let pending = Pending {
                    id: id.clone(),
                    kind: name.into(),
                    origin: origin.clone(),
                    target,
                    args: args.clone(),
                    deferral: Some(unsafe { args.GetDeferral()? }),
                    metadata: metadata.clone(),
                    revision: revision.clone(),
                    timer: timer.clone(),
                };
                event_state.pending.borrow_mut().insert(id.clone(), pending);
                metadata
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .permission_requests
                    .push(BrowserPermissionRequest {
                        request_id: id.clone(),
                        kind: name.into(),
                        origin,
                    });
                revision.bump();
                let view = timer_view.clone();
                let label = event_label.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::select! {
                        _=timer.cancelled()=>{},
                        _=tokio::time::sleep(REQUEST_LIFETIME)=>{
                            let _=view.with_webview(move |_| {
                                if let Some(state)=lookup_state(&label) {
                                    let pending=state.pending.borrow_mut().remove(&id);
                                    if let Some(pending)=&pending {state.cancel_request(pending);}
                                    drop(pending);
                                }
                            });
                        }
                    }
                });
                Ok(())
            }));
            let mut token = 0;
            unsafe {
                core.add_PermissionRequested(&handler, &mut token)?;
            }
            REGISTRATIONS.with(|entries| {
                entries
                    .borrow_mut()
                    .insert(label, Registration { core, token, state })
            });
            Ok(())
        })();
        let _ = tx.send(
            result.map_err(|_| "Native permission policy could not be installed.".to_owned()),
        );
    })
    .map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())?
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_denial_is_document_scoped_and_bounded() {
        let state = State {
            visible: Cell::new(true),
            locked: Arc::new(AtomicBool::new(false)),
            pending: RefCell::default(),
            denied_generation: Cell::new(7),
            cancelled: RefCell::default(),
        };
        state
            .cancelled
            .borrow_mut()
            .insert(("https://example.test".into(), "geolocation".into()));
        assert!(state.cancelled(7, "https://example.test", "geolocation"));
        assert!(!state.cancelled(7, "https://example.test", "camera"));
        for i in 1..64 {
            state
                .cancelled
                .borrow_mut()
                .insert((format!("https://site-{i}.test"), "geolocation".into()));
        }
        assert!(state.cancelled(7, "https://new.test", "camera"));
        assert!(!state.cancelled(8, "https://example.test", "geolocation"));
        assert!(state.cancelled.borrow().is_empty());
    }
    #[test]
    fn unknown_permission_kinds_are_denied() {
        assert_eq!(
            permission_name(COREWEBVIEW2_PERMISSION_KIND(99999)),
            "other"
        );
    }
    #[test]
    fn origin_does_not_expose_request_paths_credentials_or_queries() {
        assert_eq!(
            origin("https://user:secret@example.test/path?token=secret#x").as_deref(),
            Some("https://example.test")
        );
        assert_eq!(
            origin("http://localhost:5173/path").as_deref(),
            Some("http://localhost:5173")
        );
        assert!(origin("file:///private/path").is_none());
        assert!(origin("data:text/plain,secret").is_none());
    }
}
