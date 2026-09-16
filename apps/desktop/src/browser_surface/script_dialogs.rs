//! UI-thread ownership of native script-dialog deferrals. No model or UI policy.
//! Install before the first HTML navigation (including popup binding). Callers
//! must authenticate run/user authority before replying and retain input owners
//! until the browser acknowledges their completion, not just this deferral.
use nomifun_browser_platform::runtime::{BrowserTabSnapshot, BrowserTabTarget, WorkspaceError};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::{Arc, Mutex},
};
use tokio::sync::{oneshot, watch};
use webview2_com::{
    CoTaskMemPWSTR, Microsoft::Web::WebView2::Win32::*, ScriptDialogOpeningEventHandler,
};
use windows::core::{HSTRING, PCWSTR, PWSTR};

const TEXT_LIMIT: usize = 4096;

pub(crate) use nomifun_browser_platform::runtime::{
    BrowserDialog as Dialog, BrowserDialogKind as DialogKind,
};

struct Pending {
    snapshot: Dialog,
    args: ICoreWebView2ScriptDialogOpeningEventArgs,
    deferral: ICoreWebView2Deferral,
}
struct State {
    draining: Cell<bool>,
    pending: RefCell<Option<Rc<Pending>>>,
    updates: watch::Sender<Option<Dialog>>,
    metadata: Arc<Mutex<BrowserTabSnapshot>>,
    revision: Arc<nomifun_browser_platform::revision::BrowserRevision>,
}
impl State {
    fn publish(&self, dialog: Option<Dialog>) {
        self.metadata
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .script_dialog = dialog.clone();
        self.updates.send_replace(dialog);
        self.revision.bump();
    }
    fn finish(
        &self,
        pending: &Pending,
        accept: bool,
        text: Option<&str>,
    ) -> Result<(), WorkspaceError> {
        validate_reply(pending.snapshot.kind, accept, text)?;
        // Do not hold RefCell borrows across COM: completion may pump messages.
        unsafe {
            if accept {
                if let Some(text) = text {
                    let value = HSTRING::from(text);
                    pending
                        .args
                        .SetResultText(PCWSTR(value.as_ptr()))
                        .map_err(|_| WorkspaceError::NativeCommandFailed)?;
                }
                if pending.snapshot.kind != DialogKind::Alert {
                    pending
                        .args
                        .Accept()
                        .map_err(|_| WorkspaceError::NativeCommandFailed)?;
                }
            }
            pending
                .deferral
                .Complete()
                .map_err(|_| WorkspaceError::NativeCommandFailed)?;
        }
        let mut slot = self.pending.borrow_mut();
        if slot
            .as_ref()
            .is_some_and(|current| current.snapshot.request_id == pending.snapshot.request_id)
        {
            slot.take();
            self.publish(None);
        }
        Ok(())
    }
    fn cancel(&self) -> Result<(), WorkspaceError> {
        let pending = self.pending.borrow().clone();
        if let Some(pending) = pending {
            self.finish(&pending, false, None)?;
        }
        Ok(())
    }
}
struct Registration {
    core: ICoreWebView2,
    token: i64,
    state: Rc<State>,
}
thread_local! {static REGISTRATIONS: RefCell<HashMap<String, Registration>> = RefCell::default();}
fn state(label: &str) -> Option<Rc<State>> {
    REGISTRATIONS.with(|entries| entries.borrow().get(label).map(|entry| entry.state.clone()))
}

fn validate_reply(
    kind: DialogKind,
    accept: bool,
    text: Option<&str>,
) -> Result<(), WorkspaceError> {
    if text.is_some_and(|text| {
        !accept || kind != DialogKind::Prompt || text.len() > 65536 || text.contains('\0')
    }) {
        return Err(WorkspaceError::NotActionable);
    }
    Ok(())
}
fn bounded(text: String) -> (String, bool) {
    if text.len() <= TEXT_LIMIT {
        return (text, false);
    }
    let mut end = TEXT_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}
fn page_origin(uri: &str) -> String {
    url::Url::parse(uri)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_else(|| "null".into())
}
fn native_text(
    read: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>,
) -> windows::core::Result<String> {
    let mut value = PWSTR::null();
    let result = read(&mut value);
    let owned = CoTaskMemPWSTR::from(value);
    result?;
    Ok(owned.to_string())
}

pub(crate) async fn install(
    view: &tauri::Webview,
    metadata: Arc<Mutex<BrowserTabSnapshot>>,
    revision: Arc<nomifun_browser_platform::revision::BrowserRevision>,
) -> Result<watch::Receiver<Option<Dialog>>, WorkspaceError> {
    let label = view.label().to_owned();
    let (tx, rx) = oneshot::channel();
    view.with_webview(move |platform| {
        let result = (|| -> Result<_, WorkspaceError> {
            // Duplicate installation must not silently bind a different owner.
            if state(&label).is_some() {
                return Err(WorkspaceError::NotActionable);
            }
            let core = unsafe { platform.controller().CoreWebView2() }
                .map_err(|_| WorkspaceError::NativeCommandFailed)?;
            let (updates, receiver) = watch::channel(None);
            let state = Rc::new(State {
                draining: Cell::new(false),
                pending: RefCell::new(None),
                updates,
                metadata,
                revision,
            });
            let event_state = state.clone();
            let handler = ScriptDialogOpeningEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                // WebView2 serializes a page's modal dialogs. If a competing
                // frame emits another, cancel it without losing the first owner.
                if event_state.draining.get() || event_state.pending.borrow().is_some() {
                    return Ok(());
                }
                let mut native_kind = COREWEBVIEW2_SCRIPT_DIALOG_KIND::default();
                unsafe {
                    args.Kind(&mut native_kind)?;
                }
                let kind = match native_kind {
                    COREWEBVIEW2_SCRIPT_DIALOG_KIND_ALERT => DialogKind::Alert,
                    COREWEBVIEW2_SCRIPT_DIALOG_KIND_CONFIRM => DialogKind::Confirm,
                    COREWEBVIEW2_SCRIPT_DIALOG_KIND_PROMPT => DialogKind::Prompt,
                    COREWEBVIEW2_SCRIPT_DIALOG_KIND_BEFOREUNLOAD => DialogKind::BeforeUnload,
                    _ => return Ok(()),
                };
                let (message, message_cut) =
                    bounded(native_text(|value| unsafe { args.Message(value) })?);
                let (default_text, default_cut) =
                    bounded(native_text(|value| unsafe { args.DefaultText(value) })?);
                let uri = native_text(|value| unsafe { args.Uri(value) })?;
                let snapshot = Dialog {
                    request_id: uuid::Uuid::now_v7().to_string(),
                    target: event_state
                        .metadata
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .target
                        .clone(),
                    kind,
                    message,
                    default_text,
                    origin: page_origin(&uri),
                    text_truncated: message_cut || default_cut,
                };
                let pending = Rc::new(Pending {
                    snapshot: snapshot.clone(),
                    deferral: unsafe { args.GetDeferral()? },
                    args,
                });
                *event_state.pending.borrow_mut() = Some(pending);
                event_state.publish(Some(snapshot));
                Ok(())
            }));
            let mut token = 0;
            unsafe { core.add_ScriptDialogOpening(&handler, &mut token) }
                .map_err(|_| WorkspaceError::NativeCommandFailed)?;
            if unsafe {
                core.Settings()
                    .and_then(|settings| settings.SetAreDefaultScriptDialogsEnabled(false))
            }
            .is_err()
            {
                let _ = unsafe { core.remove_ScriptDialogOpening(token) };
                return Err(WorkspaceError::NativeCommandFailed);
            }
            REGISTRATIONS.with(|entries| {
                entries
                    .borrow_mut()
                    .insert(label, Registration { core, token, state });
            });
            Ok(receiver)
        })();
        let _ = tx.send(result);
    })
    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    rx.await.map_err(|_| WorkspaceError::NativeCommandFailed)?
}

pub(crate) async fn respond(
    view: &tauri::Webview,
    target: BrowserTabTarget,
    request_id: String,
    accept: bool,
    text: Option<String>,
) -> Result<(), WorkspaceError> {
    let label = view.label().to_owned();
    let (tx, rx) = oneshot::channel();
    view.with_webview(move |_| {
        let result = (|| {
            let state = state(&label).ok_or(WorkspaceError::StaleTarget)?;
            let pending = state
                .pending
                .borrow()
                .clone()
                .ok_or(WorkspaceError::StaleTarget)?;
            if pending.snapshot.request_id != request_id
                || pending.snapshot.target != target
                || state
                    .metadata
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .target
                    != target
            {
                return Err(WorkspaceError::StaleTarget);
            }
            state.finish(&pending, accept, text.as_deref())
        })();
        let _ = tx.send(result);
    })
    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    rx.await.map_err(|_| WorkspaceError::NativeCommandFailed)?
}

/// Run before awaiting input/popup locks. On failure the registration and
/// deferral remain owned so cleanup can retry or destroy the native controller.
pub(crate) async fn cancel(view: &tauri::Webview) -> Result<(), WorkspaceError> {
    let label = view.label().to_owned();
    let (tx, rx) = oneshot::channel();
    view.with_webview(move |_| {
        let _ = tx.send(state(&label).map_or(Ok(()), |state| state.cancel()));
    })
    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    rx.await.map_err(|_| WorkspaceError::NativeCommandFailed)?
}

/// Host-only subscription, including an already pending dialog. No new owner
/// or page is created by asking whether this view has native dialog support.
pub(crate) async fn subscribe(
    view: &tauri::Webview,
) -> Result<Option<watch::Receiver<Option<Dialog>>>, WorkspaceError> {
    let label = view.label().to_owned();
    let (tx, rx) = oneshot::channel();
    view.with_webview(move |_| {
        let _ = tx.send(state(&label).map(|state| state.updates.subscribe()));
    })
    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    rx.await.map_err(|_| WorkspaceError::NativeCommandFailed)
}

/// Stop must dismiss dialogs produced by the remainder of the interrupted JS
/// callback too (for example two consecutive alerts), before waiting for input.
pub(crate) async fn drain(view: &tauri::Webview) -> Result<(), WorkspaceError> {
    let label = view.label().to_owned();
    let (tx, rx) = oneshot::channel();
    view.with_webview(move |_| {
        let _ = tx.send(state(&label).map_or(Ok(()), |state| {
            state.draining.set(true);
            state.cancel()
        }));
    })
    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    rx.await.map_err(|_| WorkspaceError::NativeCommandFailed)?
}

pub(crate) async fn resume(view: &tauri::Webview) -> Result<(), WorkspaceError> {
    let label = view.label().to_owned();
    let (tx, rx) = oneshot::channel();
    view.with_webview(move |_| {
        if let Some(state) = state(&label) {
            state.draining.set(false);
        }
        let _ = tx.send(());
    })
    .map_err(|_| WorkspaceError::NativeCommandFailed)?;
    rx.await.map_err(|_| WorkspaceError::NativeCommandFailed)
}

/// A confirmed document/process exit makes its pending deferral unreachable.
/// Keep the controller's handler registered for a later explicit reload.
pub(super) fn document_exited(label: &str) {
    if let Some(state) = state(label) {
        let had_pending = state.pending.borrow_mut().take().is_some();
        if had_pending {
            state.publish(None);
        }
    }
}

/// Only after confirmed native destruction; dropping an observer is not proof
/// that a paused page or the action which opened its dialog has ended.
pub(super) fn close_view(label: &str) {
    if let Some(entry) = REGISTRATIONS.with(|entries| entries.borrow_mut().remove(label)) {
        let _ = unsafe { entry.core.remove_ScriptDialogOpening(entry.token) };
        entry.state.pending.borrow_mut().take();
        entry.state.publish(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn page_text_is_bounded_on_unicode_boundaries() {
        let (text, truncated) = bounded("界".repeat(2000));
        assert!(truncated);
        assert!(text.len() <= TEXT_LIMIT);
        assert!(text.chars().all(|value| value == '界'));
        assert_eq!(bounded("hello".into()), ("hello".into(), false));
    }
    #[test]
    fn origin_never_exposes_credentials_query_or_path() {
        assert_eq!(
            page_origin("https://user:password@example.com/private?token=secret#fragment"),
            "https://example.com"
        );
        assert_eq!(page_origin("data:text/plain,secret"), "null");
        assert_eq!(page_origin("file:///C:/private/secret"), "null");
    }
    #[test]
    fn prompt_text_is_not_a_generic_dialog_command() {
        assert!(validate_reply(DialogKind::Confirm, true, Some("yes")).is_err());
        assert!(validate_reply(DialogKind::Prompt, false, Some("text")).is_err());
        assert!(validate_reply(DialogKind::Prompt, true, Some("nul\0tail")).is_err());
        assert!(validate_reply(DialogKind::Prompt, true, Some(&"x".repeat(65537))).is_err());
        assert!(validate_reply(DialogKind::Prompt, true, Some("中文 reply")).is_ok());
        assert!(validate_reply(DialogKind::Alert, false, None).is_ok());
    }
}
