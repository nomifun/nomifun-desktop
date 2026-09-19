//! CEF's page-scoped protocol implements the shared semantic driver's ports.
use std::sync::{Arc, Mutex as StdMutex, atomic::{AtomicBool, Ordering}};
use nomifun_browser_macos::{engine::Page, protocol::CallbackSubscription};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

#[derive(Clone)]
pub(crate) struct View {
    pub page: Arc<Page>,
    pub diagnostics_failed: Arc<AtomicBool>,
    diagnostics_owner: Arc<StdMutex<Option<CallbackSubscription>>>,
}
impl View {
    pub fn new(page: Arc<Page>) -> Self { Self { page, diagnostics_failed: Arc::new(AtomicBool::new(false)), diagnostics_owner: Arc::new(StdMutex::new(None)) } }
}

#[path = "../frame_sessions.rs"]
pub(crate) mod frames;
#[path = "native/file_chooser.rs"]
pub(crate) mod file_chooser;
#[path = "native/external_browser.rs"]
pub(crate) mod external_browser;

pub(crate) async fn protocol_call(view: &View, method: &str, params: Value) -> Result<Value, String> {
    view.page.protocol.call(None, method, params).await
}
pub(crate) async fn protocol_call_session(view: &View, session: Option<&str>, method: &str, params: Value) -> Result<Value, String> {
    view.page.protocol.call(session, method, params).await
}

pub(crate) async fn hide(view: &View) -> Result<(), String> { view.page.hide().await }
pub(crate) async fn set_native_user_input_enabled(view: &View, enabled: bool) -> Result<(), String> { view.page.set_input_locked(!enabled).await }
pub(crate) mod site_data {
    use super::*;
    pub async fn clear(view: &View, cancel: tokio_util::sync::CancellationToken) -> Result<(), nomifun_browser_platform::runtime::WorkspaceError> {
        view.page.clear_site_data(cancel).await.map_err(|_| nomifun_browser_platform::runtime::WorkspaceError::NativeCommandFailed)
    }
}
pub(crate) mod script_dialogs {
    use super::*;
    use nomifun_browser_platform::runtime::{BrowserTabTarget, WorkspaceError};
    pub async fn drain(view: &View) -> Result<(), WorkspaceError> { view.page.set_dialog_draining(true).await.map_err(|_| WorkspaceError::NativeCommandFailed) }
    pub async fn resume(view: &View) -> Result<(), WorkspaceError> { view.page.set_dialog_draining(false).await.map_err(|_| WorkspaceError::NativeCommandFailed) }
    pub async fn respond(view: &View, target: BrowserTabTarget, id: String, accept: bool, text: Option<String>) -> Result<(), WorkspaceError> {
        if target.tab_id != format!("browser-{}", view.page.id()) { return Err(WorkspaceError::StaleTarget); }
        view.page.reply_dialog(id, target.document_generation, accept, text.unwrap_or_default(), Default::default()).await.map_err(|_| WorkspaceError::StaleTarget)
    }
}

pub(crate) mod permissions {
    use super::*;
    use nomifun_browser_platform::runtime::{BrowserTabTarget, WorkspaceError};

    pub async fn respond(
        view: &View,
        target: BrowserTabTarget,
        request_id: String,
        allow: bool,
    ) -> Result<(), WorkspaceError> {
        if target.tab_id != format!("browser-{}", view.page.id()) {
            return Err(WorkspaceError::StaleTarget);
        }
        view.page
            .reply_permission(request_id, target.document_generation, allow)
            .await
            .map_err(|_| WorkspaceError::StaleTarget)
    }
}

pub(crate) struct ProtocolEvent {
    pub method: &'static str,
    pub parent_session: String,
    pub parameters: String,
}
pub(crate) enum ProtocolMessage { Event(ProtocolEvent), Barrier(oneshot::Sender<()>) }
struct ProtocolEvents {
    id: uuid::Uuid,
    view: View,
    receiver: Option<mpsc::Receiver<ProtocolMessage>>,
    sender: mpsc::Sender<ProtocolMessage>,
    invalid: Arc<AtomicBool>,
    _subscription: CallbackSubscription,
}
async fn listen_frames(view: &View) -> Result<ProtocolEvents, String> {
    let (sender, receiver) = mpsc::channel(64);
    let output = sender.clone();
    let subscription = view.page.protocol.subscribe_callback(&["Target.attachedToTarget", "Target.detachedFromTarget"], Arc::new(move |event| {
        let method = match event.method.as_str() { "Target.attachedToTarget" => "Target.attachedToTarget", "Target.detachedFromTarget" => "Target.detachedFromTarget", _ => return false };
        output.try_send(ProtocolMessage::Event(ProtocolEvent { method, parent_session: event.session.unwrap_or_default(), parameters: event.params.to_string() })).is_ok()
    }))?;
    Ok(ProtocolEvents { id: uuid::Uuid::now_v7(), view: view.clone(), receiver: Some(receiver), sender, invalid: subscription.failed.clone(), _subscription: subscription })
}

pub(crate) struct UserFileCommandGuard {
    pub cancel: tokio_util::sync::CancellationToken,
    pub locked: Arc<AtomicBool>,
    pub visible: Arc<AtomicBool>,
    pub closed: Arc<AtomicBool>,
}
async fn set_user_files(view: &View, session: Option<&str>, params: Value, guard: UserFileCommandGuard) -> Result<Value, String> {
    let page = view.page.clone();
    let allowed = Arc::new(move || !page.input_locked() && !guard.cancel.is_cancelled() && !guard.locked.load(Ordering::Acquire) && guard.visible.load(Ordering::Acquire) && !guard.closed.load(Ordering::Acquire));
    view.page.protocol.call_guarded(session, "DOM.setFileInputFiles", params, Some(allowed)).await
}

pub(crate) mod diagnostics {
    use super::*;
    use nomifun_browser_platform::{
        revision::BrowserRevision,
        runtime::{BrowserDiagnostic, BrowserTabSnapshot},
        url_projection::project_metadata_url,
    };
    use std::sync::Mutex;

    const MAX_ENTRIES: usize = 32;
    const MAX_TEXT: usize = 512;

    fn clipped(value: &str) -> String {
        value.chars().take(MAX_TEXT).collect()
    }

    fn source_url(value: &str) -> String {
        clipped(&project_metadata_url(value))
    }

    pub async fn install(
        view: &View,
        metadata: Arc<Mutex<BrowserTabSnapshot>>,
        revision: Arc<BrowserRevision>,
    ) -> Result<(), String> {
        let failed = view.diagnostics_failed.clone();
        let subscription = view.page.protocol.subscribe_callback(
            &[
                "Runtime.consoleAPICalled",
                "Runtime.exceptionThrown",
                "Network.loadingFailed",
                "Network.responseReceived",
            ],
            Arc::new(move |event| {
                let projection = (|| {
                    let (kind, level, message, url) = match event.method.as_str() {
                        "Runtime.consoleAPICalled" => {
                            let level = match event.params["type"].as_str().unwrap_or("") {
                                "error" | "assert" => "error",
                                "warning" => "warn",
                                "debug" => "debug",
                                _ => "info",
                            };
                            let message = event.params["args"].as_array().map(|args| {
                                args.iter().take(8).map(|argument| {
                                    argument.get("value")
                                        .filter(|value| !value.is_object() && !value.is_array())
                                        .map(|value| value.as_str().map(str::to_owned).unwrap_or_else(||value.to_string()))
                                        .or_else(|| argument["unserializableValue"].as_str().map(str::to_owned))
                                        .or_else(|| argument["description"].as_str().map(str::to_owned))
                                        .unwrap_or_else(||"[object]".into())
                                }).collect::<Vec<_>>().join(" ")
                            }).unwrap_or_default();
                            let url = event.params["stackTrace"]["callFrames"][0]["url"].as_str().unwrap_or("");
                            ("console", level, message, url)
                        }
                        "Runtime.exceptionThrown" => {
                            let detail = &event.params["exceptionDetails"];
                            ("page_error", "error",
                                detail["exception"]["description"].as_str().or_else(||detail["text"].as_str()).unwrap_or("Page exception").to_owned(),
                                detail["url"].as_str().unwrap_or(""))
                        }
                        "Network.loadingFailed" => (
                            "network", "error",
                            event.params["errorText"].as_str().unwrap_or("Network request failed").to_owned(),
                            "",
                        ),
                        "Network.responseReceived" => {
                            let response = &event.params["response"];
                            let status = response["status"].as_u64().unwrap_or(0);
                            if status < 400 { return Ok::<_, ()>(None); }
                            ("network", "error", format!("HTTP {status}"), response["url"].as_str().unwrap_or(""))
                        }
                        _ => return Ok(None),
                    };
                    Ok(Some(BrowserDiagnostic {
                        id: 0,
                        kind: kind.into(),
                        level: level.into(),
                        message: clipped(&message),
                        source_url: source_url(url),
                    }))
                })();
                let Ok(Some(mut diagnostic)) = projection else {
                    if projection.is_err() { failed.store(true, Ordering::Release); }
                    return projection.is_ok();
                };
                let mut tab = metadata.lock().unwrap_or_else(|error|error.into_inner());
                diagnostic.id = tab.diagnostics.entries.last().map_or(1, |entry| entry.id.saturating_add(1));
                if tab.diagnostics.entries.len() >= MAX_ENTRIES {
                    tab.diagnostics.entries.remove(0);
                    tab.diagnostics.dropped = tab.diagnostics.dropped.saturating_add(1);
                }
                tab.diagnostics.entries.push(diagnostic);
                tab.diagnostics.unavailable = false;
                drop(tab);
                revision.bump();
                true
            }),
        )?;
        *view.diagnostics_owner.lock().unwrap() = Some(subscription);
        enable_session(view, None).await?;
        Ok(())
    }

    pub async fn enable_session(view: &View, session: Option<&str>) -> Result<(), String> {
        for method in ["Runtime.enable", "Network.enable"] {
            protocol_call_session(view, session, method, serde_json::json!({})).await?;
        }
        Ok(())
    }
    pub fn mark_unavailable(view: &View) { view.diagnostics_failed.store(true, Ordering::Release); }
}

pub(crate) async fn set_user_input_enabled(view: &View, enabled: bool) -> Result<(), String> {
    if enabled {
        // macOS owns both Agent and human chooser flows. Keep interception on
        // in UserReady so a website can never open an unmanaged CEF dialog.
        protocol_call(view, "Page.setInterceptFileChooserDialog", serde_json::json!({"enabled":true})).await?;
        return set_native_user_input_enabled(view, true).await;
    }
    let input = set_native_user_input_enabled(view, false).await;
    // Renderer commands can wait on a user dialog. Freeze native input first,
    // then drain that dialog before awaiting root chooser interception.
    script_dialogs::drain(view).await.map_err(|error| error.to_string())?;
    let chooser = protocol_call(view, "Page.setInterceptFileChooserDialog", serde_json::json!({"enabled":true})).await.map(|_|());
    input.and(chooser)
}
