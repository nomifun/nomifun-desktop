//! WebView2 transport scoped to an existing embedded browser view.
//! These methods are host-only and are never registered as Tauri commands.

use serde_json::Value;
// Shared semantic algorithms use this type; Windows still owns a WebView2 view.
pub(crate) type View = tauri::Webview;
use std::{
    cell::RefCell,
    collections::{HashMap,HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{mpsc, oneshot};
use tauri::Manager;
use webview2_com::{
    CallDevToolsProtocolMethodCompletedHandler, CoTaskMemPWSTR,
    DevToolsProtocolEventReceivedEventHandler,
    Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_11, ICoreWebView2DevToolsProtocolEventReceivedEventArgs2,
        ICoreWebView2DevToolsProtocolEventReceiver,
    },
};
use windows::{
    Win32::{
        Foundation::HWND,
        UI::{
            Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled},
            WindowsAndMessaging::GetParent,
        },
    },
    core::{HSTRING, Interface, PCWSTR, PWSTR},
};

#[path = "diagnostics.rs"]
pub(crate) mod diagnostics;
#[path = "frame_sessions.rs"]
pub(crate) mod frames;
#[path = "navigation_metadata.rs"]
pub(crate) mod navigation_metadata;
#[path = "windows_navigation.rs"]
pub(crate) mod navigation;
#[path = "navigation_request_gate.rs"]
pub(crate) mod navigation_request_gate;
#[path = "windows_presentation.rs"]
pub(crate) mod presentation;
#[path = "effective_environment.rs"]
pub(crate) mod effective_environment;
#[path = "permissions.rs"]
pub(crate) mod permissions;
#[path = "script_dialogs.rs"]
pub(crate) mod script_dialogs;
#[path = "popup.rs"]
pub(crate) mod popup;
#[path = "process_failure.rs"]
pub(crate) mod process_failure;
#[path = "file_chooser.rs"]
pub(crate) mod file_chooser;
#[path = "user_file_picker.rs"]
pub(crate) mod user_file_picker;
#[path = "file_accept.rs"]
mod file_accept;
#[path = "user_file_chooser.rs"]
pub(crate) mod user_file_chooser;
#[path = "user_downloads.rs"]
pub(crate) mod user_downloads;
#[path = "external_browser.rs"]
pub(crate) mod external_browser;
#[path = "shortcuts.rs"]
pub(crate) mod shortcuts;
#[path = "site_data.rs"]
pub(crate) mod site_data;

struct EventRegistration {
    view_label: String,
    receivers: Vec<(ICoreWebView2DevToolsProtocolEventReceiver, i64)>,
    invalid: Arc<AtomicBool>,
}

impl Drop for EventRegistration {
    fn drop(&mut self) {
        self.invalid.store(true, Ordering::Release);
        for (receiver, token) in &self.receivers {
            // All registrations, removals and COM releases stay on the UI thread.
            let _ = unsafe { receiver.remove_DevToolsProtocolEventReceived(*token) };
        }
    }
}

thread_local! {
    static EVENTS: RefCell<HashMap<uuid::Uuid, EventRegistration>> = RefCell::default();
    static COMMANDS: RefCell<HashMap<uuid::Uuid, PendingCommand>> = RefCell::default();
    // Only the interval between confirmed COM Close and Tauri unregistration.
    // Healthy closes remove entries; failed unregistration retains its proof.
    static CLOSED_VIEWS: RefCell<HashSet<String>> = RefCell::default();
}

fn native_closed(label:&str)->bool { CLOSED_VIEWS.with(|views|views.borrow().contains(label)) }
pub(super) fn forget_closed_if_unregistered(view:&tauri::Webview)->bool {
    if view.app_handle().get_webview(view.label()).is_some() {return false;}
    CLOSED_VIEWS.with(|views|views.borrow_mut().remove(view.label()));
    true
}

type CommandSender = std::rc::Rc<RefCell<Option<oneshot::Sender<Result<Value, String>>>>>;
struct PendingCommand {
    label: String,
    sender: CommandSender,
    navigation: Option<PendingNavigation>,
    before_unload: BeforeUnloadDecision,
    deferred_response: Option<Result<Value, String>>,
}
struct PendingNavigation {
    receipt: navigation::Receipt,
    source: nomifun_browser_platform::runtime::BrowserNavigationSource,
    navigate: bool,
    rejected: Arc<AtomicBool>,
}
#[derive(Default, PartialEq, Eq)]
enum BeforeUnloadDecision { #[default] None, Pending, Rejected }
#[derive(Clone, Copy)]
pub(super) struct BeforeUnloadRequest { command: uuid::Uuid, receipt: navigation::Receipt }

// This is a scope for the explicit same-document Stay control, not proof that
// this host command caused the dialog. Page/iframe timers can also open it.
pub(super) fn before_unload_request(label: &str) -> Option<BeforeUnloadRequest> {
    let receipt = navigation::requested_receipt(label)?;
    COMMANDS.with(|commands| commands.borrow().iter().find_map(|(id, pending)| {
        (pending.label == label && pending.navigation.as_ref().is_some_and(|nav| nav.receipt == receipt))
            .then_some(BeforeUnloadRequest { command: *id, receipt })
    }))
}
pub(super) fn begin_before_unload_rejection(label: &str, request: BeforeUnloadRequest) -> Result<bool, nomifun_browser_platform::runtime::WorkspaceError> {
    if navigation::requested_receipt(label) != Some(request.receipt) { return Ok(false); }
    let retained = COMMANDS.with(|commands| {
        let mut commands = commands.borrow_mut();
        let Some(pending) = commands.get_mut(&request.command) else { return false; };
        if pending.label != label || pending.before_unload != BeforeUnloadDecision::None
            || !pending.navigation.as_ref().is_some_and(|nav| nav.receipt == request.receipt) { return false; }
        pending.before_unload = BeforeUnloadDecision::Pending;
        true
    });
    if !retained { return Ok(false); }
    // Stay explicitly stops only the captured current request. It does not
    // attribute an unsolicited dialog or its callback to that request.
    match navigation::stop_requested_before_unload(label, request.receipt) {
        Ok(true) => Ok(true),
        Ok(false) => { finish_before_unload_decision(request.command, false); Ok(false) },
        Err(_) => {
            finish_before_unload_decision(request.command, false);
            Err(nomifun_browser_platform::runtime::WorkspaceError::NativeCommandFailed)
        },
    }
}
pub(super) fn complete_before_unload_rejection(label: &str, request: BeforeUnloadRequest, acknowledged: bool) {
    let rejected = acknowledged && navigation::before_unload_stopped_matches(label, request.receipt);
    finish_before_unload_decision(request.command, rejected);
}
fn finish_before_unload_decision(command: uuid::Uuid, rejected: bool) {
    let response = COMMANDS.with(|commands| {
        let mut commands = commands.borrow_mut();
        let Some(pending) = commands.get_mut(&command) else { return None; };
        if pending.before_unload != BeforeUnloadDecision::Pending { return None; }
        pending.before_unload = if rejected { BeforeUnloadDecision::Rejected } else { BeforeUnloadDecision::None };
        pending.deferred_response.take()
    });
    if let Some(response) = response { finish_protocol_command(command, response); }
}
fn finish_protocol_command(id: uuid::Uuid, result: Result<Value, String>) {
    let mut result = Some(result);
    let pending = COMMANDS.with(|commands| {
        let mut commands = commands.borrow_mut();
        let pending = commands.get_mut(&id)?;
        // Deferral.Complete may pump this callback. Its result is retained until
        // the exact false reply has either acknowledged or failed.
        if pending.before_unload == BeforeUnloadDecision::Pending {
            pending.deferred_response = result.take();
            return None;
        }
        commands.remove(&id)
    });
    let Some(pending) = pending else { return; };
    let result = result.unwrap();
    if let Some(nav) = pending.navigation {
        let failed = result.as_ref().map_or(true, navigation_response_failed);
        let download = result.as_ref().is_ok_and(|value| value["isDownload"].as_bool() == Some(true));
        let same_document = nav.navigate && !failed && !download && result.as_ref().is_ok_and(|value| value.get("loaderId").is_none());
        if known_before_unload_abort(&pending.before_unload, &result) && !download {
            nav.rejected.store(true, Ordering::Release);
            navigation::settled_rejection(&pending.label, nav.receipt, nav.source);
        } else {
            if failed {
                navigation::settled_protocol_failure(&pending.label, nav.receipt, nav.source, result.as_ref().ok().and_then(|value|value["errorText"].as_str()));
            } else {
                navigation::settled(&pending.label, nav.receipt, nav.source, false, download, same_document);
            }
        }
    }
    if let Some(sender) = pending.sender.borrow_mut().take() { let _ = sender.send(result); }
}
fn known_before_unload_abort(decision: &BeforeUnloadDecision, result: &Result<Value, String>) -> bool {
    *decision == BeforeUnloadDecision::Rejected && result.as_ref().is_ok_and(|value|value["errorText"].as_str()==Some("net::ERR_ABORTED") && value["isDownload"].as_bool()!=Some(true))
}
pub(crate) fn navigation_response_failed(response: &Value) -> bool {
    // Chromium downloads deliberately abort document navigation. The separate
    // positive download result hands settlement to the native transfer owner.
    response.get("errorText").is_some() && response["isDownload"].as_bool()!=Some(true)
}

/// A confirmed renderer/browser exit is native settlement evidence: the old
/// document cannot finish its command. Do not wait forever for its callback.
pub(super) fn fail_pending_commands(label: &str) {
    user_file_chooser::cancel(label);
    user_downloads::cancel_pending(label);
    script_dialogs::document_exited(label);
    let pending = COMMANDS.with(|commands| {
        let mut commands = commands.borrow_mut();
        let ids: Vec<_> = commands
            .iter()
            .filter(|(_, pending)| pending.label == label)
            .map(|(id, _)| *id)
            .collect();
        ids.into_iter()
            .filter_map(|id| commands.remove(&id))
            .collect::<Vec<_>>()
    });
    for pending in pending {
        if let Some(sender) = pending.sender.borrow_mut().take() {
            let _ = sender.send(Err(
                "Browser process ended before protocol completion.".into()
            ));
        }
    }
}

struct ProtocolEvent {
    method: &'static str,
    parent_session: String,
    parameters: String,
}

enum ProtocolMessage {
    Event(ProtocolEvent),
    Barrier(oneshot::Sender<()>),
}

fn enqueue_event(
    sender: &mpsc::Sender<ProtocolMessage>,
    invalid: &AtomicBool,
    event: Option<ProtocolEvent>,
) {
    if !event.is_some_and(|event| sender.try_send(ProtocolMessage::Event(event)).is_ok()) {
        invalid.store(true, Ordering::Release);
    }
}

struct ProtocolEvents {
    id: uuid::Uuid,
    view: tauri::Webview,
    receiver: Option<mpsc::Receiver<ProtocolMessage>>,
    sender: mpsc::Sender<ProtocolMessage>,
    invalid: Arc<AtomicBool>,
}

impl Drop for ProtocolEvents {
    fn drop(&mut self) {
        let id = self.id;
        let _ = self.view.with_webview(move |platform| {
            let registration = EVENTS.with(|events| events.borrow_mut().remove(&id));
            if registration.is_some() {
                // There is only one auto-attach owner per native view. Stop its
                // discovery when the owner goes away; controller close remains
                // the definitive cleanup proof when the Runtime is destroyed.
                let callback =
                    CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(())));
                let method = HSTRING::from("Target.setAutoAttach");
                let params = HSTRING::from(
                    r#"{"autoAttach":false,"waitForDebuggerOnStart":false,"flatten":true}"#,
                );
                let _ = unsafe {
                    platform.controller().CoreWebView2().and_then(|core| {
                        core.CallDevToolsProtocolMethod(
                            PCWSTR(method.as_ptr()),
                            PCWSTR(params.as_ptr()),
                            &callback,
                        )
                    })
                };
                drop(registration);
            }
        });
    }
}

/// Copy only bounded event data. The CoTaskMem guard frees even rejected values.
fn event_string(raw: PWSTR, limit: usize) -> Option<String> {
    let allocation = CoTaskMemPWSTR::from(raw);
    let result = if raw.is_null() {
        Some(String::new())
    } else {
        // SAFETY: WebView2 returns a NUL-terminated, allocated UTF-16 string.
        let text = unsafe { raw.as_wide() };
        (text.len() <= limit).then(|| String::from_utf16_lossy(text))
    };
    drop(allocation);
    result
}

async fn listen_frames(view: &tauri::Webview) -> Result<ProtocolEvents, String> {
    let id = uuid::Uuid::now_v7();
    let label = view.label().to_owned();
    let (sender, receiver) = mpsc::channel(64);
    let control_sender = sender.clone();
    let invalid = Arc::new(AtomicBool::new(false));
    let callback_invalid = invalid.clone();
    let (tx, rx) = oneshot::channel();
    view.with_webview(move |platform| {
        let result = (|| -> windows::core::Result<()> {
            if EVENTS.with(|events| {
                events
                    .borrow()
                    .values()
                    .any(|entry| entry.view_label == label)
            }) {
                return Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_UNEXPECTED,
                ));
            }
            let core = unsafe { platform.controller().CoreWebView2()? };
            let mut registration = EventRegistration {
                view_label: label,
                receivers: vec![],
                invalid: callback_invalid.clone(),
            };
            for method in ["Target.attachedToTarget", "Target.detachedFromTarget"] {
                let sender = sender.clone();
                let invalid = callback_invalid.clone();
                let handler =
                    DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, args| {
                        let data = (|| -> windows::core::Result<Option<ProtocolEvent>> {
                            let Some(args) = args else { return Ok(None) };
                            let mut raw = PWSTR::null();
                            let status = unsafe { args.ParameterObjectAsJson(&mut raw) };
                            let parameters = event_string(raw, 64 * 1024);
                            status?;
                            let args = args
                                .cast::<ICoreWebView2DevToolsProtocolEventReceivedEventArgs2>()?;
                            let mut raw = PWSTR::null();
                            let status = unsafe { args.SessionId(&mut raw) };
                            let parent_session = event_string(raw, 256);
                            status?;
                            Ok(parameters.zip(parent_session).map(
                                |(parameters, parent_session)| ProtocolEvent {
                                    method,
                                    parent_session,
                                    parameters,
                                },
                            ))
                        })();
                        // Losing one lifecycle event invalidates routing, never silently
                        // continue using a possibly detached or reassigned session.
                        enqueue_event(&sender, &invalid, data.ok().flatten());
                        Ok(())
                    }));
                let name = HSTRING::from(method);
                let receiver =
                    unsafe { core.GetDevToolsProtocolEventReceiver(PCWSTR(name.as_ptr()))? };
                let mut token = 0;
                unsafe {
                    receiver.add_DevToolsProtocolEventReceived(&handler, &mut token)?;
                }
                registration.receivers.push((receiver, token));
            }
            EVENTS.with(|events| {
                events.borrow_mut().insert(id, registration);
            });
            Ok(())
        })();
        let _ = tx.send(result.map_err(|_| "Browser frame events are unavailable.".to_owned()));
    })
    .map_err(|_| "Browser view is unavailable.".to_owned())?;
    // Keep the subscription owner even when the awaiting caller is cancelled.
    // Its Drop queues removal after the registration on the same UI thread.
    let events = ProtocolEvents {
        id,
        view: view.clone(),
        receiver: Some(receiver),
        sender: control_sender,
        invalid,
    };
    rx.await
        .map_err(|_| "Browser frame subscription was lost.".to_owned())??;
    Ok(events)
}

/// Explicit COM close acknowledges native controller destruction before Tauri
/// removes the WebView registration. Queuing Webview::close alone is not proof.
pub(crate) async fn close_native_view(view: &tauri::Webview) -> Result<(), String> {
    let label=view.label().to_owned();
    let (tx,rx)=oneshot::channel();
    view.app_handle().run_on_main_thread(move || {let _=tx.send(native_closed(&label));}).map_err(|e|e.to_string())?;
    let already_closed=rx.await.map_err(|_|"Native close proof callback lost.".to_owned())?;
    if !already_closed {
    popup::settle_view(view).await?;
    user_downloads::cancel_and_wait(view, true).await?;
    user_file_chooser::cancel_and_wait(view).await?;
    let (tx, rx) = oneshot::channel();
    let label = view.label().to_owned();
    view.with_webview(move |platform| {
        // A failed Close is not proof that the page or its commands ended.
        // Keep permission/popup guards and protocol ownership until native ack.
        let _=permissions::set_visible(&label,false);
        // SAFETY: COM controller is accessed only from its owning UI thread.
        let result = unsafe { platform.controller().Close() }
            .map_err(|_| "Native browser controller did not close.".to_owned());
        if result.is_ok() {
            finalize_closed_view(&label);
        }
        let _ = tx.send(result);
    })
    .map_err(|_| "Native browser controller is unavailable.".to_owned())?;
    rx.await
        .map_err(|_| "Native close callback was lost.".to_owned())??;
    }
    // Native close can settle/reject a last reentrant new-window request.
    // Do not release the opener registration/profile while its child remains.
    popup::settle_view(view).await?;
    let owned=view.clone();
    let (tx,rx)=oneshot::channel();
    view.app_handle().run_on_main_thread(move || {
        let result=if owned.app_handle().get_webview(owned.label()).is_some() {
            owned.close().map_err(|_| "Browser view registration did not close.".to_owned())
        } else {Ok(())};
        let _=tx.send(result);
    }).map_err(|e|e.to_string())?;
    rx.await.map_err(|_|"Browser unregistration callback was lost.".to_owned())??;
    // A separate UI turn is a registration barrier, not a timed delay.
    let owned=view.clone();let (tx,rx)=oneshot::channel();
    view.app_handle().run_on_main_thread(move || {
        let _=tx.send(if forget_closed_if_unregistered(&owned){Ok(())}else{Err("Browser view registration remains owned.".to_owned())});
    }).map_err(|e|e.to_string())?;
    rx.await.map_err(|_|"Browser close barrier was lost.".to_owned())?
}

/// UI-thread cleanup after a confirmed native Close, including rejected popup
/// candidates whose handlers were installed before their first document.
pub(super) fn finalize_closed_view(label: &str) {
    CLOSED_VIEWS.with(|views|views.borrow_mut().insert(label.into()));
    popup::acknowledge_child_close(label);
    fail_pending_commands(label);
    popup::close_view(label);
    navigation_metadata::close_view(label);
    navigation::close_view(label);
    navigation_request_gate::close_view(label);
    presentation::close_view(label);
    permissions::close_view(label);
    script_dialogs::close_view(label);
    process_failure::close_view(label);
    diagnostics::close_view(label);
    file_chooser::close_view(label);
    user_file_chooser::close_view(label);
    user_downloads::close_view(label);
    shortcuts::close_view(label);
    EVENTS.with(|events| events.borrow_mut().retain(|_, registration|registration.view_label!=label));
}

/// Wait for the actual WebView2 completion callback, not merely UI dispatch.
/// The operation owner must retain this future until callback settlement.
pub(crate) async fn protocol_call(
    view: &tauri::Webview,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    protocol_call_session(view, None, method, params).await
}

/// Private transport; child routes are admitted by FrameSessions, not callers
/// supplying arbitrary target/session IDs from the model or application IPC.
async fn protocol_call_session(
    view: &tauri::Webview,
    session: Option<&str>,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    protocol_call_inner(view,session,method,params,None,None,None,None).await
}

pub(crate) struct NavigationDispatch {
    pub source: nomifun_browser_platform::navigation::BrowserNavigationSource,
    pub url: Option<String>,
    pub action: nomifun_browser_platform::navigation::BrowserNavigationActionType,
    pub receipt: Arc<std::sync::Mutex<Option<navigation::Receipt>>>,
    pub rejected_before_unload: Arc<AtomicBool>,
    pub cancel: tokio_util::sync::CancellationToken,
    pub closing: tokio_util::sync::CancellationToken,
}

pub(crate) async fn protocol_navigation(view:&tauri::Webview,method:&str,params:Value,dispatch:NavigationDispatch,expected:Option<navigation::Receipt>)->Result<Value,String>{
    protocol_call_inner(view,None,method,params,None,None,Some(dispatch),expected).await
}
pub(crate) async fn stop_navigation(view:&tauri::Webview,expected:navigation::Receipt)->Result<(),String>{
    navigation::stop_current(view,expected).await
}

pub(crate) struct UserFileCommandGuard {
    pub cancel: tokio_util::sync::CancellationToken,
    pub locked: Arc<AtomicBool>,
    pub visible: Arc<AtomicBool>,
    pub closed: Arc<AtomicBool>,
}
impl UserFileCommandGuard {
    fn permitted(&self) -> bool {
        !self.cancel.is_cancelled() && !self.locked.load(Ordering::Acquire)
            && self.visible.load(Ordering::Acquire) && !self.closed.load(Ordering::Acquire)
    }
}
async fn set_user_files(view:&tauri::Webview,session:Option<&str>,params:Value,guard:UserFileCommandGuard)->Result<Value,String> {
    protocol_call_inner(view,session,"DOM.setFileInputFiles",params,None,Some(guard),None,None).await
}

pub(crate) async fn reconcile_file_chooser_policy(view:&tauri::Webview,input_locked:Arc<AtomicBool>)->Result<(),String> {
    protocol_call_inner(view,None,"Page.setInterceptFileChooserDialog",serde_json::json!({}),Some(input_locked),None,None,None).await.map(|_|())
}

async fn protocol_call_inner(view:&tauri::Webview,session:Option<&str>,method:&str,params:Value,current_input_policy:Option<Arc<AtomicBool>>,user_guard:Option<UserFileCommandGuard>,navigation_dispatch:Option<NavigationDispatch>,navigation_guard:Option<navigation::Receipt>)->Result<Value,String> {
    if view.app_handle().get_webview(view.label()).is_none() {
        return Err("Browser view is no longer registered.".into());
    }
    if method.len() > 128 || params.to_string().len() > 1024 * 1024 {
        return Err("Browser protocol request exceeds its limit.".to_owned());
    }
    let recovery=matches!(method,"Page.navigate"|"Page.reload"|"Page.getNavigationHistory"|"Page.navigateToHistoryEntry");
    let chooser_policy=method=="Page.setInterceptFileChooserDialog";
    let navigate=method=="Page.navigate";
    let method = HSTRING::from(method);
    let session = session.map(HSTRING::from);
    let label = view.label().to_owned();
    let command_id = uuid::Uuid::now_v7();
    let app=view.app_handle().clone();
    let (tx, rx) = oneshot::channel();
    view.with_webview(move |platform| {
        if native_closed(&label) || app.get_webview(&label).is_none() {
            let _=tx.send(Err("Browser native controller has closed.".into()));
            return;
        }
        if user_guard.as_ref().is_some_and(|guard|!guard.permitted()) {
            let _=tx.send(Err("User file request is no longer current".into()));
            return;
        }
        if navigation_dispatch.as_ref().is_some_and(|dispatch|dispatch.cancel.is_cancelled()||dispatch.closing.is_cancelled()) {
            let _=tx.send(Err("Browser navigation was cancelled before dispatch.".into()));return;
        }
        if let Some(expected)=navigation_guard {
            if let Err(error)=navigation::check(&label,expected) {let _=tx.send(Err(error));return;}
        }
        // ProcessFailed is positive evidence that the old document cannot
        // answer renderer commands. Do not enqueue a new unfinishable callback.
        if process_failure::document_exited(&label) && !recovery {
            let _=tx.send(if chooser_policy {Ok(serde_json::json!({}))} else {Err("Browser document process has exited.".into())});
            return;
        }
        let mut params=current_input_policy.map_or(params,|policy|serde_json::json!({"enabled":policy.load(Ordering::Acquire)}));
        // Managed user choosers and Agent uploads share the same native page.
        // The broker, not the OS default dialog, always handles HTML requests.
        if chooser_policy && user_file_chooser::installed(&label) { params["enabled"]=Value::Bool(true); }
        let params=HSTRING::from(params.to_string());
        if COMMANDS.with(|commands| {
            commands
                .borrow()
                .values()
                .filter(|pending| pending.label == label)
                .count()
                >= 128
        }) {
            let _ = tx.send(Err("Browser has too many pending protocol commands.".into()));
            return;
        }
        let navigation_source=navigation_dispatch.as_ref().map(|dispatch|dispatch.source);
        let navigation_rejected=navigation_dispatch.as_ref().map(|dispatch|dispatch.rejected_before_unload.clone());
        let navigation_receipt=if let Some(dispatch)=navigation_dispatch {
            match navigation::prepare(&label,dispatch.source,dispatch.url,dispatch.action) {
                Ok(receipt)=>{*dispatch.receipt.lock().unwrap_or_else(|e|e.into_inner())=Some(receipt);Some(receipt)},
                Err(error)=>{let _=tx.send(Err(error));return;},
            }
        }else{None};
        // Completion and synchronous submission failure share one sender.
        let sender = std::rc::Rc::new(std::cell::RefCell::new(Some(tx)));
        COMMANDS.with(|commands| {
            commands.borrow_mut().insert(
                command_id,
                PendingCommand {
                    label:label.clone(),
                    sender: sender.clone(),
                    navigation: navigation_receipt.map(|receipt| PendingNavigation { receipt, source:navigation_source.unwrap(), navigate, rejected:navigation_rejected.unwrap() }),
                    before_unload: BeforeUnloadDecision::None,
                    deferred_response: None,
                },
            )
        });
        let callback = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
            move |status, response| {
                let result = status
                    .map_err(|_| "Browser protocol command failed.".to_owned())
                    .and_then(|()| {
                        if response.len() > 4 * 1024 * 1024 {
                            return Err("Browser protocol response exceeds its limit.".to_owned());
                        }
                        serde_json::from_str::<Value>(&response)
                            .map_err(|_| "Browser protocol response was invalid.".to_owned())
                    });
                finish_protocol_command(command_id, result);
                Ok(())
            },
        ));
        // SAFETY: with_webview runs on the WebView2 UI thread. Strings and the
        // callback remain alive through submission; COM retains the callback.
        let submitted = unsafe {
            platform.controller().CoreWebView2().and_then(|core| {
                if let Some(session) = &session {
                    core.cast::<ICoreWebView2_11>()?
                        .CallDevToolsProtocolMethodForSession(
                            PCWSTR(session.as_ptr()),
                            PCWSTR(method.as_ptr()),
                            PCWSTR(params.as_ptr()),
                            &callback,
                        )
                } else {
                    core.CallDevToolsProtocolMethod(
                        PCWSTR(method.as_ptr()),
                        PCWSTR(params.as_ptr()),
                        &callback,
                    )
                }
            })
        };
        if submitted.is_err() {
            finish_protocol_command(command_id, Err("Browser protocol command could not be submitted.".to_owned()));
        }
    })
    .map_err(|_| "Browser view is unavailable.".to_owned())?;
    rx.await
        .map_err(|_| "Browser closed before protocol completion.".to_owned())?
}

/// Wry creates a dedicated WRY_WEBVIEW child HWND for each WebView2 controller.
/// Disable only that container. CDP still reaches the browser input pipeline.
pub(crate) async fn set_user_input_enabled(
    view: &tauri::Webview,
    enabled: bool,
) -> Result<(), String> {
    if enabled {
        protocol_call(view,"Page.setInterceptFileChooserDialog",serde_json::json!({"enabled":false})).await?;
    }
    let input = set_native_user_input_enabled(view, enabled).await;
    if !enabled {
        // A script dialog also pauses renderer protocol commands. Disable the
        // native HWND first, then dismiss dialogs before configuring CDP input
        // policy; waiting for that policy before dismissal deadlocks run start.
        script_dialogs::drain(view).await.map_err(|error|error.to_string())?;
        user_file_chooser::cancel_and_wait(view).await?;
        user_downloads::cancel_and_wait(view, false).await?;
        let chooser=protocol_call(view,"Page.setInterceptFileChooserDialog",serde_json::json!({"enabled":true})).await.map(|_|());
        input.and(chooser)
    } else {input}
}

/// Native input gate only, for an already-configured popup immediately after
/// binding. No renderer protocol call can block this behind a site dialog.
pub(crate) async fn set_native_user_input_enabled(view: &tauri::Webview, enabled: bool) -> Result<(), String> {
    let label=view.label().to_owned();
    let parent = view
        .window()
        .hwnd()
        .map_err(|_| "Browser window is unavailable.".to_owned())?
        .0 as usize;
    let (tx, rx) = oneshot::channel();
    view.with_webview(move |platform| {
        let permissions=if !enabled { permissions::deny_pending(&label) } else {Ok(())};
        // SAFETY: window ownership is checked before changing native input.
        let result = unsafe {
            let mut container = HWND::default();
            platform
                .controller()
                .ParentWindow(&mut container)
                .map_err(|_| "Browser container is unavailable.".to_owned())
                .and_then(|()| {
                    if container.0.is_null()
                        || container.0 as usize == parent
                        || GetParent(container).map(|hwnd| hwnd.0 as usize).ok() != Some(parent)
                    {
                        return Err("Browser input container ownership is invalid.".to_owned());
                    }
                    // The default browser menu is a native popup outside page DOM.
                    // Keep it out of Agent input while retaining page contextmenu events.
                    let settings = platform
                        .controller()
                        .CoreWebView2()
                        .and_then(|core| core.Settings())
                        .map_err(|_| "Browser settings are unavailable.".to_owned())?;
                    settings
                        .SetAreDefaultContextMenusEnabled(enabled)
                        .map_err(|_| "Browser context menu policy was not applied.".to_owned())?;
                    let _ = EnableWindow(container, enabled);
                    if IsWindowEnabled(container).as_bool() != enabled {
                        return Err("Browser input policy was not applied.".to_owned());
                    }
                    Ok(())
                })
        };
        let _ = tx.send(result.and(permissions));
    })
    .map_err(|_| "Browser view is unavailable.".to_owned())?;
    rx.await.map_err(|_| "Browser closed before input policy completed.".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confirmed_native_close_fences_new_commands_until_registration_is_gone() {
        let label="native-close-fence-fixture";
        assert!(!native_closed(label));
        finalize_closed_view(label);
        assert!(native_closed(label));
        // Only confirmed Tauri unregistration clears this interval in live
        // code; the fixture owns no Tauri view, so remove its test marker.
        CLOSED_VIEWS.with(|views|views.borrow_mut().remove(label));
        assert!(!native_closed(label));
    }

    #[test]
    fn file_delivery_rechecks_live_native_dispatch_authority() {
        let guard=UserFileCommandGuard {cancel:Default::default(),locked:Arc::new(AtomicBool::new(false)),visible:Arc::new(AtomicBool::new(true)),closed:Arc::new(AtomicBool::new(false))};
        assert!(guard.permitted());
        guard.locked.store(true,Ordering::Release);
        assert!(!guard.permitted());
        guard.locked.store(false,Ordering::Release);
        guard.visible.store(false,Ordering::Release);
        assert!(!guard.permitted());
        guard.visible.store(true,Ordering::Release);
        guard.closed.store(true,Ordering::Release);
        assert!(!guard.permitted());
        guard.closed.store(false,Ordering::Release);
        guard.cancel.cancel();
        assert!(!guard.permitted());
    }

    #[test]
    fn process_failure_settles_only_its_view_and_ignores_late_completion() {
        let register = |label: &str| {
            let (tx, rx) = oneshot::channel();
            let sender = std::rc::Rc::new(RefCell::new(Some(tx)));
            COMMANDS.with(|commands| {
                commands.borrow_mut().insert(
                    uuid::Uuid::now_v7(),
                    PendingCommand {
                        label: label.into(),
                        sender: sender.clone(),
                        navigation: None,
                        before_unload: BeforeUnloadDecision::None,
                        deferred_response: None,
                    },
                )
            });
            (sender, rx)
        };
        let (failed_sender, mut failed) = register("failed-view");
        let (_, mut other) = register("other-view");
        fail_pending_commands("failed-view");
        assert!(failed.try_recv().unwrap().is_err());
        assert!(failed_sender.borrow_mut().take().is_none());
        fail_pending_commands("failed-view");
        assert_eq!(other.try_recv(), Err(oneshot::error::TryRecvError::Empty));
        fail_pending_commands("other-view");
        assert!(other.try_recv().unwrap().is_err());
        assert!(COMMANDS.with(|commands| commands.borrow().is_empty()));
    }

    #[test]
    fn before_unload_response_waits_for_decision_ack_and_failed_ack_keeps_error() {
        let id = uuid::Uuid::now_v7();
        let (tx, mut rx) = oneshot::channel();
        COMMANDS.with(|commands| commands.borrow_mut().insert(id, PendingCommand {
            label: "decision-view".into(), sender: std::rc::Rc::new(RefCell::new(Some(tx))),
            navigation: None, before_unload: BeforeUnloadDecision::Pending, deferred_response: None,
        }));
        finish_protocol_command(id, Ok(serde_json::json!({"errorText":"net::ERR_ABORTED"})));
        assert_eq!(rx.try_recv(), Err(oneshot::error::TryRecvError::Empty));
        assert!(COMMANDS.with(|commands| commands.borrow().get(&id).unwrap().deferred_response.is_some()));
        finish_before_unload_decision(id, false);
        assert_eq!(rx.try_recv().unwrap().unwrap()["errorText"], "net::ERR_ABORTED");
        assert!(!COMMANDS.with(|commands| commands.borrow().contains_key(&id)));
    }

    #[test]
    fn process_exit_settles_a_deferred_before_unload_response_once() {
        let id = uuid::Uuid::now_v7();
        let (tx, mut rx) = oneshot::channel();
        COMMANDS.with(|commands| commands.borrow_mut().insert(id, PendingCommand {
            label: "exited-decision-view".into(), sender: std::rc::Rc::new(RefCell::new(Some(tx))),
            navigation: None, before_unload: BeforeUnloadDecision::Pending, deferred_response: None,
        }));
        finish_protocol_command(id, Ok(serde_json::json!({"errorText":"net::ERR_ABORTED"})));
        fail_pending_commands("exited-decision-view");
        assert!(rx.try_recv().unwrap().is_err());
        finish_before_unload_decision(id, true);
        finish_protocol_command(id, Ok(serde_json::json!({})));
        assert_eq!(rx.try_recv(), Err(oneshot::error::TryRecvError::Closed));
        assert!(!COMMANDS.with(|commands| commands.borrow().contains_key(&id)));
    }

    #[test]
    fn stay_proof_never_suppresses_dns_tls_other_or_unproved_abort() {
        let abort = Ok(serde_json::json!({"errorText":"net::ERR_ABORTED"}));
        assert!(known_before_unload_abort(&BeforeUnloadDecision::Rejected, &abort));
        assert!(!known_before_unload_abort(&BeforeUnloadDecision::None, &abort));
        assert!(!known_before_unload_abort(&BeforeUnloadDecision::Pending, &abort));
        for error in ["net::ERR_NAME_NOT_RESOLVED", "net::ERR_CERT_AUTHORITY_INVALID", "net::ERR_FAILED", ""] {
            assert!(!known_before_unload_abort(&BeforeUnloadDecision::Rejected, &Ok(serde_json::json!({"errorText":error}))));
        }
        assert!(!known_before_unload_abort(&BeforeUnloadDecision::Rejected, &Err("Protocol failed".into())));
        assert!(!known_before_unload_abort(&BeforeUnloadDecision::Rejected, &Ok(serde_json::json!({"errorText":"net::ERR_ABORTED","isDownload":true}))));
    }

    #[test]
    fn positive_download_result_precedes_aborted_document_navigation() {
        let download = serde_json::json!({"errorText":"net::ERR_ABORTED","isDownload":true});
        assert!(!navigation_response_failed(&download));
        assert!(!known_before_unload_abort(&BeforeUnloadDecision::Rejected, &Ok(download)));
        assert!(navigation_response_failed(&serde_json::json!({"errorText":"net::ERR_ABORTED","isDownload":false})));
        assert!(navigation_response_failed(&serde_json::json!({"errorText":"net::ERR_NAME_NOT_RESOLVED"})));
        assert!(!navigation_response_failed(&serde_json::json!({"loaderId":"owned-loader"})));
    }

    #[test]
    fn full_or_closed_event_receiver_invalidates_routes_without_blocking_ui() {
        let (sender, receiver) = mpsc::channel(64);
        let invalid = AtomicBool::new(false);
        let event = || {
            Some(ProtocolEvent {
                method: "Target.detachedFromTarget",
                parent_session: String::new(),
                parameters: r#"{"sessionId":"frame"}"#.into(),
            })
        };
        for _ in 0..64 {
            enqueue_event(&sender, &invalid, event());
        }
        assert!(!invalid.load(Ordering::Acquire));
        enqueue_event(&sender, &invalid, event());
        assert!(invalid.load(Ordering::Acquire));
        drop(receiver);
        let invalid = AtomicBool::new(false);
        enqueue_event(&sender, &invalid, event());
        assert!(invalid.load(Ordering::Acquire));
    }

    #[test]
    fn event_string_checks_utf16_bound_before_copying() {
        let mut text = CoTaskMemPWSTR::from("网页😀");
        assert_eq!(event_string(text.take(), 4).as_deref(), Some("网页😀"));
        let mut text = CoTaskMemPWSTR::from("网页😀");
        assert!(event_string(text.take(), 3).is_none());
        assert_eq!(event_string(PWSTR::null(), 256).as_deref(), Some(""));
    }
}

/// Used by shared task settlement before native tab destruction.
pub(crate) async fn hide(view: &View) -> Result<(), String> { view.hide().map_err(|error| error.to_string()) }
